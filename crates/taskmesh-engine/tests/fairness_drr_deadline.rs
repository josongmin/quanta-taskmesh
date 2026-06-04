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
