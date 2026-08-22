//! `cargo run -p coc-core --bin meta_audit`
//!
//! Checks each town hall's seeded layout against the mechanical rules taken
//! from current war-base design practice. Mechanical only: rules that are a
//! property of the arrangement, not a claim about how an attack would run.
//! Anything needing the latter waits on calibration (ASSUMPTIONS 5).

use coc_core::grid::{Placement, BUILDABLE, ORIGIN};
use coc_core::{builder, metrics, Weights};
use coc_data::GameData;
use std::collections::HashMap;

/// Defences whose value collapses if they share a room or sit on the rim.
const HEAVY: [&str; 5] = [
    "Scattershot",
    "Eagle Artillery",
    "Monolith",
    "Inferno Tower",
    "Multi Gear Tower",
];

fn main() -> anyhow::Result<()> {
    let d = GameData::load_default()?;
    println!(
        "{:>4} {:>7} {:>9} {:>8} {:>7} {:>7}  verdict",
        "TH", "stacked", "on rim", "AD gap", "walled", "score"
    );

    let mut failures = 0;
    for th in 1..=d.max_townhall() {
        let l = builder::seed(&d, th, 0xC0FFEE);
        let enc = metrics::enclosed_map(&l);
        let inside = |p: &Placement| {
            p.rect
                .tiles()
                .all(|(x, y)| coc_core::grid::idx(x, y).is_some_and(|c| enc[c]))
        };

        // R1: two of the same heavy defence sharing a compartment.
        let mut room: HashMap<(String, i32, i32), usize> = HashMap::new();
        for p in l.placements.iter().filter(|p| p.is_defense()) {
            *room
                .entry((p.name.clone(), p.rect.x / 10, p.rect.y / 10))
                .or_default() += 1;
        }
        let stacked: usize = HEAVY
            .iter()
            .map(|n| {
                let total = l.placements.iter().filter(|p| &p.name == n).count();
                if total < 2 {
                    return 0;
                }
                room.iter()
                    .filter(|((m, _, _), _)| m == n)
                    .map(|(_, c)| c.saturating_sub(1))
                    .sum()
            })
            .sum();

        // R2: a heavy defence sitting on the outer ring, where a hero pair can
        // pick it off from outside without committing to the base.
        let rim = 4;
        let on_rim = l
            .placements
            .iter()
            .filter(|p| HEAVY.contains(&p.name.as_str()))
            .filter(|p| {
                p.rect.x < ORIGIN + rim
                    || p.rect.y < ORIGIN + rim
                    || p.rect.right() > ORIGIN + BUILDABLE - rim
                    || p.rect.bottom() > ORIGIN + BUILDABLE - rim
            })
            .count();

        // R3: air defences spread, so one funnel does not clear them together.
        let ads: Vec<_> = l
            .placements
            .iter()
            .filter(|p| p.name == "Air Defense")
            .map(|p| p.rect.centre2())
            .collect();
        let ad_gap = min_pair_gap(&ads);

        let defs: Vec<_> = l.placements.iter().filter(|p| p.is_defense()).collect();
        let walled = defs.iter().filter(|p| inside(p)).count();
        let all_walled = defs.is_empty() || walled == defs.len();

        let m = metrics::evaluate(&l, &Weights::war());
        // Required separation scales with the base: 12 tiles is unreachable
        // inside a TH6 lattice and trivial inside a TH17 one.
        let span = l
            .placements
            .iter()
            .map(|p| p.rect.right().max(p.rect.bottom()))
            .max()
            .unwrap_or(0)
            - l.placements
                .iter()
                .map(|p| p.rect.x.min(p.rect.y))
                .min()
                .unwrap_or(0);
        let want = (span * 40 / 100).clamp(6, 14);
        let ok = stacked == 0 && on_rim == 0 && (ads.len() < 2 || ad_gap >= want);
        if !ok {
            failures += 1;
        }
        println!(
            "{th:>4} {stacked:>7} {on_rim:>9} {:>8} {:>3}/{:<3} {:>7.3}  {}",
            if ads.len() < 2 { "-".into() } else { format!("{ad_gap}/{want}") },
            walled,
            defs.len(),
            m.score,
            if ok { "ok" } else { "FAILS" }
        );
        let _ = all_walled;
    }
    println!("\ntown halls failing at least one mechanical rule: {failures}");
    Ok(())
}

/// Smallest gap between any two points, in tiles (inputs are doubled centres).
fn min_pair_gap(pts: &[(i32, i32)]) -> i32 {
    let mut best = i32::MAX;
    for i in 0..pts.len() {
        for j in i + 1..pts.len() {
            let dx = (pts[i].0 - pts[j].0) / 2;
            let dy = (pts[i].1 - pts[j].1) / 2;
            best = best.min(((dx * dx + dy * dy) as f64).sqrt() as i32);
        }
    }
    if best == i32::MAX {
        0
    } else {
        best
    }
}
