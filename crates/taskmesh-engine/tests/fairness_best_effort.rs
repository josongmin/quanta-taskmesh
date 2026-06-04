//! T04: a best-effort class never starves interactive work — it dispatches only
//! when no non-best-effort class is runnable.

mod harness;
use harness::*;

#[test]
fn best_effort_does_not_starve_interactive() {
    let g = contended(vec![
        ("interactive", best_effort_class(false)),
        ("batch", best_effort_class(true)),
    ]);
    let filler = admit_filler(&g);

    // Batch arrives first, but interactive must still be served first.
    let tickets = vec![
        queue(&g, "batch", "b1"),
        queue(&g, "batch", "b2"),
        queue(&g, "interactive", "i1"),
    ];

    g.release(filler);
    let order = drain(&g, &tickets);
    assert_eq!(order, vec!["interactive", "batch", "batch"]);
}
