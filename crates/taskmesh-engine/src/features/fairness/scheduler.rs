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
//!
//! ## Runnable means *fully* runnable
//!
//! A candidate must satisfy semantic capacity **and** the physical capability
//! pool its head request resolved to. Choosing a class that then has to wait for
//! a worker is what creates a second scheduling queue behind the first, where
//! arrival order silently overrides class fairness.
//!
//! ## Bounds
//!
//! - **DRR** skips empty rounds arithmetically. A head whose cost is `K` quanta
//!   away is served after one `O(classes)` computation, not `K` ring visits, so
//!   a `quantum = 1`, `cost = u32::MAX` class cannot hold the state mutex for
//!   four billion iterations.
//! - **WFQ** tags are `u128` fixed point at `2^64`, so every accepted weight up
//!   to `u32::MAX` still produces a non-zero increment. A scale that rounds the
//!   increment to zero is how weighted share silently degrades to FIFO.
//! - **Idle credit does not accumulate.** When a class's queue *drains*, its
//!   deficit and its virtual-finish baseline reset. Otherwise a class that idles
//!   through twenty busy periods returns holding twenty quanta of credit and
//!   runs its whole backlog ahead of a peer that was queued the entire time.

use taskmesh_contract::{ClassPolicy, FairnessPolicy, ResourceBudget, TaskClass};

use crate::engine::state::{CapacityBlock, GovernedState};
use crate::shared::PolicySet;

/// Fixed-point scale for weighted-fair virtual finish tags.
///
/// `2^64` keeps `cost * SCALE / weight` non-zero for every accepted weight
/// (`1 ..= u32::MAX`) and every cost `>= 1`, so proportional service survives
/// uniform rescaling of all weights — the property a smaller scale loses to
/// integer division.
const WFQ_SCALE: u128 = 1_u128 << 64;

/// Assign the scheduling tags a request carries while queued, and update the
/// class's WFQ cursor.
pub fn enqueue_tags(
    state: &mut GovernedState,
    class: &TaskClass,
    policy: &ClassPolicy,
    cost_cpu_units: u32,
    enqueued_at_ms: u64,
) -> (u128, u64) {
    let finish_tag = match policy.fairness {
        FairnessPolicy::WeightedFairQueue { weight, .. } => {
            let increment = wfq_increment(cost_cpu_units, weight);
            let base = {
                let virtual_time = state.virtual_time;
                let cstate = state.class_mut(class);
                cstate.last_finish_tag.max(virtual_time)
            };
            let tag = if let Some(tag) = base.checked_add(increment) {
                tag
            } else {
                // Rebasing keeps every *difference* between tags identical, so
                // proportional service is preserved across the reset.
                rebase_virtual_time(state);
                let virtual_time = state.virtual_time;
                let cstate = state.class_mut(class);
                cstate
                    .last_finish_tag
                    .max(virtual_time)
                    .saturating_add(increment)
            };
            state.class_mut(class).last_finish_tag = tag;
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

/// The virtual-time increment one request of `cost_cpu_units` adds to a class of
/// `weight`. A zero weight is treated as 1 — [`crate::Governor::validate_policy`]
/// rejects it at construction, so this is a defensive floor, not a coercion that
/// silently equalizes distinct weights.
fn wfq_increment(cost_cpu_units: u32, weight: u32) -> u128 {
    let weight = u128::from(weight.max(1));
    (u128::from(cost_cpu_units.max(1)) * WFQ_SCALE) / weight
}

/// Shift every virtual-time quantity down by the current global virtual time.
/// Differences — the only thing WFQ ordering depends on — are unchanged.
fn rebase_virtual_time(state: &mut GovernedState) {
    let origin = state.virtual_time;
    state.virtual_time = 0;
    for cstate in state.classes.values_mut() {
        cstate.last_finish_tag = cstate.last_finish_tag.saturating_sub(origin);
        for request in &mut cstate.queue {
            request.finish_tag = request.finish_tag.saturating_sub(origin);
        }
    }
}

/// Re-derive a class's WFQ baseline after a queued request was removed without
/// being served.
///
/// A cancelled request never consumed service, so it must not leave virtual-time
/// debt behind: a class that cancels ten requests would otherwise be pushed
/// behind a peer that arrived later. Recomputing from the surviving queue is
/// exact for head, middle, and tail removal, and preserves the relative order of
/// everything still queued.
///
/// Also resets DRR credit when the queue *drains*. Draining ends a busy period;
/// capacity-blocking does not, so a blocked-but-nonempty queue keeps its credit.
///
/// Cost is `O(remaining queue)`, bounded by the class's `max_queue_depth`.
pub fn on_queue_changed(state: &mut GovernedState, class: &TaskClass) {
    let virtual_time = state.virtual_time;
    let cstate = state.class_mut(class);
    if cstate.queue.is_empty() {
        cstate.last_finish_tag = virtual_time;
        cstate.deficit = 0;
        return;
    }
    let max_tag = cstate
        .queue
        .iter()
        .map(|request| request.finish_tag)
        .max()
        .unwrap_or(virtual_time);
    cstate.last_finish_tag = max_tag.max(virtual_time);
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

/// Whether any class could be promoted right now. Pure: unlike [`select`] it
/// moves no DRR deficit, no ring cursor, and no WFQ virtual time, so a caller
/// that has run out of promotion budget can ask "is there more?" without
/// charging a class for a dispatch that will not happen in this pass.
pub fn has_runnable(state: &GovernedState, policies: &PolicySet) -> bool {
    !runnable_candidates(state, policies, &policies.resources).is_empty()
}

/// Pick the next class to promote, or `None` if nothing is runnable under the
/// current budget. Mutates DRR deficits / WFQ virtual time as a side effect of
/// the dispatch decision, so it must only be called for a dispatch that will
/// actually happen.
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
        // Both authorities, one check: semantic capacity and the physical
        // capability the head froze at intake.
        if state.capacity_for(
            class,
            policy.max_inflight,
            head.cost,
            head.capability.as_deref(),
            budget,
            policies.capability_limits(),
        ) != CapacityBlock::Ok
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
/// while its deficit still covers its head cost. A quantum-`N` class is served
/// about `N` times per lap before yielding, giving service *proportional* to
/// quanta with a bounded difference.
///
/// The ring is walked **arithmetically**. Instead of visiting classes one at a
/// time until somebody can afford its head, each class's next affordable visit
/// is computed in closed form and the earliest one wins; every class is then
/// credited exactly the quanta those skipped rounds owed it. Same selection, same
/// deficits, `O(classes)` work — the naive loop is `O(cost / quantum)`, which the
/// public policy domain lets reach `u32::MAX` iterations under the state mutex.
fn drr_select(state: &mut GovernedState, pool: &[&Candidate]) -> Option<TaskClass> {
    if pool.is_empty() {
        return None;
    }

    // Keep serving the current cursor class while its deficit covers its head
    // cost — this is what lets a quantum-N class run ~N in a row.
    if let Some(cur) = state.drr_cursor.clone() {
        if let Some(c) = pool.iter().find(|c| c.class == cur) {
            let cost = u128::from(c.head_cost_cpu.max(1));
            if state.classes.get(&cur).map_or(0, |s| s.deficit) >= cost {
                state.class_mut(&cur).deficit -= cost;
                return Some(cur);
            }
        }
    }

    let ring_size = u128::try_from(pool.len()).ok()?;
    let start = match &state.drr_cursor {
        Some(cur) => pool.iter().position(|c| c.class > *cur).unwrap_or(0),
        None => 0,
    };

    // For ring slot `k`, its visits land at sequence positions
    // `offset_k + m*ring_size` (m = 0, 1, …). It can afford its head on the
    // visit that first brings `deficit + (m+1)*quantum` to `cost`.
    struct PlannedVisit {
        class: TaskClass,
        quantum: u128,
        cost: u128,
        offset: u128,
        visit_pos: u128,
    }

    // One examined position per runnable class, whatever the head costs.
    state.drr_ring_visits = state
        .drr_ring_visits
        .saturating_add(u64::try_from(pool.len()).unwrap_or(u64::MAX));
    let mut plan: Vec<PlannedVisit> = Vec::with_capacity(pool.len());
    for (index, candidate) in pool.iter().enumerate() {
        let offset = u128::try_from((index + pool.len() - start) % pool.len()).ok()?;
        let quantum = u128::from(drr_quantum(candidate.fairness));
        let cost = u128::from(candidate.head_cost_cpu.max(1));
        let deficit = state
            .classes
            .get(&candidate.class)
            .map_or(0, |cstate| cstate.deficit);
        let visits_needed = if deficit >= cost {
            1
        } else {
            (cost - deficit).div_ceil(quantum)
        };
        plan.push(PlannedVisit {
            class: candidate.class.clone(),
            quantum,
            cost,
            offset,
            visit_pos: offset + (visits_needed - 1) * ring_size,
        });
    }

    // Offsets are distinct modulo the ring size, so exactly one planned visit
    // holds the earliest position.
    let winning_pos = plan.iter().map(|visit| visit.visit_pos).min()?;
    let (winning_class, winning_cost) = plan
        .iter()
        .find(|visit| visit.visit_pos == winning_pos)
        .map(|visit| (visit.class.clone(), visit.cost))?;

    // Credit every class for the visits that actually occurred on the way.
    for visit in &plan {
        if visit.offset > winning_pos {
            continue;
        }
        let visits = (winning_pos - visit.offset) / ring_size + 1;
        let credit = visits.saturating_mul(visit.quantum);
        let cstate = state.class_mut(&visit.class);
        cstate.deficit = cstate.deficit.saturating_add(credit);
    }

    let cstate = state.class_mut(&winning_class);
    debug_assert!(
        cstate.deficit >= winning_cost,
        "DRR winner must be able to afford its head cost"
    );
    cstate.deficit = cstate.deficit.saturating_sub(winning_cost);
    state.drr_cursor = Some(winning_class.clone());
    Some(winning_class)
}

fn drr_quantum(fairness: FairnessPolicy) -> u32 {
    match fairness {
        FairnessPolicy::DeficitRoundRobin { quantum } => quantum.max(1),
        _ => 1,
    }
}
