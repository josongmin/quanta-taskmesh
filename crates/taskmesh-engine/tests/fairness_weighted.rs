//! T04: weighted-fair scheduling favors the heavier class without starving the
//! lighter one.

mod harness;
use harness::*;

#[test]
fn weighted_favors_heavy_without_starving_light() {
    let g = contended(vec![
        ("heavy", weighted_class(4)),
        ("light", weighted_class(1)),
    ]);
    let filler = admit_filler(&g);

    let mut tickets = Vec::new();
    for i in 0..5 {
        tickets.push(queue(&g, "heavy", &format!("h{i}")));
        tickets.push(queue(&g, "light", &format!("l{i}")));
    }

    g.release(filler);
    let order = drain(&g, &tickets);

    // Over the first six promotions the 4:1 weighting should give heavy the
    // clear majority, but light must appear (no starvation).
    let window = &order[..6];
    let heavy = window.iter().filter(|c| *c == "heavy").count();
    let light = window.iter().filter(|c| *c == "light").count();
    assert!(heavy >= 4, "heavy should dominate early window: {order:?}");
    assert!(light >= 1, "light must not starve: {order:?}");
    assert!(heavy > light, "{order:?}");
}
