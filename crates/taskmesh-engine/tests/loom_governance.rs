//! Exhaustive concurrency model-check of the governance design (ADR 9000 / P6).
//!
//! The real `Governor` uses `parking_lot::Mutex` + `std` atomics, which loom
//! cannot intercept. So this faithfully models the *same* concurrency design
//! (ADR 0001 §4): one global mutex over the granted-permit state, plus a Relaxed
//! atomic permit-id generator taken OUTSIDE the lock — exactly the shape of
//! `Governor::fresh_ids()` (Relaxed `fetch_add` before locking) feeding
//! `GovernedState::grant`/`unwind` under the lock.
//!
//! loom explores every thread interleaving and checks the safety invariants the
//! `concurrency_stress.rs` OS-thread test can only sample: permit ids are unique
//! (no double-grant), inflight returns to zero, and no permit is lost.
//!
//! Run with: `RUSTFLAGS="--cfg loom" cargo test -p taskmesh-engine --test loom_governance`.

#![cfg(loom)]

use std::collections::BTreeSet;

use loom::sync::atomic::{AtomicU64, Ordering};
use loom::sync::{Arc, Mutex};
use loom::thread;

/// The state the real governor keeps behind its single mutex (subset that the
/// concurrency invariant depends on): how many permits are inflight and which
/// ids are currently granted.
#[derive(Default)]
struct State {
    inflight: i64,
    granted: BTreeSet<u64>,
}

/// Mirrors `Governor::fresh_ids()` + `grant()`: take a unique id with a Relaxed
/// atomic BEFORE acquiring the lock, then commit under the lock.
fn admit(next_id: &AtomicU64, state: &Mutex<State>) -> u64 {
    let id = next_id.fetch_add(1, Ordering::Relaxed);
    let mut s = state.lock().unwrap();
    assert!(s.granted.insert(id), "permit id {id} double-granted");
    s.inflight += 1;
    id
}

/// Mirrors `release` → `GovernedState::unwind`: drop the permit under the lock.
fn release(state: &Mutex<State>, id: u64) {
    let mut s = state.lock().unwrap();
    assert!(s.granted.remove(&id), "release of unknown permit {id}");
    s.inflight -= 1;
}

#[test]
fn concurrent_admit_release_preserves_safety() {
    loom::model(|| {
        let next_id = Arc::new(AtomicU64::new(1));
        let state = Arc::new(Mutex::new(State::default()));

        let handles: Vec<_> = (0..2)
            .map(|_| {
                let next_id = next_id.clone();
                let state = state.clone();
                thread::spawn(move || {
                    let id = admit(&next_id, &state);
                    release(&state, id);
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        let s = state.lock().unwrap();
        assert_eq!(s.inflight, 0, "inflight must return to zero");
        assert!(s.granted.is_empty(), "no permit may leak");
    });
}

#[test]
fn admit_then_concurrent_release_is_consistent() {
    // One thread holds a permit while another admits+releases — checks the
    // grant/unwind bookkeeping under overlap.
    loom::model(|| {
        let next_id = Arc::new(AtomicU64::new(1));
        let state = Arc::new(Mutex::new(State::default()));

        let id0 = admit(&next_id, &state);

        let other = {
            let next_id = next_id.clone();
            let state = state.clone();
            thread::spawn(move || {
                let id = admit(&next_id, &state);
                release(&state, id);
            })
        };
        release(&state, id0);
        other.join().unwrap();

        let s = state.lock().unwrap();
        assert_eq!(s.inflight, 0);
        assert!(s.granted.is_empty());
    });
}
