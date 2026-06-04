//! Retry-after hint computation (T04). Every constant is crate-private,
//! deterministic, and test-fixed — the same state always yields the same hint.
//!
//! The adaptive hint blends contention (queue depth + inflight) with the class's
//! own fairness discipline (T04): a heavier weighted-fair class is told to come
//! back sooner, a larger DRR quantum or a slacker deadline implies a longer
//! round, and a best-effort scavenger backs off hardest.

use taskmesh_contract::{FairnessPolicy, RetryAfterPolicy};

/// Adaptive base latency floor.
pub const ADAPTIVE_BASE_MS: u64 = 50;
/// Per-queued-item contribution to the adaptive hint.
pub const ADAPTIVE_QUEUE_STEP_MS: u64 = 10;
/// Per-inflight-permit contribution to the adaptive hint.
pub const ADAPTIVE_INFLIGHT_STEP_MS: u64 = 5;

// Fairness-discipline adjustments (deterministic, test-fixed).
const WEIGHT_RELIEF_MS: u64 = 2; // heavier weight -> sooner retry
const QUANTUM_STEP_MS: u64 = 1; // larger quantum -> longer round
const SLACK_DIVISOR: u64 = 10; // slacker deadline -> longer wait
const SCAVENGER_PENALTY_MS: u64 = 200; // best-effort backs off hardest

/// Compute the retry-after hint for a rejection, given the class's retry and
/// fairness policies and the contention observed at decision time.
pub fn compute(
    retry: RetryAfterPolicy,
    fairness: FairnessPolicy,
    queue_depth: u32,
    inflight: u32,
) -> Option<u64> {
    match retry {
        RetryAfterPolicy::None => None,
        RetryAfterPolicy::FixedMs(value) => Some(value),
        RetryAfterPolicy::Adaptive => {
            let contention = ADAPTIVE_BASE_MS
                + u64::from(queue_depth) * ADAPTIVE_QUEUE_STEP_MS
                + u64::from(inflight) * ADAPTIVE_INFLIGHT_STEP_MS;
            Some(apply_fairness(contention, fairness))
        }
    }
}

fn apply_fairness(base: u64, fairness: FairnessPolicy) -> u64 {
    match fairness {
        FairnessPolicy::Fifo => base,
        FairnessPolicy::WeightedFairQueue { weight, .. } => {
            base.saturating_sub(u64::from(weight).saturating_mul(WEIGHT_RELIEF_MS))
        }
        FairnessPolicy::DeficitRoundRobin { quantum } => {
            base + u64::from(quantum) * QUANTUM_STEP_MS
        }
        FairnessPolicy::DeadlineAware { slack_ms } => base + slack_ms / SLACK_DIVISOR,
        FairnessPolicy::BestEffortScavenger => base + SCAVENGER_PENALTY_MS,
    }
}
