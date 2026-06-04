//! Cross-class dispatch (T04). Within a class the order is strict FIFO; *which*
//! class dispatches next when several are runnable is decided here.
//!
//! ## Model
//!
//! Each class declares a [`FairnessPolicy`]. When several classes are runnable
//! under the global budget, the governor arbitrates using the discipline of the
//! lexicographically-first runnable class in the contended tier, with arrival
//! order as the stable tie-break inside every discipline. Best-effort classes
//! form a strictly lower tier: they dispatch only when no non-best-effort class
//! is runnable (the scavenger guarantee — best effort never starves interactive
//! work). This keeps a heterogeneous mix deterministic without inventing a
//! blended heuristic.

use taskmesh_contract::{ClassPolicy, FairnessPolicy, ResourceBudget, TaskClass};

use crate::engine::state::GovernedState;
use crate::shared::PolicySet;

/// Fixed-point scale for weighted-fair virtual finish tags.
const WFQ_SCALE: u128 = 1_000_000;

/// Assign the scheduling tags a request carries while queued, and update the
/// class's WFQ cursor. Finish tags are computed against the class's last tag and
/// the global virtual time; deadlines derive from the class's slack.
pub fn enqueue_tags(
    state: &mut GovernedState,
    class: &TaskClass,
    policy: &ClassPolicy,
    cost_cpu_units: u32,
    enqueued_at_ms: u64,
) -> (u128, u64) {
    let virtual_time = state.virtual_time;
    let cstate = state.class_mut(class);
    let finish_tag = match policy.fairness {
        FairnessPolicy::WeightedFairQueue { weight, .. } => {
            let weight = u128::from(weight.max(1));
            let increment = (u128::from(cost_cpu_units.max(1)) * WFQ_SCALE) / weight;
            let tag = cstate.last_finish_tag.max(virtual_time) + increment;
            cstate.last_finish_tag = tag;
            tag
        }
        _ => 0,
    };
    let deadline_ms = match policy.fairness {
        FairnessPolicy::DeadlineAware { slack_ms } => enqueued_at_ms.saturating_add(slack_ms),
        _ => u64::MAX,
    };
    (finish_tag, deadline_ms)
}

/// A runnable class and the head request driving its dispatch key.
struct Candidate {
    class: TaskClass,
    fairness: FairnessPolicy,
    best_effort: bool,
    head_seq: u64,
    head_finish_tag: u128,
    head_deadline_ms: u64,
    head_cost_cpu: u32,
}

/// Pick the next class to promote, or `None` if nothing is runnable under the
/// current budget. Mutates DRR deficits / WFQ virtual time as a side effect of
/// the dispatch decision.
pub fn select(state: &mut GovernedState, policies: &PolicySet) -> Option<TaskClass> {
    let candidates = runnable_candidates(state, policies, &policies.resources);
    if candidates.is_empty() {
        return None;
    }

    // Tier split: best-effort only when nothing else is runnable.
    let has_primary = candidates.iter().any(|c| !c.best_effort);
    let pool: Vec<&Candidate> = candidates
        .iter()
        .filter(|c| c.best_effort != has_primary)
        .collect();

    let discipline = pool.first()?.fairness;
    let chosen_class = match discipline {
        FairnessPolicy::Fifo | FairnessPolicy::BestEffortScavenger => {
            pick_min(&pool, |c| (u128::from(c.head_seq), 0u128))
        }
        FairnessPolicy::WeightedFairQueue { .. } => {
            pick_min(&pool, |c| (c.head_finish_tag, u128::from(c.head_seq)))
        }
        FairnessPolicy::DeadlineAware { .. } => pick_min(&pool, |c| {
            (u128::from(c.head_deadline_ms), u128::from(c.head_seq))
        }),
        FairnessPolicy::DeficitRoundRobin { .. } => return drr_select(state, &pool),
    };

    // WFQ: advance global virtual time past the dispatched request's tag.
    if let FairnessPolicy::WeightedFairQueue { .. } = discipline {
        if let Some(c) = pool.iter().find(|c| c.class == chosen_class) {
            state.virtual_time = state.virtual_time.max(c.head_finish_tag);
        }
    }
    Some(chosen_class)
}

fn runnable_candidates(
    state: &GovernedState,
    policies: &PolicySet,
    budget: &ResourceBudget,
) -> Vec<Candidate> {
    let mut out = Vec::new();
    for (class, cstate) in &state.classes {
        let Some(head) = cstate.queue.front() else {
            continue;
        };
        let Some(policy) = policies.class(class) else {
            continue;
        };
        if state.capacity_for(class, policy.max_inflight, head.cost, budget)
            != crate::engine::state::CapacityBlock::Ok
        {
            continue;
        }
        out.push(Candidate {
            class: class.clone(),
            fairness: policy.fairness,
            best_effort: policy.best_effort,
            head_seq: head.seq_no,
            head_finish_tag: head.finish_tag,
            head_deadline_ms: head.deadline_ms,
            head_cost_cpu: head.cost.cpu_units,
        });
    }
    out
}

fn pick_min<K: Ord>(pool: &[&Candidate], key: impl Fn(&Candidate) -> K) -> TaskClass {
    pool.iter()
        .min_by(|a, b| key(a).cmp(&key(b)))
        .map(|c| c.class.clone())
        .expect("pool is non-empty")
}

/// Deficit round-robin (T04): a deterministic *active-class ring*, not
/// priority-by-quantum.
///
/// Each class carries a persistent deficit counter. When the ring cursor lands
/// on a class it is credited one quantum; it is then served (head cost debited)
/// while its deficit still covers its head cost. When the deficit is exhausted
/// — or the class drains — the cursor advances to the next runnable class in
/// class order, wrapping around. A quantum-`N` class is therefore served about
/// `N` times per lap before yielding, giving service *proportional* to quanta
/// with a bounded difference. Equal quanta degenerate to plain round-robin.
///
/// One promotion is returned per call; the cursor and deficits live in
/// `GovernedState` and persist across promotions. `pool` is class-sorted (it is
/// built by iterating the class `BTreeMap`), so "next class after the cursor,
/// wrapping" is the next ring slot.
fn drr_select(state: &mut GovernedState, pool: &[&Candidate]) -> Option<TaskClass> {
    if pool.is_empty() {
        return None;
    }

    // Keep serving the current cursor class while its deficit covers its head
    // cost — this is what lets a quantum-N class run ~N in a row.
    if let Some(cur) = state.drr_cursor.clone() {
        if let Some(c) = pool.iter().find(|c| c.class == cur) {
            let cost = u64::from(c.head_cost_cpu.max(1));
            if state.classes.get(&cur).map_or(0, |s| s.deficit) >= cost {
                state.class_mut(&cur).deficit -= cost;
                return Some(cur);
            }
        }
    }

    // Otherwise advance the ring: starting just after the cursor (wrapping),
    // credit each class on arrival and serve the first that can afford its head.
    // Bounded: every visit adds >= 1 to a finite deficit and the pool is
    // non-empty, so some runnable class is served.
    let start = match &state.drr_cursor {
        Some(cur) => pool.iter().position(|c| c.class > *cur).unwrap_or(0),
        None => 0,
    };
    let n = pool.len();
    let mut i = start;
    loop {
        let c = pool.get(i)?;
        let quantum = match c.fairness {
            FairnessPolicy::DeficitRoundRobin { quantum } => u64::from(quantum.max(1)),
            _ => 1,
        };
        let cost = u64::from(c.head_cost_cpu.max(1));
        let cs = state.class_mut(&c.class);
        cs.deficit = cs.deficit.saturating_add(quantum);
        if cs.deficit >= cost {
            cs.deficit -= cost;
            state.drr_cursor = Some(c.class.clone());
            return Some(c.class.clone());
        }
        i = (i + 1) % n;
    }
}
