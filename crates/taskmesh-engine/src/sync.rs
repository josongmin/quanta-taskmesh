//! The synchronization seam the concurrency model checkers swap in.
//!
//! Production compiles the governor against `parking_lot::Mutex` and `std`
//! atomics. Under `--cfg loom` or `--cfg shuttle` the **same** governor code
//! compiles against the checker's instrumented primitives, so
//! `tests/loom_governance.rs` and `tests/shuttle_governance.rs` explore the real
//! [`crate::Governor`] — its actual `admit`/`claim`/`abandon`/`release`/`reap`
//! transitions and the effects it drains outside the lock — rather than a
//! hand-written replica of its locking design. A replica proves only that the
//! replica is right.
//!
//! The seam is deliberately tiny: one mutex type with a single `lock()` and the
//! `AtomicU64` the id generators use. Nothing else in the engine synchronizes.
//! (The process-wide lease-nonce counter in `engine::state` is a `std` atomic
//! on purpose: no transition synchronizes on it — it only has to hand out
//! distinct values, which a relaxed `fetch_add` does under any memory model.)

#[cfg(all(loom, shuttle))]
compile_error!("`--cfg loom` and `--cfg shuttle` are mutually exclusive model-check builds");

// The cfg selects the seam; the feature brings the checker crate. One without
// the other is a half-configured build that would fail somewhere less obvious.
#[cfg(all(loom, not(feature = "loom")))]
compile_error!("`--cfg loom` requires `--features loom` (see `just loom`)");
#[cfg(all(shuttle, not(feature = "shuttle")))]
compile_error!("`--cfg shuttle` requires `--features shuttle` (see `just shuttle`)");
// And the reverse: the feature without the cfg would build the production seam
// while the `#![cfg(loom)]` model files compile to *zero tests* — a green rail
// that proved nothing. Refuse that shape too.
#[cfg(all(feature = "loom", not(loom)))]
compile_error!("`--features loom` requires `RUSTFLAGS=\"--cfg loom\"` (see `just loom`)");
#[cfg(all(feature = "shuttle", not(shuttle)))]
compile_error!("`--features shuttle` requires `RUSTFLAGS=\"--cfg shuttle\"` (see `just shuttle`)");

#[cfg(loom)]
mod imp {
    pub use loom::sync::atomic::{AtomicU64, Ordering};

    /// Loom's mutex, with its poison result collapsed: a poisoned guard means an
    /// earlier interleaving already panicked and the model has failed.
    pub struct Mutex<T>(loom::sync::Mutex<T>);

    impl<T> Mutex<T> {
        pub fn new(value: T) -> Self {
            Self(loom::sync::Mutex::new(value))
        }

        pub fn lock(&self) -> loom::sync::MutexGuard<'_, T> {
            self.0
                .lock()
                .expect("governor state mutex poisoned inside a loom model")
        }
    }
}

#[cfg(shuttle)]
mod imp {
    pub use shuttle::sync::atomic::{AtomicU64, Ordering};

    /// Shuttle's mutex, with its poison result collapsed for the same reason as
    /// the loom variant.
    pub struct Mutex<T>(shuttle::sync::Mutex<T>);

    impl<T> Mutex<T> {
        pub fn new(value: T) -> Self {
            Self(shuttle::sync::Mutex::new(value))
        }

        pub fn lock(&self) -> shuttle::sync::MutexGuard<'_, T> {
            self.0
                .lock()
                .expect("governor state mutex poisoned inside a shuttle schedule")
        }
    }
}

#[cfg(not(any(loom, shuttle)))]
mod imp {
    pub use parking_lot::Mutex;
    pub use std::sync::atomic::{AtomicU64, Ordering};
}

pub use imp::{AtomicU64, Mutex, Ordering};
