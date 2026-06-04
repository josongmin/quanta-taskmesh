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

#[test]
fn the_whole_primary_tier_precedes_the_whole_best_effort_tier() {
    // Two primary classes and two best-effort classes, arrivals interleaved.
    // Every primary request must dispatch before any best-effort one, and each
    // tier is FIFO by arrival within itself.
    let g = contended(vec![
        ("p1", best_effort_class(false)),
        ("p2", best_effort_class(false)),
        ("b1", best_effort_class(true)),
        ("b2", best_effort_class(true)),
    ]);
    let filler = admit_filler(&g);

    let tickets = vec![
        queue(&g, "b1", "b1a"),
        queue(&g, "p1", "p1a"),
        queue(&g, "b2", "b2a"),
        queue(&g, "p2", "p2a"),
    ];

    g.release(filler);
    let order = drain(&g, &tickets);

    let last_primary = order
        .iter()
        .rposition(|c| c == "p1" || c == "p2")
        .expect("a primary ran");
    let first_best_effort = order
        .iter()
        .position(|c| c == "b1" || c == "b2")
        .expect("a best-effort ran");
    assert!(
        first_best_effort > last_primary,
        "best-effort tier must wait for the entire primary tier: {order:?}"
    );
    // Arrival-order within the primary tier (p1 arrived before p2).
    let p1 = order.iter().position(|c| c == "p1").unwrap();
    let p2 = order.iter().position(|c| c == "p2").unwrap();
    assert!(
        p1 < p2,
        "primary tier dispatches FIFO by arrival: {order:?}"
    );
}
