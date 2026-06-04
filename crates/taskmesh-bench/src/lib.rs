//! `taskmesh-bench` — benchmark harness for the taskmesh control-plane.
//!
//! Heavy measurement dependencies (criterion, hdrhistogram, rand) are isolated
//! here so the library crates stay lean. See `docs/adr/9000-benchmark-strategy.md`
//! for the 9-pillar strategy. This crate currently implements the Phase-0
//! building blocks: open-loop latency recording (P1), control-plane behavioral
//! metrics (P5), and a seeded Poisson/Zipf workload generator (P4) shared with
//! proof fixtures.

pub mod loadgen;
pub mod metrics;
pub mod workload;
