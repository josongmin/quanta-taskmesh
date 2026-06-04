//! T04: FIFO preserves global arrival order across classes.

mod harness;
use harness::*;

#[test]
fn fifo_preserves_arrival_order() {
    // Three FIFO classes, a global budget of one cpu unit: only one runs at a
    // time, so every other request queues and promotion order is observable.
    let g = contended(vec![
        ("x", fifo_class()),
        ("y", fifo_class()),
        ("z", fifo_class()),
    ]);

    let filler = admit_filler(&g);
    // Enqueue across classes in a known arrival order.
    let tickets = vec![
        queue(&g, "y", "y1"),
        queue(&g, "z", "z1"),
        queue(&g, "x", "x1"),
        queue(&g, "y", "y2"),
    ];

    g.release(filler);
    let order = drain(&g, &tickets);
    assert_eq!(order, vec!["y", "z", "x", "y"]);
}

#[test]
fn within_class_is_fifo() {
    let g = contended(vec![("x", fifo_class())]);
    let filler = admit_filler_on(&g, "x");
    let (t1, _) = queue(&g, "x", "a");
    let (t2, _) = queue(&g, "x", "b");
    g.release(filler);
    // first enqueued promotes first; second stays queued until t1 releases
    assert!(g.claim(t1).is_some());
    assert!(g.claim(t2).is_none());
}
