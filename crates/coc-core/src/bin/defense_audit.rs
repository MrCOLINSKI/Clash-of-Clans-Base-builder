//! `cargo run -p coc-core --bin defense_audit`
//!
//! Judges a layout by the rules a real war-base analyser encodes, rather than
//! by the geometric proxies this project invented.
//!
//! Transcribed from `danielholmes/coc-base-analyser`, whose rule set is the
//! codified version of what experienced players check. Troop ranges come from
//! that tool and agree with the extracted data (Archer 3.5 tiles = the shipped
//! `AttackRange` of 350).
//!
//! Four rules are implemented here, the ones that are pure geometry:
//!
//! - **High-HP under air defence** — every position a Dragon could attack a
//!   Town Hall, storage or Clan Castle from must sit inside some air-targeting
//!   defence's range.
//! - **Air-sniped defence** — every position a Minion could attack a
//!   *ground-only* defence from must likewise be covered, or the defence is
//!   free damage.
//! - **Minimum compartments** — at least eight wall compartments containing
//!   buildings.
//! - **Archer anchor** — no tile where an Archer can stand, hit a building, and
//!   be outside every ground-targeting defence.

use coc_core::builder::{self, Plan};
use coc_core::grid::{idx, Layout, Placement, Rect, BUILDABLE, ORIGIN, TOTAL};
use coc_data::GameData;

const ARCHER: f64 = 3.5;
const MINION: f64 = 0.75;
const DRAGON: f64 = 1.0;
/// Sampling step for troop standing positions, in tiles.
const STEP: f64 = 0.5;

fn dist_to_rect(px: f64, py: f64, r: &Rect) -> f64 {
    let dx = (r.x as f64 - px).max(px - r.right() as f64).max(0.0);
    let dy = (r.y as f64 - py).max(py - r.bottom() as f64).max(0.0);
    (dx * dx + dy * dy).sqrt()
}

fn in_range(p: &Placement, px: f64, py: f64) -> bool {
    let (cx, cy) = p.rect.centre2();
    let (cx, cy) = (cx as f64 / 2.0, cy as f64 / 2.0);
    let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
    let r = p.range as f64 / 100.0;
    let rmin = p.min_range as f64 / 100.0;
    d <= r && d >= rmin
}

/// Where a troop of this range stops to attack a footprint: the ring at its
/// reach, sampled on a half-tile grid.
fn attack_positions(r: &Rect, range: f64) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    let pad = range.ceil() as i32 + 1;
    let mut y = (r.y - pad) as f64;
    while y <= (r.bottom() + pad) as f64 {
        let mut x = (r.x - pad) as f64;
        while x <= (r.right() + pad) as f64 {
            let d = dist_to_rect(x, y, r);
            if d <= range && d > range - STEP {
                out.push((x, y));
            }
            x += STEP;
        }
        y += STEP;
    }
    out
}

fn is_high_hp(p: &Placement) -> bool {
    p.is_town_hall() || p.is_storage() || p.name == "Clan Castle"
}

/// Wall compartments containing at least one building.
fn compartments_with_buildings(l: &Layout) -> usize {
    let occ = l.occupancy(); // 0 empty, 1 structure, 2 wall
    let mut seen = vec![false; (TOTAL * TOTAL) as usize];
    let mut count = 0;
    for y in ORIGIN..ORIGIN + BUILDABLE {
        for x in ORIGIN..ORIGIN + BUILDABLE {
            let c = idx(x, y).unwrap();
            if seen[c] || occ[c] == 2 {
                continue;
            }
            // Flood this region; walls bound it.
            let mut stack = vec![(x, y)];
            seen[c] = true;
            let mut has_building = false;
            while let Some((cx, cy)) = stack.pop() {
                let ci = idx(cx, cy).unwrap();
                if occ[ci] == 1 {
                    has_building = true;
                }
                for (nx, ny) in [(cx + 1, cy), (cx - 1, cy), (cx, cy + 1), (cx, cy - 1)] {
                    if nx < ORIGIN || ny < ORIGIN || nx >= ORIGIN + BUILDABLE || ny >= ORIGIN + BUILDABLE {
                        continue;
                    }
                    let ni = idx(nx, ny).unwrap();
                    if !seen[ni] && occ[ni] != 2 {
                        seen[ni] = true;
                        stack.push((nx, ny));
                    }
                }
            }
            if has_building {
                count += 1;
            }
        }
    }
    count
}

struct Report {
    sniped: usize,
    exposed_hp: usize,
    compartments: usize,
    anchors: usize,
}

fn audit(l: &Layout) -> Report {
    let air: Vec<&Placement> = l.defenses().filter(|d| d.air_targets).collect();
    let ground: Vec<&Placement> = l.defenses().filter(|d| d.ground_targets).collect();

    // Air-sniped: a ground-only defence with any Minion position uncovered.
    let sniped = l
        .defenses()
        .filter(|d| d.ground_targets && !d.air_targets)
        .filter(|d| {
            attack_positions(&d.rect, MINION)
                .iter()
                .any(|&(px, py)| !air.iter().any(|a| in_range(a, px, py)))
        })
        .count();

    // High-HP buildings a Dragon can hit from outside every air defence.
    let exposed_hp = l
        .placements
        .iter()
        .filter(|p| is_high_hp(p))
        .filter(|p| {
            attack_positions(&p.rect, DRAGON)
                .iter()
                .any(|&(px, py)| !air.iter().any(|a| in_range(a, px, py)))
        })
        .count();

    // Archer anchors: a free tile in reach of a building but of no ground defence.
    let occ = l.occupancy();
    let mut anchors = 0;
    for y in ORIGIN..ORIGIN + BUILDABLE {
        for x in ORIGIN..ORIGIN + BUILDABLE {
            if occ[idx(x, y).unwrap()] != 0 {
                continue;
            }
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            let hits = l
                .placements
                .iter()
                .any(|p| !p.is_trap && dist_to_rect(px, py, &p.rect) <= ARCHER);
            if hits && !ground.iter().any(|d| in_range(d, px, py)) {
                anchors += 1;
            }
        }
    }

    Report {
        sniped,
        exposed_hp,
        compartments: compartments_with_buildings(l),
        anchors,
    }
}

fn main() -> anyhow::Result<()> {
    let d = GameData::load_default()?;
    println!(
        "{:>4} {:>6}  {:>8} {:>10} {:>7} {:>8}  verdict",
        "TH", "plan", "sniped", "exposed HP", "comps", "anchors"
    );
    let mut fails = 0;
    for th in 1..=d.max_townhall() {
        for (label, plan) in [("grid", Plan::Lattice), ("ring", Plan::Ring)] {
            let l = builder::seed_with(&d, th, 0xC0FFEE, plan);
            let r = audit(&l);
            let ok = r.sniped == 0 && r.exposed_hp == 0 && r.anchors == 0
                && (th < 7 || r.compartments >= 8);
            if !ok && th >= 7 {
                fails += 1;
            }
            println!(
                "{th:>4} {label:>6}  {:>8} {:>10} {:>7} {:>8}  {}",
                r.sniped, r.exposed_hp, r.compartments, r.anchors,
                if ok { "ok" } else { "FAILS" }
            );
        }
    }
    println!("\nlayouts failing at least one analyser rule (TH7+): {fails}");
    Ok(())
}
