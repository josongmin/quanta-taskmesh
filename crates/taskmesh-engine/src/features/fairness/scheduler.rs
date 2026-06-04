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
            let weight = weight.max(1) as u128;
            let increment = (cost_cpu_units.max(1) as u128 * WFQ_SCALE) / weight;
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

    let discipline = pool[0].fairness;
    let chosen_class = match discipline {
        FairnessPolicy::Fifo | FairnessPolicy::BestEffortScavenger => {
            pick_min(&pool, |c| (c.head_seq as u128, 0u128))
        }
        FairnessPolicy::WeightedFairQueue { .. } => {
            pick_min(&pool, |c| (c.head_finish_tag, c.head_seq as u128))
        }
        FairnessPolicy::DeadlineAware { .. } => {
            pick_min(&pool, |c| (c.head_deadline_ms as u128, c.head_seq as u128))
        }
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

/// Deficit round-robin: credit each runnable class one quantum, then serve the
/// class with the largest deficit (arrival/class order tie-break), debiting the
/// head cost. Accumulation guarantees deterministic rotation across classes.
fn drr_select(state: &mut GovernedState, pool: &[&Candidate]) -> Option<TaskClass> {
    for c in pool {
        if let FairnessPolicy::DeficitRoundRobin { quantum } = c.fairness {
            let cs = state.class_mut(&c.class);
            cs.deficit = cs.deficit.saturating_add(quantum.max(1) as u64);
        }
    }
    let chosen = pool
        .iter()
        .max_by(|a, b| {
            let da = state.classes.get(&a.class).map(|s| s.deficit).unwrap_or(0);
            let db = state.classes.get(&b.class).map(|s| s.deficit).unwrap_or(0);
            da.cmp(&db).then(b.class.cmp(&a.class)) // larger deficit, then earlier class
        })
        .map(|c| (c.class.clone(), c.head_cost_cpu))?;
    let cs = state.class_mut(&chosen.0);
    cs.deficit = cs.deficit.saturating_sub(chosen.1.max(1) as u64);
    Some(chosen.0)
}
