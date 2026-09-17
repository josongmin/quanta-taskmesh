//! Differential (executable-specification) property test of the admission /
//! promotion state machine.
//!
//! `prop_invariants.rs` checks *invariants* (accounting closes, caps hold).
//! This file checks *verdicts*: a reference model small enough to read in a
//! minute is driven with the same random operation sequence as the production
//! `Governor`, and every verdict and every observable — inflight, queued, cpu
//! held, pool occupancy, the state and block reason of every ticket ever issued,
//! the set of live permits, the retained terminal records — must agree after
//! every step.
//!
//! The model is the documented contract, not a copy of the engine. It has no
//! promotion budget, no lock gap, no fairness tags, no scheduler: it says what
//! must be *true* after each operation and lets the test say whether the
//! engine's machinery delivers it. Where D08 carves out behaviour, the model
//! states the promised outcome:
//!  * a capacity-freeing transition promotes until no queued head fits, FIFO by
//!    arrival across classes and strict FIFO inside one — so after any
//!    operation returns, no queued head fits (`QueuedBehind` is a lock-gap
//!    phenomenon a single-threaded sequence without a re-entrant waker never
//!    observes — the waker-driven and shuttle tests own it; a block reason the
//!    engine reports here must be one of the three limits);
//!  * a newcomer that fits is admitted even when its class has a queued head,
//!    because that head can only be waiting on a pool the newcomer does not
//!    need — the one D08 overtake;
//!  * a permit that dies unclaimed leaves its ticket a `Terminal` answer;
//!    abandoning a ticket, queued or promoted, leaves nothing.
//!
//! Restricted configuration: one to three FIFO classes, `max_inflight`, an
//! overflow policy (queue within depth, or reject), a per-class cpu cost, an
//! optional global cpu budget, two capability pools with optional limits.
//! Deliberately outside the model (covered elsewhere or not at all): memory
//! modes and reconcile, DRR/WFQ/deadline/best-effort tiers, wakers and the
//! wake path, child scope and the recursion guard, stale-lease reaping (only
//! the no-op sweep is exercised), retry-after policies (always `None`), and
//! the promotion budget boundary (sequences queue at most a dozen requests).
//! The verdict table the model encodes (which limit is reported when several
//! apply; what a full queue sheds with) is the one in
//! `docs/taskmesh-library-spec.md` §"Admission verdicts".

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use proptest::collection::vec;
use proptest::prelude::*;
use taskmesh_contract::{
    AdmissionVerdict, ClassPolicy, FairnessPolicy, ManualClock, MemoryReleasePolicy,
    OverflowPolicy, ResourceBudget, TaskClass, TaskSpec,
};
use taskmesh_engine::{
    AdmissionDecision, CapacityBlock, ClaimOutcome, Governor, LeakSweepReport, PermitId, PolicySet,
    ReleaseOutcome, TerminalReason, Ticket,
};

// ---- the reference model ----------------------------------------------------

/// Model-side identities. They are the model's own counters; the test keeps
/// the bijection to the engine's ids, so the model never has to predict how the
/// engine allocates them.
type ModelPermit = u64;
type ModelTicket = u64;

const CLASS_NAMES: [&str; 3] = ["a", "b", "c"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pool {
    None,
    Blocking,
    LargeStack,
}

impl Pool {
    fn name(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Blocking => Some("blocking"),
            Self::LargeStack => Some("large_stack"),
        }
    }
}

#[derive(Debug, Clone)]
struct ClassSpec {
    max_inflight: u32,
    cost: u32,
    /// `Some(depth)` queues within depth; `None` rejects on any block.
    queue_depth: Option<u32>,
}

#[derive(Debug, Clone)]
struct Config {
    classes: Vec<ClassSpec>,
    /// `0` is unlimited, as in `ResourceBudget`.
    cpu_budget: u32,
    /// `0` is ungated, as in `PolicySet::with_capability_limits`.
    blocking_limit: u32,
    large_stack_limit: u32,
}

/// The limit a request runs into. The order is the documented verdict
/// precedence (library spec, "Admission verdicts"): class inflight, then the
/// capability pool, then the cpu budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Limit {
    Inflight,
    Pool,
    Cpu,
}

impl Limit {
    fn block(self) -> CapacityBlock {
        match self {
            Self::Inflight => CapacityBlock::Inflight,
            Self::Pool => CapacityBlock::Capability,
            Self::Cpu => CapacityBlock::Cpu,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TicketState {
    Queued(Limit),
    Ready(ModelPermit),
    Terminal(TerminalReason),
}

#[derive(Debug, Clone)]
struct QueuedRequest {
    ticket: ModelTicket,
    arrival: u64,
    pool: Pool,
}

#[derive(Debug, Clone)]
struct LivePermit {
    class: usize,
    pool: Pool,
    /// The ticket this permit was promoted for, until it is claimed.
    unclaimed: Option<ModelTicket>,
}

/// Same shape as the engine's `AdmissionDecision`, in the model's id space.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Verdict {
    Admitted(ModelPermit),
    Queued(ModelTicket),
    Rejected(AdmissionVerdict),
}

struct Model {
    config: Config,
    queues: Vec<VecDeque<QueuedRequest>>,
    live: BTreeMap<ModelPermit, LivePermit>,
    tickets: BTreeMap<ModelTicket, TicketState>,
    next_arrival: u64,
    next_ticket: ModelTicket,
    next_permit: ModelPermit,
}

impl Model {
    fn new(config: Config) -> Self {
        let queues = config.classes.iter().map(|_| VecDeque::new()).collect();
        Self {
            config,
            queues,
            live: BTreeMap::new(),
            tickets: BTreeMap::new(),
            next_arrival: 0,
            next_ticket: 1,
            next_permit: 1,
        }
    }

    fn inflight(&self, class: usize) -> u32 {
        u32::try_from(self.live.values().filter(|p| p.class == class).count()).expect("small")
    }

    fn cpu_held(&self, class: usize) -> u32 {
        self.inflight(class) * self.config.classes[class].cost
    }

    fn cpu_held_total(&self) -> u32 {
        (0..self.config.classes.len())
            .map(|class| self.cpu_held(class))
            .sum()
    }

    fn in_use(&self, pool: Pool) -> u32 {
        u32::try_from(self.live.values().filter(|p| p.pool == pool).count()).expect("small")
    }

    fn limit(&self, pool: Pool) -> u32 {
        match pool {
            Pool::None => 0,
            Pool::Blocking => self.config.blocking_limit,
            Pool::LargeStack => self.config.large_stack_limit,
        }
    }

    /// The first limit a request of `class` on `pool` would exceed right now.
    fn exceeded(&self, class: usize, pool: Pool) -> Option<Limit> {
        let spec = &self.config.classes[class];
        if self.inflight(class) >= spec.max_inflight {
            return Some(Limit::Inflight);
        }
        let limit = self.limit(pool);
        if limit != 0 && self.in_use(pool) >= limit {
            return Some(Limit::Pool);
        }
        if self.config.cpu_budget != 0 && self.cpu_held_total() + spec.cost > self.config.cpu_budget
        {
            return Some(Limit::Cpu);
        }
        None
    }

    fn grant(&mut self, class: usize, pool: Pool, unclaimed: Option<ModelTicket>) -> ModelPermit {
        let permit = self.next_permit;
        self.next_permit += 1;
        self.live.insert(
            permit,
            LivePermit {
                class,
                pool,
                unclaimed,
            },
        );
        if let Some(ticket) = unclaimed {
            self.tickets.insert(ticket, TicketState::Ready(permit));
        }
        permit
    }

    /// Fits → admitted. Blocked → queue within depth, or shed with the verdict
    /// for the limit: a full pool is `SubstrateSaturated`; a full class or
    /// budget is `CpuSaturated` when the class cannot queue and `QueueFull`
    /// when its queue is full.
    fn admit(&mut self, class: usize, pool: Pool) -> Verdict {
        let arrival = self.next_arrival;
        self.next_arrival += 1;
        let Some(limit) = self.exceeded(class, pool) else {
            return Verdict::Admitted(self.grant(class, pool, None));
        };
        let retry_after_ms = None;
        match self.config.classes[class].queue_depth {
            Some(depth) if self.queues[class].len() < usize::try_from(depth).expect("small") => {
                let ticket = self.next_ticket;
                self.next_ticket += 1;
                self.queues[class].push_back(QueuedRequest {
                    ticket,
                    arrival,
                    pool,
                });
                self.tickets.insert(ticket, TicketState::Queued(limit));
                Verdict::Queued(ticket)
            }
            Some(_) => Verdict::Rejected(match limit {
                Limit::Pool => AdmissionVerdict::SubstrateSaturated { retry_after_ms },
                Limit::Inflight | Limit::Cpu => AdmissionVerdict::QueueFull { retry_after_ms },
            }),
            None => Verdict::Rejected(match limit {
                Limit::Pool => AdmissionVerdict::SubstrateSaturated { retry_after_ms },
                Limit::Inflight | Limit::Cpu => AdmissionVerdict::CpuSaturated { retry_after_ms },
            }),
        }
    }

    /// `Ready` transfers the permit and consumes the ticket; `Terminal` is
    /// reported once and then forgotten; anything unknown is `Invalid`.
    fn claim(&mut self, ticket: ModelTicket) -> ClaimOutcome {
        match self.tickets.get(&ticket).copied() {
            Some(TicketState::Queued(_)) => ClaimOutcome::Pending,
            Some(TicketState::Ready(permit)) => {
                self.tickets.remove(&ticket);
                self.live
                    .get_mut(&permit)
                    .expect("ready implies live")
                    .unclaimed = None;
                ClaimOutcome::Ready(permit)
            }
            Some(TicketState::Terminal(reason)) => {
                self.tickets.remove(&ticket);
                ClaimOutcome::Terminal(reason)
            }
            None => ClaimOutcome::Invalid,
        }
    }

    /// The waiter gives up: a queued request leaves the queue, a promoted one
    /// gives its permit back. Either way the ticket is gone and whatever was
    /// freed is promoted.
    fn abandon(&mut self, ticket: ModelTicket) {
        match self.tickets.remove(&ticket) {
            Some(TicketState::Queued(_)) => {
                for queue in &mut self.queues {
                    queue.retain(|request| request.ticket != ticket);
                }
                self.promote();
            }
            Some(TicketState::Ready(permit)) => {
                self.live.remove(&permit);
                self.promote();
            }
            Some(TicketState::Terminal(_)) | None => {}
        }
    }

    /// A live permit is returned and queued work promoted; a permit released
    /// before its waiter claimed it leaves that waiter a `Released` answer.
    fn release(&mut self, permit: ModelPermit) -> ReleaseOutcome {
        let Some(live) = self.live.remove(&permit) else {
            return ReleaseOutcome::UnknownPermit;
        };
        if let Some(ticket) = live.unclaimed {
            self.tickets
                .insert(ticket, TicketState::Terminal(TerminalReason::Released));
        }
        self.promote();
        ReleaseOutcome::Released
    }

    /// Promote until no queued head fits: the earliest-arrived head that fits,
    /// every time. Heads only — in-class order is strict.
    fn promote(&mut self) {
        loop {
            let next = (0..self.queues.len())
                .filter_map(|class| {
                    let head = self.queues[class].front()?;
                    self.exceeded(class, head.pool)
                        .is_none()
                        .then_some((head.arrival, class))
                })
                .min();
            let Some((_, class)) = next else {
                return;
            };
            let head = self.queues[class].pop_front().expect("head exists");
            self.grant(class, head.pool, Some(head.ticket));
        }
    }

    fn queued(&self, class: usize) -> u32 {
        u32::try_from(self.queues[class].len()).expect("small")
    }

    fn terminal_count(&self) -> usize {
        self.tickets
            .values()
            .filter(|state| matches!(state, TicketState::Terminal(_)))
            .count()
    }

    /// The `index`-th ticket ever issued (wrapping), or `None` before the first.
    fn any_ticket(&self, index: usize) -> Option<ModelTicket> {
        let issued = self.next_ticket - 1;
        (issued > 0).then(|| 1 + u64::try_from(index).expect("small") % issued)
    }

    /// The `index`-th permit ever granted (wrapping), or `None` before the first.
    fn any_permit(&self, index: usize) -> Option<ModelPermit> {
        let granted = self.next_permit - 1;
        (granted > 0).then(|| 1 + u64::try_from(index).expect("small") % granted)
    }
}

// ---- driving both sides ------------------------------------------------------

#[derive(Debug, Clone)]
enum Op {
    Admit {
        class: usize,
        pool: Pool,
    },
    Claim(usize),
    Abandon(usize),
    Release(usize),
    /// A sweep with nothing stale (the clock never moves): it must find nothing
    /// and change nothing.
    Reap,
}

fn class_spec() -> impl Strategy<Value = ClassSpec> {
    (
        1u32..=3,
        1u32..=3,
        prop_oneof![Just(None), (1u32..=4).prop_map(Some)],
    )
        .prop_map(|(max_inflight, cost, queue_depth)| ClassSpec {
            max_inflight,
            cost,
            queue_depth,
        })
}

fn config() -> impl Strategy<Value = Config> {
    (
        vec(class_spec(), 1..=3),
        // Every cost fits the budget, so the policy always validates.
        prop_oneof![Just(0u32), 3u32..=8],
        0u32..=3,
        0u32..=2,
    )
        .prop_map(
            |(classes, cpu_budget, blocking_limit, large_stack_limit)| Config {
                classes,
                cpu_budget,
                blocking_limit,
                large_stack_limit,
            },
        )
}

fn pool() -> impl Strategy<Value = Pool> {
    prop_oneof![
        Just(Pool::None),
        Just(Pool::Blocking),
        Just(Pool::LargeStack)
    ]
}

fn op(classes: usize) -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (0..classes, pool()).prop_map(|(class, pool)| Op::Admit { class, pool }),
        2 => (0usize..64).prop_map(Op::Claim),
        1 => (0usize..64).prop_map(Op::Abandon),
        3 => (0usize..64).prop_map(Op::Release),
        1 => Just(Op::Reap),
    ]
}

fn scenario() -> impl Strategy<Value = (Config, Vec<Op>)> {
    config().prop_flat_map(|config| {
        let classes = config.classes.len();
        (Just(config), vec(op(classes), 1..=80))
    })
}

fn governor(config: &Config) -> Governor {
    let classes: BTreeMap<TaskClass, ClassPolicy> = config
        .classes
        .iter()
        .enumerate()
        .map(|(index, spec)| {
            let mut policy = ClassPolicy::new()
                .max_inflight(spec.max_inflight)
                .cpu_units(spec.cost)
                .fairness(FairnessPolicy::Fifo)
                // Leak-detecting so the sweep actually inspects every permit.
                .memory_release_policy(MemoryReleasePolicy::LeakDetecting);
            if let Some(depth) = spec.queue_depth {
                policy = policy
                    .max_queue_depth(depth)
                    .overflow_policy(OverflowPolicy::QueueWithinDepth);
            }
            (TaskClass::new(CLASS_NAMES[index]), policy)
        })
        .collect();
    let policy = PolicySet::new(ResourceBudget::new().cpu_units(config.cpu_budget), classes)
        .with_capability_limits(BTreeMap::from([
            ("blocking".to_string(), config.blocking_limit),
            ("large_stack".to_string(), config.large_stack_limit),
        ]))
        .expect("registered pools");
    Governor::new(policy, Arc::new(ManualClock::new(0))).expect("valid policy")
}

/// The bijection between the engine's ids and the model's, built as each id
/// first becomes observable.
#[derive(Default)]
struct Binding {
    permits: BTreeMap<PermitId, ModelPermit>,
    permits_rev: BTreeMap<ModelPermit, PermitId>,
    tickets: BTreeMap<ModelTicket, Ticket>,
}

impl Binding {
    fn bind_permit(
        &mut self,
        engine: PermitId,
        model: ModelPermit,
        step: usize,
    ) -> Result<(), TestCaseError> {
        match (self.permits.get(&engine), self.permits_rev.get(&model)) {
            (None, None) => {
                self.permits.insert(engine, model);
                self.permits_rev.insert(model, engine);
                Ok(())
            }
            (Some(bound), Some(bound_rev)) if *bound == model && *bound_rev == engine => Ok(()),
            (bound, bound_rev) => Err(TestCaseError::fail(format!(
                "step {step}: engine permit {engine} / model permit {model} conflict with an \
                 earlier binding (engine→{bound:?}, model→{bound_rev:?})"
            ))),
        }
    }

    fn engine_permit(&self, model: ModelPermit) -> PermitId {
        self.permits_rev[&model]
    }

    fn engine_ticket(&self, model: ModelTicket) -> Ticket {
        self.tickets[&model]
    }

    /// Translate an engine claim outcome into the model's id space.
    fn claim_in_model_space(
        &self,
        outcome: ClaimOutcome,
        step: usize,
    ) -> Result<ClaimOutcome, TestCaseError> {
        match outcome {
            ClaimOutcome::Ready(engine) => self.permits.get(&engine).copied().map_or_else(
                || {
                    Err(TestCaseError::fail(format!(
                        "step {step}: claim returned engine permit {engine} that was never observed"
                    )))
                },
                |model| Ok(ClaimOutcome::Ready(model)),
            ),
            other => Ok(other),
        }
    }
}

/// Every observable the contract exposes must agree, after every operation.
fn check_agreement(
    g: &Governor,
    model: &Model,
    binding: &mut Binding,
    step: usize,
) -> Result<(), TestCaseError> {
    let snapshot = g.snapshot();
    prop_assert_eq!(snapshot.conservation_violation(), None, "step {}", step);
    prop_assert_eq!(g.accounting_fault(), None, "step {}", step);
    for (class, name) in CLASS_NAMES
        .iter()
        .enumerate()
        .take(model.config.classes.len())
    {
        let observed = &snapshot.classes[&TaskClass::new(*name)];
        prop_assert_eq!(
            observed.inflight,
            model.inflight(class),
            "step {}: class {} inflight",
            step,
            name
        );
        prop_assert_eq!(
            observed.queued,
            model.queued(class),
            "step {}: class {} queued",
            step,
            name
        );
        prop_assert_eq!(
            observed.cpu_units_held,
            u128::from(model.cpu_held(class)),
            "step {}: class {} cpu held",
            step,
            name
        );
    }
    for pool in [Pool::Blocking, Pool::LargeStack] {
        let name = pool.name().expect("gated pool");
        let observed = snapshot
            .capabilities
            .get(name)
            .map_or(0, |usage| usage.in_use);
        prop_assert_eq!(
            observed,
            model.in_use(pool),
            "step {}: pool {} in use",
            step,
            name
        );
    }
    // Every ticket ever issued, consumed ones included: those must be `Invalid`.
    let issued: Vec<(ModelTicket, Ticket)> = binding
        .tickets
        .iter()
        .map(|(model_ticket, engine_ticket)| (*model_ticket, *engine_ticket))
        .collect();
    for (model_ticket, engine_ticket) in issued {
        let expected = model.tickets.get(&model_ticket).copied();
        let observed = g.ticket_status(engine_ticket);
        match (expected, observed) {
            (Some(TicketState::Queued(limit)), ClaimOutcome::Pending) => {
                prop_assert_eq!(
                    g.pending_block_reason(engine_ticket),
                    Some(limit.block()),
                    "step {}: ticket {} block reason",
                    step,
                    model_ticket
                );
            }
            (Some(TicketState::Ready(model_permit)), ClaimOutcome::Ready(engine_permit)) => {
                binding.bind_permit(engine_permit, model_permit, step)?;
            }
            (Some(TicketState::Terminal(expected)), ClaimOutcome::Terminal(observed)) => {
                prop_assert_eq!(
                    expected,
                    observed,
                    "step {}: ticket {} terminal reason",
                    step,
                    model_ticket
                );
            }
            (None, ClaimOutcome::Invalid) => {}
            (expected, observed) => prop_assert!(
                false,
                "step {}: ticket {}: model {:?}, engine {:?}",
                step,
                model_ticket,
                expected,
                observed
            ),
        }
    }
    prop_assert_eq!(
        g.permit_ledgers().len(),
        model.live.len(),
        "step {}: live permit count",
        step
    );
    for (model_permit, live) in &model.live {
        let engine_permit = binding.engine_permit(*model_permit);
        let Some(ledger) = g.permit_ledger(engine_permit) else {
            return Err(TestCaseError::fail(format!(
                "step {step}: model permit {model_permit} is live but engine permit {engine_permit} is not"
            )));
        };
        prop_assert_eq!(
            ledger.class,
            TaskClass::new(CLASS_NAMES[live.class]),
            "step {}: permit {} class",
            step,
            model_permit
        );
        prop_assert_eq!(
            ledger.capability.as_deref(),
            live.pool.name(),
            "step {}: permit {} pool",
            step,
            model_permit
        );
    }
    prop_assert_eq!(
        g.retained_terminal_tickets(),
        model.terminal_count(),
        "step {}: retained terminal tickets",
        step
    );
    Ok(())
}

fn abandon_both(g: &Governor, model: &mut Model, binding: &Binding, model_ticket: ModelTicket) {
    g.abandon(binding.engine_ticket(model_ticket));
    model.abandon(model_ticket);
}

fn release_both(
    g: &Governor,
    model: &mut Model,
    binding: &Binding,
    step: usize,
    model_permit: ModelPermit,
) -> Result<(), TestCaseError> {
    let engine = g.release(binding.engine_permit(model_permit));
    let expected = model.release(model_permit);
    prop_assert_eq!(
        engine,
        expected,
        "step {}: release of permit {}",
        step,
        model_permit
    );
    Ok(())
}

fn apply(
    g: &Governor,
    model: &mut Model,
    binding: &mut Binding,
    step: usize,
    op: &Op,
) -> Result<(), TestCaseError> {
    match *op {
        Op::Admit { class, pool } => {
            let spec =
                TaskSpec::io(TaskClass::new(CLASS_NAMES[class])).operation(format!("op{step}"));
            let engine = g.admit_resolved(&spec, pool.name(), None);
            let expected = model.admit(class, pool);
            match (&engine, &expected) {
                (AdmissionDecision::Admitted { permit_id }, Verdict::Admitted(model_permit)) => {
                    binding.bind_permit(*permit_id, *model_permit, step)?;
                }
                (AdmissionDecision::Queued { ticket }, Verdict::Queued(model_ticket)) => {
                    binding.tickets.insert(*model_ticket, *ticket);
                }
                (AdmissionDecision::Rejected(observed), Verdict::Rejected(expected)) => {
                    prop_assert_eq!(observed, expected, "step {}: rejection verdict", step);
                }
                (engine, expected) => prop_assert!(
                    false,
                    "step {}: admit {} on {:?}: engine {:?}, model {:?}",
                    step,
                    CLASS_NAMES[class],
                    pool,
                    engine,
                    expected
                ),
            }
        }
        Op::Claim(index) => {
            if let Some(model_ticket) = model.any_ticket(index) {
                let engine = g.claim(binding.engine_ticket(model_ticket));
                let expected = model.claim(model_ticket);
                prop_assert_eq!(
                    binding.claim_in_model_space(engine, step)?,
                    expected,
                    "step {}: claim of ticket {}",
                    step,
                    model_ticket
                );
            }
        }
        Op::Abandon(index) => {
            if let Some(model_ticket) = model.any_ticket(index) {
                abandon_both(g, model, binding, model_ticket);
            }
        }
        Op::Release(index) => {
            if let Some(model_permit) = model.any_permit(index) {
                release_both(g, model, binding, step, model_permit)?;
            }
        }
        Op::Reap => {
            prop_assert_eq!(
                g.reap_leaks_with(1_000),
                LeakSweepReport::default(),
                "step {}: a sweep with nothing stale finds nothing",
                step
            );
        }
    }
    check_agreement(g, model, binding, step)
}

proptest! {
    // 1,024 cases by default (`PROPTEST_CASES` still overrides). 256 was
    // marginal: a mutant that skips the promotion pass after abandoning a
    // queued ticket needs an abandon of a *blocked head with runnable work
    // behind it*, which 256 random sequences sometimes never generate. At
    // 1,024 that mutant was found in every one of ten runs.
    #![proptest_config(ProptestConfig {
        cases: 1_024,
        ..ProptestConfig::default()
    })]

    /// The production governor and the reference model agree on every verdict
    /// and every observable, after every operation of a random sequence.
    #[test]
    fn engine_agrees_with_the_reference_model((config, ops) in scenario()) {
        let g = governor(&config);
        let mut model = Model::new(config);
        let mut binding = Binding::default();
        for (step, op) in ops.iter().enumerate() {
            apply(&g, &mut model, &mut binding, step, op)?;
        }
        // Drain: release everything live (which promotes whatever fits), then
        // abandon whatever is still queued; both sides must end empty.
        let mut step = ops.len();
        while let Some(model_permit) = model.live.keys().next().copied() {
            release_both(&g, &mut model, &binding, step, model_permit)?;
            check_agreement(&g, &model, &mut binding, step)?;
            step += 1;
        }
        let queued: Vec<ModelTicket> = model
            .tickets
            .iter()
            .filter(|(_, state)| matches!(state, TicketState::Queued(_)))
            .map(|(ticket, _)| *ticket)
            .collect();
        for model_ticket in queued {
            abandon_both(&g, &mut model, &binding, model_ticket);
            check_agreement(&g, &model, &mut binding, step)?;
            step += 1;
        }
        prop_assert!(g.permit_ledgers().is_empty(), "engine drained");
        prop_assert_eq!(
            g.snapshot().classes.values().map(|c| c.queued).sum::<u32>(),
            0,
            "engine queues drained"
        );
    }
}
