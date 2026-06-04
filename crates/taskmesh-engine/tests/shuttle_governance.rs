//! Randomized concurrency model-check (ADR 9000 / P6).
//!
//! loom (`loom_governance.rs`) explores *every* interleaving but only for tiny
//! state (2 threads). shuttle complements it with a randomized scheduler that
//! scales to larger state — more threads and more ops — sampling thousands of
//! schedules. Same modeled design as loom: a Relaxed atomic id taken OUTSIDE the
//! lock, then committed under one global mutex (mirrors `Governor::fresh_ids` +
//! `GovernedState::grant`/`unwind`).
//!
//! Run with: `RUSTFLAGS="--cfg shuttle" cargo test -p taskmesh-engine --test shuttle_governance --release`.

#![cfg(shuttle)]

use std::collections::BTreeSet;

use shuttle::sync::atomic::{AtomicU64, Ordering};
use shuttle::sync::{Arc, Mutex};
use shuttle::thread;

#[derive(Default)]
struct State {
    inflight: i64,
    granted: BTreeSet<u64>,
}

fn admit(next_id: &AtomicU64, state: &Mutex<State>) -> u64 {
    let id = next_id.fetch_add(1, Ordering::Relaxed);
    let mut s = state.lock().unwrap();
    assert!(s.granted.insert(id), "permit id {id} double-granted");
    s.inflight += 1;
    id
}

fn release(state: &Mutex<State>, id: u64) {
    let mut s = state.lock().unwrap();
    assert!(s.granted.remove(&id), "release of unknown permit {id}");
    s.inflight -= 1;
}

#[test]
fn randomized_concurrent_admit_release_is_safe() {
    shuttle::check_random(
        || {
            let next_id = Arc::new(AtomicU64::new(1));
            let state = Arc::new(Mutex::new(State::default()));

            // 4 threads, each admitting and releasing twice — larger state than
            // loom can exhaust, sampled randomly.
            let handles: Vec<_> = (0..4)
                .map(|_| {
                    let next_id = next_id.clone();
                    let state = state.clone();
                    thread::spawn(move || {
                        for _ in 0..2 {
                            let id = admit(&next_id, &state);
                            release(&state, id);
                        }
                    })
                })
                .collect();
            for h in handles {
                h.join().unwrap();
            }

            let s = state.lock().unwrap();
            assert_eq!(s.inflight, 0, "inflight must return to zero");
            assert!(s.granted.is_empty(), "no permit may leak");
        },
        10_000,
    );
}

#[test]
fn randomized_overlapping_holders_are_consistent() {
    shuttle::check_random(
        || {
            let next_id = Arc::new(AtomicU64::new(1));
            let state = Arc::new(Mutex::new(State::default()));

            // One long holder overlaps several admit/release churners.
            let holder = {
                let next_id = next_id.clone();
                let state = state.clone();
                thread::spawn(move || {
                    let id = admit(&next_id, &state);
                    thread::yield_now();
                    release(&state, id);
                })
            };
            let churners: Vec<_> = (0..3)
                .map(|_| {
                    let next_id = next_id.clone();
                    let state = state.clone();
                    thread::spawn(move || {
                        let id = admit(&next_id, &state);
                        release(&state, id);
                    })
                })
                .collect();

            holder.join().unwrap();
            for c in churners {
                c.join().unwrap();
            }

            let s = state.lock().unwrap();
            assert_eq!(s.inflight, 0);
            assert!(s.granted.is_empty());
        },
        10_000,
    );
}

/// The host `TicketGuard` handoff under a randomized scheduler at higher
/// concurrency than loom can exhaust: a queued ticket is concurrently promoted
/// (on release), claimed (the waiter), and abandoned by *two* racers
/// (cancel + acquire-timeout firing together). Models `Governor::{promote,
/// claim, abandon}` plus the host "claimer owns and releases" rule. Across every
/// sampled schedule the promoted permit is accounted exactly once — no leak, no
/// double-free, no stranded promoted-but-unclaimed permit, and abandon stays
/// idempotent when two racers both fire.
#[derive(Default)]
struct TicketState {
    queued: bool,
    granted: bool,
    claimed: bool,
    inflight: i64,
}

#[test]
fn randomized_promote_claim_double_abandon_is_exactly_once() {
    shuttle::check_random(
        || {
            let st = Arc::new(Mutex::new(TicketState {
                queued: true,
                ..Default::default()
            }));

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
            let claimer = {
                let st = st.clone();
                thread::spawn(move || {
                    let mut s = st.lock().unwrap();
                    if s.granted && !s.claimed {
                        s.claimed = true;
                    }
                })
            };
            let abandon = |st: Arc<Mutex<TicketState>>| {
                thread::spawn(move || {
                    let mut s = st.lock().unwrap();
                    if s.claimed {
                        // owned by the claimer — abandon must not touch it
                    } else if s.granted {
                        s.granted = false;
                        s.inflight -= 1;
                    } else if s.queued {
                        s.queued = false;
                    }
                })
            };
            let abandoner1 = abandon(st.clone());
            let abandoner2 = abandon(st.clone());

            promoter.join().unwrap();
            claimer.join().unwrap();
            abandoner1.join().unwrap();
            abandoner2.join().unwrap();

            let mut s = st.lock().unwrap();
            if s.claimed {
                s.inflight -= 1; // claimer owns it and releases when done
            }
            assert_eq!(s.inflight, 0, "permit released exactly once");
            assert!(
                !(s.granted && !s.claimed),
                "no promoted-but-unclaimed permit may be stranded"
            );
        },
        20_000,
    );
}
