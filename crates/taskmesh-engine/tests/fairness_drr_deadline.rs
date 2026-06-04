//! T04: deficit round-robin rotation and deadline-aware prioritization.

mod harness;
use harness::*;

#[test]
fn drr_rotates_across_classes_deterministically() {
    let g = contended(vec![
        ("a", drr_class(1)),
        ("b", drr_class(1)),
        ("c", drr_class(1)),
    ]);
    let filler = admit_filler(&g);

    let tickets = vec![
        queue(&g, "a", "a1"),
        queue(&g, "a", "a2"),
        queue(&g, "b", "b1"),
        queue(&g, "b", "b2"),
        queue(&g, "c", "c1"),
        queue(&g, "c", "c2"),
    ];

    g.release(filler);
    let order = drain(&g, &tickets);
    assert_eq!(order, vec!["a", "b", "c", "a", "b", "c"]);
}

#[test]
fn drr_serves_proportional_to_quantum_not_priority() {
    // a: quantum 1, b: quantum 3, with a 1:3 backlog. Classic deficit round-robin
    // interleaves *proportionally* — one `a` per three `b` — and both drain
    // together. This is the documented T04 behavior (bounded service difference),
    // NOT priority-by-quantum (which would serve all of `b` before any `a`) and
    // NOT plain round-robin (which would be 1:1). Regression guard for the ring.
    let g = contended(vec![("a", drr_class(1)), ("b", drr_class(3))]);
    let filler = admit_filler(&g);

    let mut tickets = Vec::new();
    for i in 0..4 {
        tickets.push(queue(&g, "a", &format!("a{i}")));
    }
    for i in 0..12 {
        tickets.push(queue(&g, "b", &format!("b{i}")));
    }

    g.release(filler);
    let order = drain(&g, &tickets);

    let expected: Vec<String> = std::iter::repeat_n(["a", "b", "b", "b"], 4)
        .flatten()
        .map(String::from)
        .collect();
    assert_eq!(
        order, expected,
        "DRR must interleave proportionally to quanta (1:3), not as priority"
    );
}

#[test]
fn deadline_aware_prioritizes_tighter_slack() {
    let g = contended(vec![
        ("tight", deadline_class(10)),
        ("loose", deadline_class(10_000)),
    ]);
    let filler = admit_filler(&g);

    // Loose arrives first, but tighter slack must win.
    let tickets = vec![
        queue(&g, "loose", "l1"),
        queue(&g, "tight", "t1"),
        queue(&g, "loose", "l2"),
        queue(&g, "tight", "t2"),
    ];

    g.release(filler);
    let order = drain(&g, &tickets);
    assert_eq!(order, vec!["tight", "tight", "loose", "loose"]);
}

#[test]
fn deadline_ties_break_by_arrival_order() {
    // Equal slack, all enqueued at the same instant → identical deadlines. The
    // tie-break is arrival (seq) order, so dispatch is global FIFO across classes.
    let g = contended(vec![("x", deadline_class(100)), ("y", deadline_class(100))]);
    let filler = admit_filler(&g);

    let tickets = vec![
        queue(&g, "x", "x1"),
        queue(&g, "y", "y1"),
        queue(&g, "x", "x2"),
        queue(&g, "y", "y2"),
    ];

    g.release(filler);
    let order = drain(&g, &tickets);
    assert_eq!(
        order,
        vec!["x", "y", "x", "y"],
        "equal deadlines must fall back to arrival order"
    );
}
