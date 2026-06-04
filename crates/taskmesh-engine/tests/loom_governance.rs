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

/// Models the promote-vs-abandon race the host `TicketGuard` protects: one
/// thread releases a permit (which *promotes* the single queued ticket → grants
/// it, unclaimed), while the waiting thread *abandons* the ticket (cancel /
/// acquire-timeout). However they interleave, the promoted permit must be
/// released exactly once — never leaked (stuck inflight) and never double-freed.
fn promote_on_release(state: &Mutex<QState>) {
    let mut s = state.lock().unwrap();
    if s.queued {
        s.queued = false;
        s.granted = true; // promoted, awaiting claim
        s.inflight += 1;
    }
}

fn abandon(state: &Mutex<QState>) {
    let mut s = state.lock().unwrap();
    if s.granted {
        // Promoted-but-unclaimed: abandoning releases it.
        s.granted = false;
        s.inflight -= 1;
    } else if s.queued {
        // Still queued: just drop it from the queue.
        s.queued = false;
    }
}

#[derive(Default)]
struct QState {
    inflight: i64,
    queued: bool,
    granted: bool,
}

#[test]
fn promote_and_abandon_never_leak_or_double_free() {
    loom::model(|| {
        let state = Arc::new(Mutex::new(QState {
            inflight: 0,
            queued: true,
            granted: false,
        }));

        let s1 = state.clone();
        let promoter = thread::spawn(move || promote_on_release(&s1));
        let s2 = state.clone();
        let abandoner = thread::spawn(move || abandon(&s2));
        promoter.join().unwrap();
        abandoner.join().unwrap();

        let s = state.lock().unwrap();
        assert_eq!(
            s.inflight, 0,
            "promoted permit must not leak or double-free"
        );
        assert!(!s.queued, "ticket must not remain queued");
        assert!(!s.granted, "no unclaimed promoted permit may remain");
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

/// The full three-way race the host's `TicketGuard` must survive: a queued
/// ticket is, concurrently, (1) promoted by a releasing thread, (2) claimed by
/// the waiting thread, and (3) abandoned (cancel / acquire-timeout). Models the
/// exact ownership handoff in `Governor::{release→promote, claim, abandon}` plus
/// the host's "claimer owns it and releases" rule. Across every interleaving the
/// promoted permit is accounted exactly once: never leaked (stuck inflight),
/// never double-freed, and never stranded promoted-but-unclaimed.
#[derive(Default)]
struct ThreeWay {
    queued: bool,
    granted: bool,
    claimed: bool,
    inflight: i64,
}

#[test]
fn promote_claim_abandon_three_way_is_exactly_once() {
    loom::model(|| {
        let st = Arc::new(Mutex::new(ThreeWay {
            queued: true,
            ..Default::default()
        }));

        // (1) releasing thread promotes the queued head into a granted permit.
        let promoter = {
            let st = st.clone();
            thread::spawn(move || {
                let mut s = st.lock().unwrap();
                if s.queued {
                    s.queued = false;
                    s.granted = true;
                    s.inflight += 1;
                }
            })
        };
        // (2) waiter claims the promoted permit (takes ownership).
        let claimer = {
            let st = st.clone();
            thread::spawn(move || {
                let mut s = st.lock().unwrap();
                if s.granted && !s.claimed {
                    s.claimed = true;
                }
            })
        };
        // (3) TicketGuard abandons: release if promoted-unclaimed, else dequeue.
        let abandoner = {
            let st = st.clone();
            thread::spawn(move || {
                let mut s = st.lock().unwrap();
                if s.claimed {
                    // already owned by the claimer — abandon must not touch it
                } else if s.granted {
                    s.granted = false;
                    s.inflight -= 1;
                } else if s.queued {
                    s.queued = false;
                }
            })
        };
        promoter.join().unwrap();
        claimer.join().unwrap();
        abandoner.join().unwrap();

        let mut s = st.lock().unwrap();
        // A claimer that won ownership releases its permit when done.
        if s.claimed {
            s.inflight -= 1;
        }
        assert_eq!(s.inflight, 0, "permit must be released exactly once");
        assert!(
            !(s.granted && !s.claimed),
            "no promoted-but-unclaimed permit may be stranded"
        );
    });
}
