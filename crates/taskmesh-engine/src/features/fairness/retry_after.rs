//! Retry-after hint computation (T04). Every constant is crate-private,
//! deterministic, and test-fixed — the same state always yields the same hint.

use taskmesh_contract::RetryAfterPolicy;

/// Adaptive base latency floor.
pub const ADAPTIVE_BASE_MS: u64 = 50;
/// Per-queued-item contribution to the adaptive hint.
pub const ADAPTIVE_QUEUE_STEP_MS: u64 = 10;
/// Per-inflight-permit contribution to the adaptive hint.
pub const ADAPTIVE_INFLIGHT_STEP_MS: u64 = 5;

/// Compute the retry-after hint for a rejection, given the class policy and the
/// current contention as observed at decision time.
pub fn compute(policy: RetryAfterPolicy, queue_depth: u32, inflight: u32) -> Option<u64> {
    match policy {
        RetryAfterPolicy::None => None,
        RetryAfterPolicy::FixedMs(value) => Some(value),
        RetryAfterPolicy::Adaptive => Some(
            ADAPTIVE_BASE_MS
                + (queue_depth as u64) * ADAPTIVE_QUEUE_STEP_MS
                + (inflight as u64) * ADAPTIVE_INFLIGHT_STEP_MS,
        ),
    }
}
