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

use taskmesh_contract::{ClassPolicy, FairnessPolicy, TaskClass};

use crate::engine::state::{ClassState, GovernedState};
use crate::features::admission::pending::{self, CapacityAssessment};
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
) -> (u128, u128, u64) {
    let (finish_tag, wfq_arrival_tag) = match policy.fairness {
        FairnessPolicy::WeightedFairQueue { weight, .. } => {
            let increment = wfq_increment(cost_cpu_units, weight);
            let mut arrival = state.virtual_time;
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
                arrival = state.virtual_time;
                let virtual_time = state.virtual_time;
                let cstate = state.class_mut(class);
                cstate
                    .last_finish_tag
                    .max(virtual_time)
                    .saturating_add(increment)
            };
            state.class_mut(class).last_finish_tag = tag;
            (tag, arrival)
        }
        _ => (0, 0),
    };
    let deadline_ms = match policy.fairness {
        FairnessPolicy::DeadlineAware { slack_ms } => enqueued_at_ms.saturating_add(slack_ms),
        _ => u64::MAX,
    };
    (finish_tag, wfq_arrival_tag, deadline_ms)
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
        cstate.last_served_finish_tag = cstate.last_served_finish_tag.saturating_sub(origin);
        for request in &mut cstate.queue {
            request.finish_tag = request.finish_tag.saturating_sub(origin);
            request.wfq_arrival_tag = request.wfq_arrival_tag.saturating_sub(origin);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{has_runnable, rebase_virtual_time};
    use crate::engine::state::{ClassState, GovernedState};
    use crate::shared::PolicySet;
    use std::collections::BTreeMap;
    use taskmesh_contract::{ClassPolicy, ResourceBudget, TaskClass};

    #[test]
    fn has_runnable_is_false_when_every_class_queue_is_empty() {
        let class = TaskClass::new("idle");
        let policies = PolicySet::new(
            ResourceBudget::new(),
            BTreeMap::from([(class.clone(), ClassPolicy::new())]),
        );
        let mut state = GovernedState::default();
        state.classes.insert(class, ClassState::default());

        assert!(
            !has_runnable(&state, &policies),
            "an empty active-class ring cannot owe a promotion continuation"
        );
    }

    #[test]
    fn virtual_time_rebase_preserves_relative_finish_distances() {
        let mut state = GovernedState::default();
        state.virtual_time = 40;
        state.classes.insert(
            TaskClass::new("a"),
            ClassState {
                last_finish_tag: 55,
                ..ClassState::default()
            },
        );
        state.classes.insert(
            TaskClass::new("b"),
            ClassState {
                last_finish_tag: 75,
                ..ClassState::default()
            },
        );

        rebase_virtual_time(&mut state);

        assert_eq!(state.virtual_time, 0);
        assert_eq!(state.classes[&TaskClass::new("a")].last_finish_tag, 15);
        assert_eq!(state.classes[&TaskClass::new("b")].last_finish_tag, 35);
    }
}

/// Re-derive a class's WFQ tags after a queued request was removed without
/// receiving service.
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
pub fn on_unserved_removed(state: &mut GovernedState, class: &TaskClass, policy: &ClassPolicy) {
    let virtual_time = state.virtual_time;
    let cstate = state.class_mut(class);
    if cstate.queue.is_empty() {
        cstate.last_finish_tag = virtual_time;
        cstate.last_served_finish_tag = virtual_time;
        cstate.deficit = 0;
        return;
    }
    if let FairnessPolicy::WeightedFairQueue { weight, .. } = policy.fairness {
        rebuild_wfq_queue(cstate, weight, virtual_time);
    } else {
        cstate.last_finish_tag = cstate
            .queue
            .iter()
            .map(|request| request.finish_tag)
            .max()
            .unwrap_or(virtual_time)
            .max(virtual_time);
    }
}

/// Record a request that left the queue because it received service.
///
/// The service tag is computed from actual prior service, rather than from the
/// request's original queue position. A request may overtake a head blocked on
/// another capability pool; the skipped request is still unserved and must not
/// be counted in the class baseline. Only that non-head service changes the
/// canonical order of the surviving tags; ordinary head service leaves the
/// already-derived tail intact and runs in `O(1)` under the governor lock.
pub fn on_request_served(
    state: &mut GovernedState,
    class: &TaskClass,
    policy: &ClassPolicy,
    queue_index: usize,
    service_finish_tag: u128,
) {
    let virtual_time = state.virtual_time;
    let cstate = state.class_mut(class);
    if cstate.queue.is_empty() {
        cstate.last_finish_tag = virtual_time;
        cstate.last_served_finish_tag = virtual_time;
        cstate.deficit = 0;
        return;
    }
    cstate.last_served_finish_tag = cstate.last_served_finish_tag.max(service_finish_tag);
    if queue_index > 0 {
        if let FairnessPolicy::WeightedFairQueue { weight, .. } = policy.fairness {
            rebuild_wfq_queue(cstate, weight, virtual_time);
        }
    }
}

fn rebuild_wfq_queue(cstate: &mut ClassState, weight: u32, virtual_time: u128) {
    #[cfg(feature = "test-util")]
    {
        cstate.wfq_reprice_visits = cstate
            .wfq_reprice_visits
            .saturating_add(u64::try_from(cstate.queue.len()).unwrap_or(u64::MAX));
    }
    let mut cursor = cstate.last_served_finish_tag;
    for request in &mut cstate.queue {
        let start = cursor.max(request.wfq_arrival_tag);
        request.finish_tag = start.saturating_add(wfq_increment(request.cost.cpu_units, weight));
        cursor = request.finish_tag;
    }
    cstate.last_finish_tag = cursor.max(virtual_time);
}

/// A runnable class and the head request driving its dispatch key.
struct Candidate {
    class: TaskClass,
    queue_index: usize,
    fairness: FairnessPolicy,
    best_effort: bool,
    head_seq: u64,
    head_finish_tag: u128,
    head_deadline_ms: u64,
    head_cost_cpu: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub class: TaskClass,
    pub queue_index: usize,
    pub service_finish_tag: u128,
}

/// Whether any class could be promoted right now. Pure: unlike [`select`] it
/// moves no DRR deficit, no ring cursor, and no WFQ virtual time, so a caller
/// that has run out of promotion budget can ask "is there more?" without
/// charging a class for a dispatch that will not happen in this pass.
pub fn has_runnable(state: &GovernedState, policies: &PolicySet) -> bool {
    !runnable_candidates(state, policies).is_empty()
}

/// Pick the next class to promote, or `None` if nothing is runnable under the
/// current budget. Mutates DRR deficits / WFQ virtual time as a side effect of
/// the dispatch decision, so it must only be called for a dispatch that will
/// actually happen.
pub fn select(state: &mut GovernedState, policies: &PolicySet) -> Option<Selection> {
    let candidates = runnable_candidates(state, policies);
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
        FairnessPolicy::DeficitRoundRobin { .. } => drr_select(state, &pool)?,
    };

    // WFQ: advance global virtual time past the dispatched request's tag.
    if let FairnessPolicy::WeightedFairQueue { .. } = discipline {
        if let Some(c) = pool.iter().find(|c| c.class == chosen_class) {
            state.virtual_time = state.virtual_time.max(c.head_finish_tag);
        }
    }
    let chosen = candidates
        .iter()
        .find(|candidate| candidate.class == chosen_class)?;
    Some(Selection {
        class: chosen_class,
        queue_index: chosen.queue_index,
        service_finish_tag: chosen.head_finish_tag,
    })
}

fn runnable_candidates(state: &GovernedState, policies: &PolicySet) -> Vec<Candidate> {
    let mut out = Vec::new();
    for (class, cstate) in &state.classes {
        let Some(policy) = policies.class(class) else {
            continue;
        };
        let Some((queue_index, head)) = cstate.queue.iter().enumerate().find(|(index, request)| {
            !pending::queued_behind_index(&cstate.queue, *index)
                && matches!(
                    pending::assess_pending(state, policies, request, policy.max_inflight, false),
                    CapacityAssessment::Runnable
                )
        }) else {
            continue;
        };
        let service_finish_tag = match policy.fairness {
            FairnessPolicy::WeightedFairQueue { .. } if queue_index == 0 => head.finish_tag,
            FairnessPolicy::WeightedFairQueue { weight, .. } => cstate
                .last_served_finish_tag
                .max(head.wfq_arrival_tag)
                .saturating_add(wfq_increment(head.cost.cpu_units, weight)),
            _ => head.finish_tag,
        };
        out.push(Candidate {
            class: class.clone(),
            queue_index,
            fairness: policy.fairness,
            best_effort: policy.best_effort,
            head_seq: head.seq_no,
            head_finish_tag: service_finish_tag,
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
