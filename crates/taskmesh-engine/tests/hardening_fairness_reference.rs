//! Fairness regressions (H16-007), each checked against an independent model
//! rather than against the production helper that produced the behavior.
//!
//! * **TM16-012** — DRR credited one quantum per ring visit, so a head costing
//!   `K` quanta needed `K` visits. The public policy domain allows `quantum = 1`
//!   and `cost = u32::MAX`, i.e. four billion iterations, all of them inside the
//!   global state mutex.
//! * **TM16-013** — WFQ increments were `cost * 1_000_000 / weight`. Scale every
//!   weight in a system up and the integer division floors every increment to
//!   zero: weighted share silently becomes FIFO, and the configuration that
//!   caused it looks fine.
//! * **TM16-014** — a cancelled request left its virtual finish tag behind, so a
//!   class that cancelled ten requests it never ran was pushed behind a peer
//!   that arrived later.
//! * **TM16-037** — a drained queue kept its residual deficit, so a class that
//!   idled through twenty busy periods came back holding twenty quanta and ran
//!   its whole backlog ahead of a peer that had been queued the entire time.

mod harness;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use harness::*;
use taskmesh_contract::{
    ClassPolicy, FairnessPolicy, ManualClock, OverflowPolicy, ResourceBudget, TaskClass, TaskSpec,
};
use taskmesh_engine::{
    AdmissionDecision, ClaimOutcome, Governor, PermitId, PolicySet, ReleaseOutcome,
};

// ---- an independent DRR reference ------------------------------------------

/// A deliberately naive deficit round-robin: it credits one quantum per visit
/// and walks the ring one class at a time, exactly as the textbook describes.
///
/// This is the oracle. It is slow by construction — that is the point, because
/// the production scheduler's job is to produce *this* answer without doing
/// *this* work.
struct ReferenceDrr {
    quanta: BTreeMap<String, u64>,
    deficits: BTreeMap<String, u64>,
    queues: BTreeMap<String, VecDeque<u64>>,
    cursor: Option<String>,
    /// Classes whose head cannot run right now (capacity-blocked). They are
    /// not visited, and — a capacity block does not end a busy period — they
    /// keep whatever credit they had.
    blocked: BTreeSet<String>,
}

impl ReferenceDrr {
    fn new(quanta: BTreeMap<String, u64>) -> Self {
        Self {
            deficits: quanta.keys().map(|k| (k.clone(), 0)).collect(),
            queues: quanta
                .keys()
                .map(|k| (k.clone(), VecDeque::new()))
                .collect(),
            quanta,
            cursor: None,
            blocked: BTreeSet::new(),
        }
    }

    fn set_blocked(&mut self, class: &str, blocked: bool) {
        if blocked {
            self.blocked.insert(class.to_string());
        } else {
            self.blocked.remove(class);
        }
    }

    fn enqueue(&mut self, class: &str, cost: u64) {
        self.queues
            .get_mut(class)
            .expect("known class")
            .push_back(cost);
    }

    fn candidates(&self) -> Vec<String> {
        self.queues
            .iter()
            .filter(|(class, queue)| !queue.is_empty() && !self.blocked.contains(*class))
            .map(|(class, _)| class.clone())
            .collect()
    }

    /// Serve one request, returning the class served.
    fn select(&mut self) -> Option<String> {
        let pool = self.candidates();
        if pool.is_empty() {
            return None;
        }
        // Keep serving the cursor class while it can still afford its head.
        if let Some(cursor) = self.cursor.clone() {
            if pool.contains(&cursor) {
                let cost = *self.queues[&cursor].front().expect("non-empty");
                if self.deficits[&cursor] >= cost {
                    *self.deficits.get_mut(&cursor).expect("known") -= cost;
                    self.pop(&cursor);
                    return Some(cursor);
                }
            }
        }
        // Walk the ring one visit at a time.
        let start = match &self.cursor {
            Some(cursor) => pool.iter().position(|c| c > cursor).unwrap_or(0),
            None => 0,
        };
        let mut index = start;
        loop {
            let class = pool[index].clone();
            let quantum = self.quanta[&class];
            *self.deficits.get_mut(&class).expect("known") += quantum;
            let cost = *self.queues[&class].front().expect("non-empty");
            if self.deficits[&class] >= cost {
                *self.deficits.get_mut(&class).expect("known") -= cost;
                self.cursor = Some(class.clone());
                self.pop(&class);
                return Some(class);
            }
            index = (index + 1) % pool.len();
        }
    }

    fn pop(&mut self, class: &str) {
        let queue = self.queues.get_mut(class).expect("known class");
        queue.pop_front();
        if queue.is_empty() {
            // A drained queue ends the busy period: unused credit does not carry
            // into the next one.
            *self.deficits.get_mut(class).expect("known class") = 0;
        }
    }
}

// ---- TM16-012 / TM16-037: DRR ----------------------------------------------

fn drr_governor(classes: &[(&str, u32)], queue_depth: u32) -> Governor {
    let mut map: BTreeMap<TaskClass, ClassPolicy> = BTreeMap::new();
    for (name, quantum) in classes {
        map.insert(
            TaskClass::new((*name).to_string()),
            ClassPolicy::new()
                .max_inflight(1_000)
                .max_queue_depth(queue_depth)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth)
                .fairness(FairnessPolicy::DeficitRoundRobin { quantum: *quantum }),
        );
    }
    Governor::new(
        PolicySet::new(ResourceBudget::new().cpu_units(1), map),
        Arc::new(ManualClock::new(0)),
    )
    .expect("valid policy")
}

fn spec(class: &str, op: &str) -> TaskSpec {
    TaskSpec::io(TaskClass::new(class.to_string())).operation(op.to_string())
}

fn admit_one(g: &Governor, class: &str, op: &str) -> PermitId {
    match g.admit(&spec(class, op)) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("expected admission, got {other:?}"),
    }
}

fn queue_one(g: &Governor, class: &str, op: &str) -> u64 {
    match g.admit(&spec(class, op)) {
        AdmissionDecision::Queued { ticket } => ticket,
        other => panic!("expected a queued ticket, got {other:?}"),
    }
}

/// Release `holder` and report which class the freed unit went to.
fn serve_next(
    g: &Governor,
    tickets: &mut Vec<(u64, String)>,
    holder: PermitId,
) -> (String, PermitId) {
    assert_eq!(g.release(holder), ReleaseOutcome::Released);
    let index = tickets
        .iter()
        .position(|(ticket, _)| matches!(g.ticket_status(*ticket), ClaimOutcome::Ready(_)))
        .expect("exactly one request is promoted per freed unit");
    let (ticket, class) = tickets.remove(index);
    let ClaimOutcome::Ready(permit) = g.claim(ticket) else {
        panic!("ticket {ticket} was promoted a moment ago");
    };
    (class, permit)
}

#[test]
fn drr_selection_matches_the_reference_for_mixed_quanta_and_costs() {
    // Unequal quanta, several classes, repeated refills: the production
    // scheduler's order must equal the naive reference's order exactly.
    let classes = [("a", 3u32), ("b", 1), ("c", 2)];
    let g = drr_governor(&classes, 64);
    let mut reference = ReferenceDrr::new(
        classes
            .iter()
            .map(|(name, quantum)| ((*name).to_string(), u64::from(*quantum)))
            .collect(),
    );

    let holder = admit_one(&g, "a", "holder");
    let mut tickets = Vec::new();
    for round in 0..4 {
        for (name, _) in &classes {
            for i in 0..3 {
                tickets.push((
                    queue_one(&g, name, &format!("{name}{round}{i}")),
                    (*name).to_string(),
                ));
                reference.enqueue(name, 1);
            }
        }
    }

    let mut observed = Vec::new();
    let mut expected = Vec::new();
    let mut holder = holder;
    while !tickets.is_empty() {
        let (class, permit) = serve_next(&g, &mut tickets, holder);
        observed.push(class);
        expected.push(reference.select().expect("the reference still has work"));
        holder = permit;
    }
    assert_eq!(g.release(holder), ReleaseOutcome::Released);
    assert_eq!(
        observed, expected,
        "production order must equal the reference"
    );
}

#[test]
fn drr_selection_matches_the_reference_for_heterogeneous_costs() {
    // Every other DRR fixture costs one unit per request, which leaves the
    // arithmetic ring walk's closed forms degenerate: `cost - deficit` is
    // always one, one visit always suffices, and the credit for skipped rounds
    // is always a single quantum. Heterogeneous costs are where the closed
    // form has to agree with the naive walk: a head costing several quanta is
    // served on a later lap, and every class passed on the way is credited
    // exactly the laps it was passed. One release frees the whole budget, so
    // capacity never filters a candidate and the order is the scheduler's
    // alone; fewer than `PROMOTION_BUDGET` requests keeps it to one pass.
    let classes = [("a", 1u32, 3u32), ("b", 2, 1), ("c", 1, 2)]; // (name, quantum, cost)
    let per_class = 6u32;
    let total: u32 = classes.iter().map(|(_, _, cost)| cost * per_class).sum();
    let queued_total = usize::try_from(per_class).expect("small") * classes.len();
    assert!(
        queued_total < taskmesh_engine::PROMOTION_BUDGET,
        "the whole backlog is promoted in one pass"
    );
    let mut map: BTreeMap<TaskClass, ClassPolicy> = BTreeMap::new();
    for (name, quantum, cost) in &classes {
        map.insert(
            TaskClass::new(*name),
            ClassPolicy::new()
                .max_inflight(1_000)
                .max_queue_depth(1_000)
                .cpu_units(*cost)
                .overflow_policy(OverflowPolicy::QueueWithinDepth)
                .fairness(FairnessPolicy::DeficitRoundRobin { quantum: *quantum }),
        );
    }
    map.insert(
        TaskClass::new("holder"),
        ClassPolicy::new()
            .max_inflight(1)
            .cpu_units(total)
            .fairness(FairnessPolicy::DeficitRoundRobin { quantum: 1 }),
    );
    let g = Governor::new(
        PolicySet::new(ResourceBudget::new().cpu_units(total), map),
        Arc::new(ManualClock::new(0)),
    )
    .expect("valid policy");
    let mut reference = ReferenceDrr::new(
        classes
            .iter()
            .map(|(name, quantum, _)| ((*name).to_string(), u64::from(*quantum)))
            .collect(),
    );

    let holder = admit_one(&g, "holder", "holder");
    let mut tickets: Vec<(u64, String)> = Vec::new();
    for round in 0..per_class {
        for (name, _, cost) in &classes {
            tickets.push((
                queue_one(&g, name, &format!("{name}{round}")),
                (*name).to_string(),
            ));
            reference.enqueue(name, u64::from(*cost));
        }
    }

    // One release frees exactly the backlog's total cost.
    assert_eq!(g.release(holder), ReleaseOutcome::Released);
    let mut promoted: Vec<(PermitId, String)> = tickets
        .iter()
        .filter_map(|(ticket, class)| match g.ticket_status(*ticket) {
            ClaimOutcome::Ready(permit) => Some((permit, class.clone())),
            _ => None,
        })
        .collect();
    promoted.sort_by_key(|(permit, _)| *permit);
    assert_eq!(
        promoted.len(),
        queued_total,
        "every queued request fits the freed budget, so all of them are promoted"
    );
    let observed: Vec<String> = promoted.iter().map(|(_, class)| class.clone()).collect();
    let expected: Vec<String> = (0..queued_total)
        .map(|_| reference.select().expect("the reference still has work"))
        .collect();
    assert_eq!(
        observed, expected,
        "heterogeneous-cost DRR order must equal the reference walk"
    );

    for (ticket, _) in &tickets {
        let ClaimOutcome::Ready(permit) = g.claim(*ticket) else {
            panic!("promoted by the single pass");
        };
        assert_eq!(g.release(permit), ReleaseOutcome::Released);
    }
    assert_eq!(g.snapshot().conservation_violation(), None);
}

#[test]
fn residual_credit_kept_through_a_capacity_block_is_spent_on_the_first_visit_back() {
    // Every other DRR fixture has each class's deficit below its head cost
    // whenever the ring is planned: a class is served the moment it can afford
    // its head, and a drained queue resets its credit. The one way a runnable
    // *non-cursor* class shows up already able to afford its head is a
    // capacity block: `x` (quantum 4, cost 1, `max_inflight` 1) is served once,
    // keeps a residual deficit of 3, is then excluded from the ring because its
    // own quota is full, and comes back runnable when its permit releases. On
    // that lap the planner must (a) not compute `cost - deficit` (it
    // underflows) and (b) place `x` on its very first visit, ahead of the
    // cursor's successor — exactly what the naive walk does.
    let mut map: BTreeMap<TaskClass, ClassPolicy> = BTreeMap::new();
    map.insert(
        TaskClass::new("x"),
        ClassPolicy::new()
            .max_inflight(1)
            .max_queue_depth(16)
            .cpu_units(1)
            .overflow_policy(OverflowPolicy::QueueWithinDepth)
            .fairness(FairnessPolicy::DeficitRoundRobin { quantum: 4 }),
    );
    map.insert(
        TaskClass::new("y"),
        ClassPolicy::new()
            .max_inflight(16)
            .max_queue_depth(16)
            .cpu_units(1)
            .overflow_policy(OverflowPolicy::QueueWithinDepth)
            .fairness(FairnessPolicy::DeficitRoundRobin { quantum: 1 }),
    );
    map.insert(
        TaskClass::new("h"),
        ClassPolicy::new()
            .max_inflight(16)
            .cpu_units(1)
            .fairness(FairnessPolicy::DeficitRoundRobin { quantum: 1 }),
    );
    let g = Governor::new(
        PolicySet::new(ResourceBudget::new().cpu_units(2), map),
        Arc::new(ManualClock::new(0)),
    )
    .expect("valid policy");
    let mut reference = ReferenceDrr::new(BTreeMap::from([
        ("x".to_string(), 4u64),
        ("y".to_string(), 1u64),
    ]));

    // Two holders fill the cpu budget; the backlog queues behind them.
    let h1 = admit_one(&g, "h", "h1");
    let h2 = admit_one(&g, "h", "h2");
    let mut tickets: Vec<(u64, String)> = Vec::new();
    for op in ["x0", "x1"] {
        tickets.push((queue_one(&g, "x", op), "x".to_string()));
        reference.enqueue("x", 1);
    }
    for op in ["y0", "y1", "y2"] {
        tickets.push((queue_one(&g, "y", op), "y".to_string()));
        reference.enqueue("y", 1);
    }

    // Lap 1: `x` is served first (deficit 0 + 4 ≥ 1 → residual 3) and is now
    // at its inflight quota: blocked, credit kept.
    let (served, x_permit) = serve_next(&g, &mut tickets, h1);
    assert_eq!(served, "x");
    assert_eq!(reference.select().as_deref(), Some("x"));
    reference.set_blocked("x", true);

    // Lap 2: with `x` blocked the ring is `y` alone.
    let (served, y_permit) = serve_next(&g, &mut tickets, h2);
    assert_eq!(served, "y");
    assert_eq!(reference.select().as_deref(), Some("y"));

    // `x` comes back runnable with deficit 3 ≥ cost 1 while the cursor sits on
    // `y`. Releasing x's own permit both frees a unit and lifts the block; the
    // freed unit must go to `x` on its first visit — not to `y`, which is what
    // treating the residual as "needs another lap" (or a wrapped
    // `cost - deficit`) would do.
    reference.set_blocked("x", false);
    let (served, x_permit_2) = serve_next(&g, &mut tickets, x_permit);
    assert_eq!(
        served, "x",
        "a class whose kept credit already covers its head is served on its first visit back"
    );
    assert_eq!(reference.select().as_deref(), Some("x"));
    reference.set_blocked("x", true);

    // Drain the rest in lockstep with the reference; every remaining request
    // is `y`, and the orders must agree to the end.
    let mut holder = y_permit;
    while !tickets.is_empty() {
        let (served, permit) = serve_next(&g, &mut tickets, holder);
        assert_eq!(Some(served.as_str()), reference.select().as_deref());
        holder = permit;
    }
    assert_eq!(g.release(holder), ReleaseOutcome::Released);
    assert_eq!(g.release(x_permit_2), ReleaseOutcome::Released);
    assert_eq!(g.snapshot().conservation_violation(), None);
}

#[test]
fn drr_order_is_exact_across_the_promotion_budget_boundary() {
    // One release frees far more than `PROMOTION_BUDGET` units at once, so
    // promotion runs as several bounded passes with the lock dropped between
    // them. The order across that seam must still be the reference's order:
    // a pass that ran out of budget must not have charged a class for a
    // dispatch it did not make (which shifts the ring by one every 64 grants).
    let classes = [("a", 3u32), ("b", 1), ("c", 2)];
    let budget = u32::try_from(taskmesh_engine::PROMOTION_BUDGET).expect("small");
    let total = budget * 2 + 7; // straddles two boundaries
    let mut map: BTreeMap<TaskClass, ClassPolicy> = BTreeMap::new();
    for (name, quantum) in &classes {
        map.insert(
            TaskClass::new(*name),
            ClassPolicy::new()
                .max_inflight(1_000)
                .max_queue_depth(1_000)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth)
                .fairness(FairnessPolicy::DeficitRoundRobin { quantum: *quantum }),
        );
    }
    // A holder class whose one permit occupies the entire budget.
    map.insert(
        TaskClass::new("holder"),
        ClassPolicy::new()
            .max_inflight(1)
            .cpu_units(total)
            .fairness(FairnessPolicy::DeficitRoundRobin { quantum: 1 }),
    );
    let g = Governor::new(
        PolicySet::new(ResourceBudget::new().cpu_units(total), map),
        Arc::new(ManualClock::new(0)),
    )
    .expect("valid policy");
    let mut reference = ReferenceDrr::new(
        classes
            .iter()
            .map(|(name, quantum)| ((*name).to_string(), u64::from(*quantum)))
            .collect(),
    );

    let holder = admit_one(&g, "holder", "holder");
    let mut tickets: Vec<(u64, String)> = Vec::new();
    let per_class = usize::try_from(total).expect("small") / classes.len() + 1;
    for round in 0..per_class {
        for (name, _) in &classes {
            tickets.push((
                queue_one(&g, name, &format!("{name}{round}")),
                (*name).to_string(),
            ));
            reference.enqueue(name, 1);
        }
    }
    let queued_total = tickets.len();
    assert!(
        queued_total > usize::try_from(total).expect("small"),
        "more queued than fits, so the tail stays queued"
    );

    // One release: `total` units come free and are handed out in passes.
    assert_eq!(g.release(holder), ReleaseOutcome::Released);

    // Promotion order is recovered from the permit ids: the engine assigns them
    // in grant order.
    let mut promoted: Vec<(PermitId, String)> = tickets
        .iter()
        .filter_map(|(ticket, class)| match g.ticket_status(*ticket) {
            ClaimOutcome::Ready(permit) => Some((permit, class.clone())),
            _ => None,
        })
        .collect();
    promoted.sort_by_key(|(permit, _)| *permit);
    assert_eq!(
        promoted.len(),
        usize::try_from(total).expect("small"),
        "every freed unit is used: the passes continue past the budget"
    );
    let observed: Vec<String> = promoted.into_iter().map(|(_, class)| class).collect();
    let expected: Vec<String> = (0..total)
        .map(|_| reference.select().expect("reference has work"))
        .collect();
    assert_eq!(
        observed, expected,
        "order must equal the reference across every budget seam"
    );

    let snapshot = g.snapshot();
    assert_eq!(snapshot.conservation_violation(), None);
    let queued_left: u32 = snapshot.classes.values().map(|c| c.queued).sum();
    assert_eq!(
        usize::try_from(queued_left).expect("small"),
        queued_total - usize::try_from(total).expect("small")
    );
}

/// A waker that, when woken, submits a new request of the same class — the
/// only vantage point from which the promotion budget's continuation gap is
/// observable: the first pass has granted `PROMOTION_BUDGET` permits, dropped
/// the lock, and is notifying waiters before it resumes.
struct SubmittingWaker {
    governor: Arc<Governor>,
    outcome: std::sync::Mutex<Option<(AdmissionDecision, Option<taskmesh_engine::CapacityBlock>)>>,
}

impl taskmesh_contract::PermitWaker for SubmittingWaker {
    fn wake(&self) {
        let mut slot = self.outcome.lock().expect("lock");
        if slot.is_some() {
            return;
        }
        let decision = self.governor.admit(&spec("c", "newcomer"));
        let blocked_on = match &decision {
            AdmissionDecision::Queued { ticket } => self.governor.pending_block_reason(*ticket),
            _ => None,
        };
        *slot = Some((decision, blocked_on));
    }
}

#[test]
fn a_newcomer_queues_behind_a_runnable_head_of_its_own_class() {
    // D08 in-class FIFO across the promotion budget's continuation gap. One
    // release frees far more than `PROMOTION_BUDGET` units; the first pass
    // grants 64, releases the lock, and wakes those 64 waiters. The first
    // waiter submits a *new* request of the same class. Capacity is free and
    // 86 earlier arrivals are still queued and runnable — waiting only for the
    // continuation pass. The newcomer must queue behind them (and say so), and
    // be granted after every one of them.
    let budget = taskmesh_engine::PROMOTION_BUDGET;
    let queued_total = budget * 2 + 22; // 150: two full passes and a tail
    let units = u32::try_from(queued_total + 1).expect("small"); // room for all + newcomer
    let mut map: BTreeMap<TaskClass, ClassPolicy> = BTreeMap::new();
    map.insert(
        TaskClass::new("c"),
        ClassPolicy::new()
            .max_inflight(1_000)
            .max_queue_depth(1_000)
            .cpu_units(1)
            .overflow_policy(OverflowPolicy::QueueWithinDepth)
            .fairness(FairnessPolicy::Fifo),
    );
    map.insert(
        TaskClass::new("holder"),
        ClassPolicy::new().max_inflight(1).cpu_units(units),
    );
    let g = Arc::new(
        Governor::new(
            PolicySet::new(ResourceBudget::new().cpu_units(units), map),
            Arc::new(ManualClock::new(0)),
        )
        .expect("valid policy"),
    );

    let holder = admit_one(&g, "holder", "holder");
    let waker = Arc::new(SubmittingWaker {
        governor: Arc::clone(&g),
        outcome: std::sync::Mutex::new(None),
    });
    let port: Arc<dyn taskmesh_contract::PermitWaker> = waker.clone();
    let AdmissionDecision::Queued { ticket: first } = g.admit_waitable(&spec("c", "q0"), port)
    else {
        panic!("the first request queues behind the holder");
    };
    let mut tickets = vec![first];
    for i in 1..queued_total {
        tickets.push(queue_one(&g, "c", &format!("q{i}")));
    }

    assert_eq!(g.release(holder), ReleaseOutcome::Released);

    let (decision, blocked_on) = waker
        .outcome
        .lock()
        .expect("lock")
        .take()
        .expect("the first waiter was woken by the first pass");
    let AdmissionDecision::Queued { ticket: newcomer } = decision else {
        panic!("newcomer must queue behind the runnable heads, got {decision:?}");
    };
    assert_eq!(
        blocked_on,
        Some(taskmesh_engine::CapacityBlock::QueuedBehind),
        "the block reason names the rule: capacity existed, earlier arrivals came first"
    );

    // After the drain everyone is promoted, in arrival order: the newcomer's
    // permit id is the largest of all.
    let mut permits: Vec<PermitId> = Vec::new();
    for ticket in &tickets {
        let ClaimOutcome::Ready(permit) = g.claim(*ticket) else {
            panic!("every earlier arrival is promoted by the continuation passes");
        };
        permits.push(permit);
    }
    let ClaimOutcome::Ready(last) = g.claim(newcomer) else {
        panic!("the newcomer is promoted once the queue ahead of it drains");
    };
    assert!(
        permits.iter().all(|p| *p < last),
        "the newcomer is granted after every earlier arrival"
    );
    for permit in permits.into_iter().chain(std::iter::once(last)) {
        assert_eq!(g.release(permit), ReleaseOutcome::Released);
    }
    assert_eq!(g.snapshot().conservation_violation(), None);
}

#[test]
fn a_head_blocked_on_another_capability_pool_does_not_hold_back_a_newcomer() {
    // The one D08 overtaking exception. `c` can run on the blocking pool or
    // the large-stack pool; the large-stack pool has one slot. With that slot
    // taken, a large-stack request queues (blocked on `Capability`). A newcomer
    // of the *same class* that needs only the blocking pool must be admitted
    // directly: holding it behind a head that waits for a different physical
    // pool would let one saturated pool idle every other pool the class can use.
    let mut map: BTreeMap<TaskClass, ClassPolicy> = BTreeMap::new();
    map.insert(
        TaskClass::new("c"),
        ClassPolicy::new()
            .max_inflight(8)
            .max_queue_depth(8)
            .cpu_units(1)
            .overflow_policy(OverflowPolicy::QueueWithinDepth),
    );
    let g = Governor::new(
        PolicySet::new(ResourceBudget::new().cpu_units(100), map)
            .with_capability_limits(BTreeMap::from([
                ("large_stack".to_string(), 1),
                ("blocking".to_string(), 10),
            ]))
            .expect("registered pools"),
        Arc::new(ManualClock::new(0)),
    )
    .expect("valid policy");

    let AdmissionDecision::Admitted { permit_id: holder } =
        g.admit_resolved(&spec("c", "holder"), Some("large_stack"), None)
    else {
        panic!("the single large-stack slot is free");
    };
    let AdmissionDecision::Queued { ticket: head } =
        g.admit_resolved(&spec("c", "head"), Some("large_stack"), None)
    else {
        panic!("the second large-stack request waits for the slot");
    };
    assert_eq!(
        g.pending_block_reason(head),
        Some(taskmesh_engine::CapacityBlock::Capability)
    );

    // Same class, different pool: not held back.
    let AdmissionDecision::Admitted {
        permit_id: newcomer,
    } = g.admit_resolved(&spec("c", "newcomer"), Some("blocking"), None)
    else {
        panic!("a newcomer on an unrelated pool must not wait for a capability-blocked head");
    };
    assert_eq!(g.snapshot().capabilities["blocking"].in_use, 1);
    assert_eq!(g.snapshot().capabilities["large_stack"].in_use, 1);
    // The head itself is still ordered: it runs when *its* pool frees.
    assert!(matches!(g.ticket_status(head), ClaimOutcome::Pending));
    assert_eq!(g.release(holder), ReleaseOutcome::Released);
    let ClaimOutcome::Ready(head_permit) = g.claim(head) else {
        panic!("the freed large-stack slot promotes the head");
    };
    for permit in [newcomer, head_permit] {
        assert_eq!(g.release(permit), ReleaseOutcome::Released);
    }
    assert_eq!(g.snapshot().conservation_violation(), None);
}

/// A waker that submits a newcomer of a *different* class when woken.
struct CrossClassSubmittingWaker {
    governor: Arc<Governor>,
    newcomer_class: &'static str,
    outcome: std::sync::Mutex<Option<(AdmissionDecision, Option<taskmesh_engine::CapacityBlock>)>>,
}

impl taskmesh_contract::PermitWaker for CrossClassSubmittingWaker {
    fn wake(&self) {
        let mut slot = self.outcome.lock().expect("lock");
        if slot.is_some() {
            return;
        }
        let decision = self.governor.admit(&spec(self.newcomer_class, "newcomer"));
        let blocked_on = match &decision {
            AdmissionDecision::Queued { ticket } => self.governor.pending_block_reason(*ticket),
            _ => None,
        };
        *slot = Some((decision, blocked_on));
    }
}

#[test]
fn a_newcomer_of_another_class_also_waits_for_the_pending_continuation() {
    // The continuation gap is the only moment capacity is free while runnable
    // work is queued. A newcomer of a *different* queueable class arriving in
    // that gap does not take the slot either: it queues, and the continuation
    // pass orders it against the earlier arrivals under the tier's discipline
    // (FIFO here → by arrival, so it is granted last). Outside the gap this
    // cannot happen at all, since every capacity-freeing transition promotes
    // under the same lock.
    let budget = taskmesh_engine::PROMOTION_BUDGET;
    let queued_total = budget + 36; // one full pass and a tail
    let units = u32::try_from(queued_total + 1).expect("small");
    let mut map: BTreeMap<TaskClass, ClassPolicy> = BTreeMap::new();
    for name in ["a", "b"] {
        map.insert(
            TaskClass::new(name),
            ClassPolicy::new()
                .max_inflight(1_000)
                .max_queue_depth(1_000)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth)
                .fairness(FairnessPolicy::Fifo),
        );
    }
    map.insert(
        TaskClass::new("holder"),
        ClassPolicy::new().max_inflight(1).cpu_units(units),
    );
    let g = Arc::new(
        Governor::new(
            PolicySet::new(ResourceBudget::new().cpu_units(units), map),
            Arc::new(ManualClock::new(0)),
        )
        .expect("valid policy"),
    );
    let holder = admit_one(&g, "holder", "holder");
    let waker = Arc::new(CrossClassSubmittingWaker {
        governor: Arc::clone(&g),
        newcomer_class: "b",
        outcome: std::sync::Mutex::new(None),
    });
    let port: Arc<dyn taskmesh_contract::PermitWaker> = waker.clone();
    let AdmissionDecision::Queued { ticket: first } = g.admit_waitable(&spec("a", "a0"), port)
    else {
        panic!("queues behind the holder");
    };
    let mut tickets = vec![first];
    for i in 1..queued_total {
        tickets.push(queue_one(&g, "a", &format!("a{i}")));
    }

    assert_eq!(g.release(holder), ReleaseOutcome::Released);

    let (decision, blocked_on) = waker
        .outcome
        .lock()
        .expect("lock")
        .take()
        .expect("woken by the first pass");
    let AdmissionDecision::Queued { ticket: newcomer } = decision else {
        panic!("a queueable newcomer must wait for the pending pass, got {decision:?}");
    };
    assert_eq!(
        blocked_on,
        Some(taskmesh_engine::CapacityBlock::QueuedBehind)
    );

    let mut permits: Vec<PermitId> = Vec::new();
    for ticket in &tickets {
        let ClaimOutcome::Ready(permit) = g.claim(*ticket) else {
            panic!("every earlier `a` arrival is promoted by the continuation");
        };
        permits.push(permit);
    }
    let ClaimOutcome::Ready(last) = g.claim(newcomer) else {
        panic!("the newcomer is promoted after them");
    };
    assert!(
        permits.iter().all(|p| *p < last),
        "FIFO across classes by arrival"
    );
    for permit in permits.into_iter().chain(std::iter::once(last)) {
        assert_eq!(g.release(permit), ReleaseOutcome::Released);
    }
    // Nothing owed once the drain finished: a later direct admit is direct.
    match g.admit(&spec("b", "later")) {
        AdmissionDecision::Admitted { permit_id } => {
            assert_eq!(g.release(permit_id), ReleaseOutcome::Released);
        }
        other => panic!("no continuation pending: direct admit expected, got {other:?}"),
    }
    assert_eq!(g.snapshot().conservation_violation(), None);
}

#[test]
fn an_unqueueable_newcomer_is_admitted_in_the_continuation_gap() {
    // The pending-continuation rule queues a gap arrival so the pass can order
    // it against the earlier arrivals — but only a class that *has* a queue
    // can be ordered that way. A `Reject`-overflow class whose memory policy
    // does not queue has nowhere to wait: routed to a queue it does not have,
    // it would be shed as `QueueFull` at depth zero while capacity sits free.
    // It is admitted instead — the one overtake the gap permits, and only by
    // work that could not have waited.
    let budget = taskmesh_engine::PROMOTION_BUDGET;
    let queued_total = budget + 36; // one full pass and a tail
    let units = u32::try_from(queued_total + 1).expect("small");
    let mut map: BTreeMap<TaskClass, ClassPolicy> = BTreeMap::new();
    map.insert(
        TaskClass::new("a"),
        ClassPolicy::new()
            .max_inflight(1_000)
            .max_queue_depth(1_000)
            .cpu_units(1)
            .overflow_policy(OverflowPolicy::QueueWithinDepth)
            .fairness(FairnessPolicy::Fifo),
    );
    // `Reject` overflow, `Reject` memory overcommit: no queue at all.
    map.insert(
        TaskClass::new("b"),
        ClassPolicy::new().max_inflight(1_000).cpu_units(1),
    );
    map.insert(
        TaskClass::new("holder"),
        ClassPolicy::new().max_inflight(1).cpu_units(units),
    );
    let g = Arc::new(
        Governor::new(
            PolicySet::new(ResourceBudget::new().cpu_units(units), map),
            Arc::new(ManualClock::new(0)),
        )
        .expect("valid policy"),
    );
    let holder = admit_one(&g, "holder", "holder");
    let waker = Arc::new(CrossClassSubmittingWaker {
        governor: Arc::clone(&g),
        newcomer_class: "b",
        outcome: std::sync::Mutex::new(None),
    });
    let port: Arc<dyn taskmesh_contract::PermitWaker> = waker.clone();
    let AdmissionDecision::Queued { ticket: first } = g.admit_waitable(&spec("a", "a0"), port)
    else {
        panic!("queues behind the holder");
    };
    let mut tickets = vec![first];
    for i in 1..queued_total {
        tickets.push(queue_one(&g, "a", &format!("a{i}")));
    }

    assert_eq!(g.release(holder), ReleaseOutcome::Released);

    let (decision, blocked_on) = waker
        .outcome
        .lock()
        .expect("lock")
        .take()
        .expect("woken by the first pass");
    let AdmissionDecision::Admitted {
        permit_id: newcomer,
    } = decision
    else {
        panic!(
            "an unqueueable newcomer must be admitted in the gap, not routed to a queue it does not have: got {decision:?}"
        );
    };
    assert_eq!(blocked_on, None, "an admitted request was not blocked");

    // Every earlier `a` arrival is still promoted by the continuation; the
    // newcomer took the one spare unit, so the tail waits for its release.
    assert_eq!(g.release(newcomer), ReleaseOutcome::Released);
    for ticket in &tickets {
        let ClaimOutcome::Ready(permit) = g.claim(*ticket) else {
            panic!("every earlier `a` arrival is promoted once the budget allows");
        };
        assert_eq!(g.release(permit), ReleaseOutcome::Released);
    }
    assert_eq!(g.snapshot().conservation_violation(), None);
}

#[test]
fn no_continuation_is_owed_when_a_pass_ends_exactly_at_its_budget() {
    // A pass that grants exactly `PROMOTION_BUDGET` permits has spent its
    // budget, and only then asks whether anything runnable remains. With the
    // queues drained the honest answer is no — and that answer is what the
    // woken waiters see. A queueable newcomer submitted from the wake must be
    // admitted directly: nothing is pending, so there is nothing to queue
    // behind. An oracle that always claims more work would queue it behind an
    // empty ring, waiting for a continuation pass with nothing to do.
    let budget = taskmesh_engine::PROMOTION_BUDGET;
    let units = u32::try_from(budget + 1).expect("small"); // the pass + the newcomer
    let mut map: BTreeMap<TaskClass, ClassPolicy> = BTreeMap::new();
    for name in ["a", "b"] {
        map.insert(
            TaskClass::new(name),
            ClassPolicy::new()
                .max_inflight(1_000)
                .max_queue_depth(1_000)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth)
                .fairness(FairnessPolicy::Fifo),
        );
    }
    map.insert(
        TaskClass::new("holder"),
        ClassPolicy::new().max_inflight(1).cpu_units(units),
    );
    let g = Arc::new(
        Governor::new(
            PolicySet::new(ResourceBudget::new().cpu_units(units), map),
            Arc::new(ManualClock::new(0)),
        )
        .expect("valid policy"),
    );
    let holder = admit_one(&g, "holder", "holder");
    let waker = Arc::new(CrossClassSubmittingWaker {
        governor: Arc::clone(&g),
        newcomer_class: "b",
        outcome: std::sync::Mutex::new(None),
    });
    let port: Arc<dyn taskmesh_contract::PermitWaker> = waker.clone();
    let AdmissionDecision::Queued { ticket: first } = g.admit_waitable(&spec("a", "a0"), port)
    else {
        panic!("queues behind the holder");
    };
    let mut tickets = vec![first];
    for i in 1..budget {
        tickets.push(queue_one(&g, "a", &format!("a{i}")));
    }
    assert_eq!(
        tickets.len(),
        budget,
        "exactly one budget's worth is queued"
    );

    assert_eq!(g.release(holder), ReleaseOutcome::Released);

    let (decision, blocked_on) = waker
        .outcome
        .lock()
        .expect("lock")
        .take()
        .expect("woken by the pass");
    let AdmissionDecision::Admitted {
        permit_id: newcomer,
    } = decision
    else {
        panic!(
            "no continuation is owed at exactly the budget: a queueable newcomer must be admitted directly, got {decision:?}"
        );
    };
    assert_eq!(blocked_on, None);
    for ticket in &tickets {
        let ClaimOutcome::Ready(permit) = g.claim(*ticket) else {
            panic!("every queued `a` was granted by the one pass");
        };
        assert_eq!(g.release(permit), ReleaseOutcome::Released);
    }
    assert_eq!(g.release(newcomer), ReleaseOutcome::Released);
    assert_eq!(g.snapshot().conservation_violation(), None);
}

#[test]
fn a_reject_class_that_queues_on_memory_waits_for_the_pending_continuation_too() {
    // Rule 2's second half: `class_can_queue` is true for a `Reject`-overflow
    // class whose memory policy is `Queue` — it has a queue, so in the gap it
    // waits for the continuation like any queueable class (its own queue is
    // empty here, so rule 1 does not apply). Dropping the memory clause would
    // let it overtake the runnable `a` heads.
    let budget = taskmesh_engine::PROMOTION_BUDGET;
    let queued_total = budget + 8;
    let units = u32::try_from(queued_total + 1).expect("small");
    let mut map: BTreeMap<TaskClass, ClassPolicy> = BTreeMap::new();
    map.insert(
        TaskClass::new("a"),
        ClassPolicy::new()
            .max_inflight(1_000)
            .max_queue_depth(1_000)
            .cpu_units(1)
            .overflow_policy(OverflowPolicy::QueueWithinDepth)
            .fairness(FairnessPolicy::Fifo),
    );
    map.insert(
        TaskClass::new("c"),
        ClassPolicy::new()
            .max_inflight(1_000)
            .max_queue_depth(1_000)
            .cpu_units(1)
            .memory_units(1)
            .overflow_policy(OverflowPolicy::Reject)
            .memory_overcommit_policy(taskmesh_contract::MemoryOvercommitPolicy::Queue)
            .fairness(FairnessPolicy::Fifo),
    );
    map.insert(
        TaskClass::new("holder"),
        ClassPolicy::new().max_inflight(1).cpu_units(units),
    );
    let g = Arc::new(
        Governor::new(
            PolicySet::new(
                ResourceBudget::new().cpu_units(units).memory_units(1_000),
                map,
            ),
            Arc::new(ManualClock::new(0)),
        )
        .expect("valid policy"),
    );
    let holder = admit_one(&g, "holder", "holder");
    let waker = Arc::new(CrossClassSubmittingWaker {
        governor: Arc::clone(&g),
        newcomer_class: "c",
        outcome: std::sync::Mutex::new(None),
    });
    let port: Arc<dyn taskmesh_contract::PermitWaker> = waker.clone();
    let AdmissionDecision::Queued { ticket: first } = g.admit_waitable(&spec("a", "a0"), port)
    else {
        panic!("queues behind the holder");
    };
    let mut tickets = vec![first];
    for i in 1..queued_total {
        tickets.push(queue_one(&g, "a", &format!("a{i}")));
    }
    assert_eq!(g.release(holder), ReleaseOutcome::Released);
    let (decision, blocked_on) = waker
        .outcome
        .lock()
        .expect("lock")
        .take()
        .expect("woken by the first pass");
    let AdmissionDecision::Queued { ticket: newcomer } = decision else {
        panic!("a Reject class that queues on memory can queue, so it waits for the pending pass; got {decision:?}");
    };
    assert_eq!(
        blocked_on,
        Some(taskmesh_engine::CapacityBlock::QueuedBehind)
    );
    let mut permits: Vec<PermitId> = Vec::new();
    for ticket in &tickets {
        let ClaimOutcome::Ready(permit) = g.claim(*ticket) else {
            panic!("promoted by the continuation");
        };
        permits.push(permit);
    }
    let ClaimOutcome::Ready(last) = g.claim(newcomer) else {
        panic!("promoted after the earlier arrivals");
    };
    assert!(permits.iter().all(|p| *p < last));
    for permit in permits.into_iter().chain(std::iter::once(last)) {
        assert_eq!(g.release(permit), ReleaseOutcome::Released);
    }
    assert_eq!(g.snapshot().conservation_violation(), None);
}

#[test]
fn a_reject_class_that_queues_on_memory_joins_its_queue_behind_a_runnable_head() {
    // `overflow_policy = Reject` but `memory_overcommit_policy = Queue`: the
    // class *does* have a queue (memory-blocked requests wait in it). A
    // newcomer arriving in the continuation gap must join that queue — not be
    // shed with a `CpuSaturated` verdict while cpu and memory are both free.
    let budget = taskmesh_engine::PROMOTION_BUDGET;
    let queued_total = budget + 8;
    let mem = u32::try_from(queued_total + 1).expect("small");
    let mut map: BTreeMap<TaskClass, ClassPolicy> = BTreeMap::new();
    map.insert(
        TaskClass::new("c"),
        ClassPolicy::new()
            .max_inflight(1_000)
            .max_queue_depth(1_000)
            .cpu_units(0)
            .memory_units(1)
            .overflow_policy(OverflowPolicy::Reject)
            .memory_overcommit_policy(taskmesh_contract::MemoryOvercommitPolicy::Queue)
            .fairness(FairnessPolicy::Fifo),
    );
    map.insert(
        TaskClass::new("holder"),
        ClassPolicy::new()
            .max_inflight(1)
            .cpu_units(0)
            .memory_units(mem),
    );
    let g = Arc::new(
        Governor::new(
            PolicySet::new(
                ResourceBudget::new().cpu_units(1_000).memory_units(mem),
                map,
            ),
            Arc::new(ManualClock::new(0)),
        )
        .expect("valid policy"),
    );
    let holder = admit_one(&g, "holder", "holder");
    let waker = Arc::new(CrossClassSubmittingWaker {
        governor: Arc::clone(&g),
        newcomer_class: "c",
        outcome: std::sync::Mutex::new(None),
    });
    let port: Arc<dyn taskmesh_contract::PermitWaker> = waker.clone();
    let AdmissionDecision::Queued { ticket: first } = g.admit_waitable(&spec("c", "c0"), port)
    else {
        panic!("memory-blocked requests of this class queue");
    };
    assert_eq!(
        g.pending_block_reason(first),
        Some(taskmesh_engine::CapacityBlock::Memory)
    );
    let mut tickets = vec![first];
    for i in 1..queued_total {
        tickets.push(queue_one(&g, "c", &format!("c{i}")));
    }
    assert_eq!(g.release(holder), ReleaseOutcome::Released);

    let (decision, blocked_on) = waker
        .outcome
        .lock()
        .expect("lock")
        .take()
        .expect("woken by the first pass");
    let AdmissionDecision::Queued { ticket: newcomer } = decision else {
        panic!("the class has a queue, so the newcomer joins it; got {decision:?}");
    };
    assert_eq!(
        blocked_on,
        Some(taskmesh_engine::CapacityBlock::QueuedBehind)
    );
    let mut permits: Vec<PermitId> = Vec::new();
    for ticket in &tickets {
        let ClaimOutcome::Ready(permit) = g.claim(*ticket) else {
            panic!("promoted by the continuation");
        };
        permits.push(permit);
    }
    let ClaimOutcome::Ready(last) = g.claim(newcomer) else {
        panic!("promoted last");
    };
    assert!(permits.iter().all(|p| *p < last));
    for permit in permits.into_iter().chain(std::iter::once(last)) {
        assert_eq!(g.release(permit), ReleaseOutcome::Released);
    }
    assert_eq!(g.snapshot().conservation_violation(), None);
}

/// A waker that submits a same-class newcomer on a *different* capability pool
/// when woken, recording the decision.
struct PoolSubmittingWaker {
    governor: Arc<Governor>,
    class: &'static str,
    pool: &'static str,
    outcome: std::sync::Mutex<Option<AdmissionDecision>>,
}

impl taskmesh_contract::PermitWaker for PoolSubmittingWaker {
    fn wake(&self) {
        let mut slot = self.outcome.lock().expect("lock");
        if slot.is_some() {
            return;
        }
        *slot = Some(self.governor.admit_resolved(
            &spec(self.class, "newcomer"),
            Some(self.pool),
            None,
        ));
    }
}

#[test]
fn the_capability_exception_holds_inside_the_continuation_gap_too() {
    // The pending-continuation rule queues gap arrivals so the pass can order
    // them — but a same-class newcomer whose head is blocked on *another* pool
    // must still be admitted directly, even mid-gap. Queued behind that head it
    // would inherit the head's pool saturation (in-class FIFO never promotes
    // past a blocked head), which is exactly the head-of-line loss the
    // exception prevents. `a` supplies the pending continuation; `c` has a
    // large-stack head stuck behind a held slot; the newcomer needs `blocking`.
    let budget = taskmesh_engine::PROMOTION_BUDGET;
    let queued_total = budget + 4;
    // Budget: the held large-stack slot (1) + the holder, exactly full; once the
    // holder goes there is room for every `a` and the newcomer.
    let units = u32::try_from(queued_total + 2).expect("small");
    let mut map: BTreeMap<TaskClass, ClassPolicy> = BTreeMap::new();
    for name in ["a", "c"] {
        map.insert(
            TaskClass::new(name),
            ClassPolicy::new()
                .max_inflight(1_000)
                .max_queue_depth(1_000)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth)
                .fairness(FairnessPolicy::Fifo),
        );
    }
    map.insert(
        TaskClass::new("holder"),
        ClassPolicy::new().max_inflight(1).cpu_units(units - 1),
    );
    let g = Arc::new(
        Governor::new(
            PolicySet::new(ResourceBudget::new().cpu_units(units), map)
                .with_capability_limits(BTreeMap::from([
                    ("large_stack".to_string(), 1),
                    ("blocking".to_string(), 10),
                ]))
                .expect("registered pools"),
            Arc::new(ManualClock::new(0)),
        )
        .expect("valid policy"),
    );
    // Occupy the single large-stack slot, then queue a `c` head on it.
    let AdmissionDecision::Admitted { permit_id: slot } =
        g.admit_resolved(&spec("c", "slot"), Some("large_stack"), None)
    else {
        panic!("slot free");
    };
    let AdmissionDecision::Queued {
        ticket: blocked_head,
    } = g.admit_resolved(&spec("c", "blocked-head"), Some("large_stack"), None)
    else {
        panic!("second large-stack request waits for the slot");
    };
    // Fill the cpu budget so `a` arrivals queue, with a waker on the first.
    let holder = admit_one(&g, "holder", "holder");
    let waker = Arc::new(PoolSubmittingWaker {
        governor: Arc::clone(&g),
        class: "c",
        pool: "blocking",
        outcome: std::sync::Mutex::new(None),
    });
    let port: Arc<dyn taskmesh_contract::PermitWaker> = waker.clone();
    let AdmissionDecision::Queued { ticket: first } = g.admit_waitable(&spec("a", "a0"), port)
    else {
        panic!("queues behind the holder");
    };
    let mut tickets = vec![first];
    for i in 1..queued_total {
        tickets.push(queue_one(&g, "a", &format!("a{i}")));
    }
    // Release the holder: pass 1 grants 64 `a`s and wakes the first; the waker
    // submits the `c` newcomer while the continuation is pending.
    assert_eq!(g.release(holder), ReleaseOutcome::Released);
    let decision = waker.outcome.lock().expect("lock").take().expect("woken");
    let AdmissionDecision::Admitted {
        permit_id: newcomer,
    } = decision
    else {
        panic!("a newcomer on another pool must not be queued behind a capability-blocked head: {decision:?}");
    };
    // The blocked head is still waiting on its own pool, everything in `a`
    // drains, and the ledgers close.
    assert!(matches!(
        g.ticket_status(blocked_head),
        ClaimOutcome::Pending
    ));
    for ticket in &tickets {
        let ClaimOutcome::Ready(permit) = g.claim(*ticket) else {
            panic!("promoted by the continuation");
        };
        assert_eq!(g.release(permit), ReleaseOutcome::Released);
    }
    assert_eq!(g.release(newcomer), ReleaseOutcome::Released);
    assert_eq!(g.release(slot), ReleaseOutcome::Released);
    let ClaimOutcome::Ready(head_permit) = g.claim(blocked_head) else {
        panic!("the freed slot promotes the head");
    };
    assert_eq!(g.release(head_permit), ReleaseOutcome::Released);
    assert_eq!(g.snapshot().conservation_violation(), None);
}

#[test]
fn a_head_costing_many_quanta_is_served_without_walking_every_round() {
    // `quantum = 1`, `cost = u32::MAX`: the naive ring needs 4.3 billion visits,
    // under the state mutex. The arithmetic form needs one pass over the classes.
    // The reference is deliberately not run here — it is the thing being avoided.
    let mut map: BTreeMap<TaskClass, ClassPolicy> = BTreeMap::new();
    map.insert(
        TaskClass::new("expensive"),
        ClassPolicy::new()
            .max_inflight(4)
            .max_queue_depth(4)
            .cpu_units(u32::MAX)
            .overflow_policy(OverflowPolicy::QueueWithinDepth)
            .fairness(FairnessPolicy::DeficitRoundRobin { quantum: 1 }),
    );
    let g = Governor::new(
        PolicySet::new(ResourceBudget::new().cpu_units(u32::MAX), map),
        Arc::new(ManualClock::new(0)),
    )
    .expect("valid policy");

    let g = Arc::new(g);
    let holder = admit_one(&g, "expensive", "holder");
    let ticket = queue_one(&g, "expensive", "queued");

    // The release runs on its own thread and the test waits a bounded time for
    // it. A scheduler proportional to cost/quantum does not "take a while"
    // here — it takes hours — and a test that merely measures `elapsed` after
    // the call returns would hang with it instead of failing.
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let releaser = Arc::clone(&g);
    std::thread::spawn(move || {
        let started = Instant::now();
        let outcome = releaser.release(holder);
        // Expected send failure: the receiver gave up, and the harness is what
        // reports that, so a `Result` here carries no extra information.
        let _receiver_may_have_timed_out = done_tx.send((outcome, started.elapsed()));
    });
    let (outcome, elapsed) = done_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("promotion did not complete within 5s: selection is proportional to cost/quantum");
    assert_eq!(outcome, ReleaseOutcome::Released);
    assert!(
        elapsed < Duration::from_secs(5),
        "promotion took {elapsed:?}"
    );
    assert!(
        matches!(g.ticket_status(ticket), ClaimOutcome::Ready(_)),
        "the queued head must be promoted"
    );
    let ClaimOutcome::Ready(permit) = g.claim(ticket) else {
        panic!("promoted");
    };
    assert_eq!(g.release(permit), ReleaseOutcome::Released);
}

#[test]
fn drr_examines_each_runnable_class_once_per_selection() {
    // The work oracle for TM16-012, independent of wall-clock: the scheduler
    // reports how many ring positions it examined. With `quantum = 1` and a
    // head costing 10_000 quanta, walking the ring visit by visit examines
    // 10_000 positions; the arithmetic form examines one per runnable class.
    // (Costs are kept finite so a regressed scheduler *finishes* and is caught
    // by the count, rather than by a timeout.)
    let mut map: BTreeMap<TaskClass, ClassPolicy> = BTreeMap::new();
    for (name, cost) in [("expensive", 10_000u32), ("cheap", 1)] {
        map.insert(
            TaskClass::new(name),
            ClassPolicy::new()
                .max_inflight(4)
                .max_queue_depth(4)
                .cpu_units(cost)
                .overflow_policy(OverflowPolicy::QueueWithinDepth)
                .fairness(FairnessPolicy::DeficitRoundRobin { quantum: 1 }),
        );
    }
    let g = Governor::new(
        // Room for one expensive head *and* the cheap one once the holder goes.
        PolicySet::new(ResourceBudget::new().cpu_units(10_001), map),
        Arc::new(ManualClock::new(0)),
    )
    .expect("valid policy");

    let holder = admit_one(&g, "expensive", "holder");
    let expensive = queue_one(&g, "expensive", "queued-expensive");
    // A cheap request of a *different* class fits the one spare unit and has no
    // queued head of its own, so it is admitted directly (in-class FIFO does
    // not hold one class hostage to another). Return it: the pass under test
    // is the expensive promotion.
    let cheap = admit_one(&g, "cheap", "direct-cheap");

    let before = g.drr_ring_visits();
    assert_eq!(g.release(holder), ReleaseOutcome::Released);
    // One runnable class, served in one selection; the pass that finds the
    // queue empty afterwards examines nothing. The bound is O(runnable classes)
    // per selection — two allows a second selection, never a walk.
    let visits = g.drr_ring_visits() - before;
    assert!(
        visits <= 2,
        "selection examined {visits} ring positions; the bound is O(runnable classes), not O(cost/quantum)"
    );
    // The oracle must have counted the selection at all: a promotion that
    // examined no ring position is not a DRR selection, and an oracle stuck at
    // zero would satisfy any upper bound.
    assert!(
        visits >= 1,
        "the selection that promoted the expensive head examined no ring position; the oracle is not counting"
    );
    assert!(
        matches!(g.ticket_status(expensive), ClaimOutcome::Ready(_)),
        "the expensive head fits the freed budget and must be promoted"
    );
    let ClaimOutcome::Ready(permit) = g.claim(expensive) else {
        panic!("promoted");
    };
    assert_eq!(g.release(permit), ReleaseOutcome::Released);
    assert_eq!(g.release(cheap), ReleaseOutcome::Released);
}

#[test]
fn an_idle_class_does_not_bank_credit_across_busy_periods() {
    // The TM16-037 shape: `a` is served once per cycle and drains, so it would
    // bank one unused quantum every time. After twenty such cycles a fresh
    // backlog must still interleave — the current queues are identical, so the
    // history must not decide the order.
    for warmup in [0usize, 1, 20] {
        let g = drr_governor(&[("a", 2), ("b", 1)], 64);
        let mut holder = admit_one(&g, "b", "seed");

        for cycle in 0..warmup {
            let mut tickets = vec![
                (queue_one(&g, "a", &format!("a{cycle}")), "a".to_string()),
                (queue_one(&g, "b", &format!("b{cycle}")), "b".to_string()),
            ];
            // Serve both, leaving the second permit as the next cycle's holder.
            let (_, permit) = serve_next(&g, &mut tickets, holder);
            let (_, permit) = serve_next(&g, &mut tickets, permit);
            holder = permit;
            assert_eq!(
                g.snapshot().classes[&TaskClass::new("a")].queued,
                0,
                "cycle {cycle}: a drained"
            );
        }

        let mut tickets = Vec::new();
        for i in 0..10 {
            tickets.push((queue_one(&g, "a", &format!("A{i}")), "a".to_string()));
        }
        for i in 0..10 {
            tickets.push((queue_one(&g, "b", &format!("B{i}")), "b".to_string()));
        }

        let mut order = Vec::new();
        while !tickets.is_empty() {
            let (class, permit) = serve_next(&g, &mut tickets, holder);
            order.push(class);
            holder = permit;
        }
        assert_eq!(g.release(holder), ReleaseOutcome::Released);

        // `a` has quantum 2 and `b` quantum 1, so the shape is a,a,b repeating.
        // What must NOT happen is ten consecutive `a`s because of idle history.
        let leading_a = order.iter().take_while(|class| *class == "a").count();
        assert!(
            leading_a <= 2,
            "warmup {warmup}: idle history produced a burst of {leading_a} (order {order:?})"
        );
        // And the long-run split still reflects the quanta, not the history.
        let a_count = order.iter().filter(|class| *class == "a").count();
        assert_eq!(a_count, 10, "warmup {warmup}: every `a` request ran");
    }
}

#[test]
fn a_capacity_blocked_queue_keeps_its_credit() {
    // The reset is scoped to *draining*, not to being unrunnable. A class whose
    // queue is still occupied has not ended its busy period, so it must not be
    // punished for the fact that capacity was unavailable.
    let g = drr_governor(&[("a", 4), ("b", 1)], 64);
    let holder = admit_one(&g, "b", "holder");
    let mut tickets = vec![
        (queue_one(&g, "a", "a0"), "a".to_string()),
        (queue_one(&g, "a", "a1"), "a".to_string()),
        (queue_one(&g, "b", "b0"), "b".to_string()),
    ];
    // While the holder is live, nothing is runnable — `a` is blocked, not drained.
    assert_eq!(g.snapshot().classes[&TaskClass::new("a")].queued, 2);
    let (first, permit) = serve_next(&g, &mut tickets, holder);
    let (second, permit) = serve_next(&g, &mut tickets, permit);
    let (third, permit) = serve_next(&g, &mut tickets, permit);
    assert_eq!(g.release(permit), ReleaseOutcome::Released);
    assert_eq!(
        vec![first, second, third],
        vec!["a".to_string(), "a".to_string(), "b".to_string()],
        "a quantum-4 class serves both its queued requests before yielding"
    );
}

// ---- TM16-013: WFQ precision ------------------------------------------------

#[test]
fn weighted_share_survives_uniform_rescaling_of_every_weight() {
    // The same ratio, expressed at two scales, must produce the same order. At
    // the old fixed-point scale the large-weight increments both floored to
    // zero and the discipline degenerated to arrival order.
    let small = weighted_promotion_order(1, 2);
    let large = weighted_promotion_order(2_000_000, 4_000_000);
    assert_eq!(
        small, large,
        "weighted share must not depend on the absolute magnitude of the weights"
    );
    // The order itself is derivable, so assert it rather than a proxy. With
    // cost 1 and weights 1 and 2, the virtual finish tags are a = 1, 2, 3 and
    // b = 0.5, 1.0, 1.5; ties break on arrival, and `a` arrives first each round.
    assert_eq!(
        small,
        vec![
            "b".to_string(),
            "a".to_string(),
            "b".to_string(),
            "b".to_string(),
            "a".to_string(),
            "a".to_string(),
        ],
        "weighted finish-time order"
    );
    // Both classes offered three requests, so both are fully served; what the
    // weight buys is position. The heavier class takes two of the first three
    // slots.
    let heavy_early = small.iter().take(3).filter(|class| *class == "b").count();
    assert_eq!(
        heavy_early, 2,
        "the heavier class is served earlier: {small:?}"
    );
}

#[test]
fn extreme_weights_still_produce_a_non_zero_increment() {
    // Any accepted weight must move the class's virtual finish tag. With a
    // scale that floors the heavy class's increment to zero, its tags never
    // advance and it is served for as long as it has work (b,b,b,a,a,a) — or,
    // if both floor to zero, the tie-break makes the order pure arrival
    // (a,b,a,b,a,b). Neither is proportional service, and "a was served at
    // some point" is true of both, so the exact order is what is asserted.
    //
    // 1 : u32::MAX — b's increment is 2^64 / u32::MAX ≈ 2^32, a's is 2^64.
    // b runs three times before a's first tag is reached: b,b,b,a,a,a.
    assert_eq!(
        weighted_promotion_order(1, u32::MAX),
        vec!["b", "b", "b", "a", "a", "a"],
        "a ~4-billion-to-one weight must serve the heavy class first, then the light one"
    );
    // (u32::MAX - 1) / 2 : u32::MAX - 1 — an exact 1:2 ratio at the top of the
    // domain must give the same order as 1:2 at the bottom of it
    // (scale-invariance; the tag tie between a's first and b's second request
    // resolves by arrival in both).
    assert_eq!(
        weighted_promotion_order((u32::MAX - 1) / 2, u32::MAX - 1),
        weighted_promotion_order(1, 2),
        "the top of the weight domain must schedule like the bottom"
    );
    // And 1:2 itself is genuinely proportional, not FIFO: b gets two of the
    // first three slots.
    let order = weighted_promotion_order(1, 2);
    assert_eq!(
        order.iter().take(3).filter(|c| *c == "b").count(),
        2,
        "1:2 must serve b twice in the first three: {order:?}"
    );
}

/// Queue three requests per class under contention and report the service order.
fn weighted_promotion_order(weight_a: u32, weight_b: u32) -> Vec<String> {
    let g = contended(vec![
        ("a", weighted_class(weight_a)),
        ("b", weighted_class(weight_b)),
    ]);
    let filler = admit_filler(&g);
    let mut tickets = Vec::new();
    for i in 0..3 {
        tickets.push(queue(&g, "a", &format!("a{i}")));
        tickets.push(queue(&g, "b", &format!("b{i}")));
    }
    assert_eq!(g.release(filler), ReleaseOutcome::Released);
    drain(&g, &tickets)
}

// ---- TM16-014: cancellation leaves no service debt --------------------------

#[test]
fn cancelled_requests_leave_no_virtual_time_debt() {
    // Ten enqueue/abandon cycles on `a`, none of which ran. Then `a` and `b`
    // arrive with equal weight, `a` first. `a` must go first: it was never
    // served, so it owes nothing.
    let g = contended(vec![("a", weighted_class(1)), ("b", weighted_class(1))]);
    let filler = admit_filler(&g);
    for i in 0..10 {
        let (ticket, _) = queue(&g, "a", &format!("cancelled{i}"));
        g.abandon(ticket);
    }
    let tickets = vec![queue(&g, "a", "real-a"), queue(&g, "b", "real-b")];
    assert_eq!(g.release(filler), ReleaseOutcome::Released);
    let order = drain(&g, &tickets);
    assert_eq!(
        order,
        vec!["a".to_string(), "b".to_string()],
        "an abandoned request never consumed service, so it owes no debt"
    );
}

#[test]
fn cancelling_from_the_middle_preserves_the_order_of_the_survivors() {
    // Removal must not reorder what is left, whichever position it happens at.
    // The survivors are the same class, so class names cannot tell them apart:
    // the order is checked by *ticket*, against the arrival order minus the
    // victim.
    for victim in 0..4usize {
        let g = contended(vec![("a", weighted_class(1)), ("b", weighted_class(1))]);
        let filler = admit_filler(&g);
        let queued: Vec<u64> = (0..4).map(|i| queue(&g, "a", &format!("a{i}")).0).collect();
        g.abandon(queued[victim]);
        let expected: Vec<u64> = queued
            .iter()
            .copied()
            .enumerate()
            .filter(|(i, _)| *i != victim)
            .map(|(_, t)| t)
            .collect();
        let mut remaining = expected.clone();
        let mut served: Vec<u64> = Vec::new();
        let mut holder = filler;
        while !remaining.is_empty() {
            assert_eq!(g.release(holder), ReleaseOutcome::Released);
            let ready: Vec<u64> = remaining
                .iter()
                .copied()
                .filter(|t| matches!(g.ticket_status(*t), ClaimOutcome::Ready(_)))
                .collect();
            assert_eq!(
                ready.len(),
                1,
                "victim {victim}: one unit frees one request"
            );
            let ticket = ready[0];
            let ClaimOutcome::Ready(permit) = g.claim(ticket) else {
                panic!("promoted a moment ago");
            };
            remaining.retain(|t| *t != ticket);
            served.push(ticket);
            holder = permit;
        }
        assert_eq!(g.release(holder), ReleaseOutcome::Released);
        assert_eq!(
            served, expected,
            "victim {victim}: survivors must run in arrival order"
        );
    }
}
