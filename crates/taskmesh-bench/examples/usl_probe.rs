//! USL probe (ADR 9000 / P3): sweep admit→release throughput across concurrency
//! levels on one shared governor, then fit the Universal Scalability Law to
//! recover the contention (α) and coherency (β) coefficients and the predicted
//! throughput peak (N_max) of the single global mutex.
//!
//! Run: `cargo run -p taskmesh-bench --example usl_probe --release`

use std::thread::available_parallelism;

use taskmesh_bench::loadgen::contention_throughput;
use taskmesh_bench::metrics::{argmax_throughput, fit_usl};

fn main() {
    let max = available_parallelism().map(|n| n.get()).unwrap_or(8);
    let mut levels = vec![1usize];
    let mut n = 2;
    while n <= max {
        levels.push(n);
        n *= 2;
    }
    if *levels.last().unwrap() != max {
        levels.push(max);
    }

    let ops = 200_000usize;
    let runs = 3;
    println!("# USL contention sweep  (ops/thread={ops}, {runs} runs averaged, counter clock)");
    println!("{:>8}  {:>16}", "threads", "throughput/s");

    let mut samples = Vec::new();
    for &t in &levels {
        let tput = (0..runs)
            .map(|_| contention_throughput(t, ops))
            .sum::<f64>()
            / runs as f64;
        println!("{t:>8}  {tput:>16.0}");
        samples.push((t as f64, tput));
    }

    // The empirical peak is always honest, even when the fit leaves its domain.
    if let Some((pn, pt)) = argmax_throughput(&samples) {
        println!("\nempirical peak:  N={pn:.0} threads  @  {pt:.0} ops/s");
    }

    match fit_usl(&samples) {
        Some(fit) => {
            println!(
                "USL fit:  α(contention)={:.5}  β(coherency)={:.6}  X(1)={:.0} ops/s",
                fit.alpha, fit.beta, fit.x1
            );
            if fit.is_in_domain() {
                match (fit.n_max, fit.peak_throughput) {
                    (Some(nmax), Some(peak)) => {
                        println!("predicted peak:  N_max≈{nmax:.1} threads  @  {peak:.0} ops/s")
                    }
                    _ => println!("near-linear scaling within range (α≈0, β≈0)"),
                }
            } else {
                // α≥1 or β<0: throughput degrades as workers are added. Reporting
                // this as "linear" would be a silent misread of the data.
                println!(
                    "⚠ retrograde / contention-bound: fit is outside the USL valid domain \
                     (need 0≤α<1, β≥0).\n  The single global mutex does not scale — adding \
                     workers reduces throughput. Trust the empirical peak above."
                );
            }
        }
        None => println!("USL fit unavailable (need an n=1 baseline plus ≥2 further points)"),
    }
}
