//! Geometric layout metrics.
//!
//! # What this is, and what it is not
//!
//! These are **proxy** measures computed from geometry alone: defence coverage,
//! how deep the town hall sits, how much of the base is enclosed by walls. They
//! are not simulated destruction.
//!
//! The project's own rule is that real fitness comes from running battles, and
//! that optimizing before the simulator is calibrated produces confidently
//! wrong bases. That still holds. What these metrics legitimately do is replace
//! *hand-placement* with something measurable and repeatable — a base scoring
//! 0.72 here is demonstrably better arranged than one scoring 0.31 by the same
//! ruler, which is a far cry from knowing how it survives a real attack.
//!
//! Every value is normalised to 0..1 so the weights are readable.

use crate::grid::{idx, Layout, BUILDABLE, ORIGIN, TOTAL};
use coc_data::model::UNITS_PER_TILE;
use serde::{Deserialize, Serialize};

/// Scores for one layout. All components are 0..1, higher is better.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    /// Mean number of defences covering a buildable tile, normalised.
    pub coverage: f64,
    /// Fraction of the buildable area covered by at least one defence.
    pub covered_fraction: f64,
    /// How deep the town hall sits, as a fraction of the maximum possible.
    pub th_depth: f64,
    /// Fraction of structures that sit inside a walled region.
    pub enclosed: f64,
    /// Fraction of storages that sit inside a walled region.
    pub loot_protected: f64,
    /// How evenly defence coverage is spread; penalises stacked defences and
    /// undefended sides alike.
    pub balance: f64,
    /// Fraction of structures not exposed on the outer ring.
    pub perimeter_safety: f64,
    /// Weighted composite.
    pub score: f64,
}

/// Weighting profile for the composite score.
#[derive(Debug, Clone, Copy)]
pub struct Weights {
    pub coverage: f64,
    pub covered_fraction: f64,
    pub th_depth: f64,
    pub enclosed: f64,
    pub loot_protected: f64,
    pub balance: f64,
    pub perimeter_safety: f64,
}

impl Weights {
    /// War profile: the town hall and three stars are what matter.
    pub fn war() -> Weights {
        Weights {
            coverage: 0.16,
            covered_fraction: 0.20,
            th_depth: 0.26,
            enclosed: 0.14,
            loot_protected: 0.04,
            balance: 0.12,
            perimeter_safety: 0.08,
        }
    }

    /// Farming profile: storages matter more than the town hall.
    pub fn farming() -> Weights {
        Weights {
            coverage: 0.16,
            covered_fraction: 0.18,
            th_depth: 0.06,
            enclosed: 0.16,
            loot_protected: 0.28,
            balance: 0.10,
            perimeter_safety: 0.06,
        }
    }

    /// Trophy profile: sits between the two.
    pub fn trophy() -> Weights {
        Weights {
            coverage: 0.18,
            covered_fraction: 0.20,
            th_depth: 0.18,
            enclosed: 0.16,
            loot_protected: 0.12,
            balance: 0.10,
            perimeter_safety: 0.06,
        }
    }

    pub fn named(profile: &str) -> Weights {
        match profile {
            "farming" => Weights::farming(),
            "trophy" => Weights::trophy(),
            _ => Weights::war(),
        }
    }
}

/// Per-tile count of how many defences reach that tile.
///
/// Range is the shipped `AttackRange` in game units; the minimum range of a
/// Mortar carves a genuine hole, which is why it is honoured here rather than
/// treated as full-disc coverage.
pub fn coverage_map(layout: &Layout) -> Vec<u16> {
    let mut map = vec![0u16; (TOTAL * TOTAL) as usize];
    for d in layout.defenses() {
        let (cx2, cy2) = d.rect.centre2();
        let r = d.range as f64 / UNITS_PER_TILE as f64;
        let rmin = d.min_range as f64 / UNITS_PER_TILE as f64;
        let lo = ((cx2 as f64 / 2.0) - r).floor() as i32;
        let hi = ((cx2 as f64 / 2.0) + r).ceil() as i32;
        let lo_y = ((cy2 as f64 / 2.0) - r).floor() as i32;
        let hi_y = ((cy2 as f64 / 2.0) + r).ceil() as i32;
        for y in lo_y..=hi_y {
            for x in lo..=hi {
                let dx = (x as f64 + 0.5) - cx2 as f64 / 2.0;
                let dy = (y as f64 + 0.5) - cy2 as f64 / 2.0;
                let dist = (dx * dx + dy * dy).sqrt();
                if dist <= r && dist >= rmin {
                    if let Some(c) = idx(x, y) {
                        map[c] += 1;
                    }
                }
            }
        }
    }
    map
}

/// Tiles enclosed by walls, found by flooding inward from the border.
///
/// Anything the flood cannot reach without crossing a wall is inside. This is
/// the same question a ground troop asks, which is why it is worth measuring
/// even before the pathfinder exists.
pub fn enclosed_map(layout: &Layout) -> Vec<bool> {
    let occ = layout.occupancy();
    let mut outside = vec![false; (TOTAL * TOTAL) as usize];
    let mut stack = Vec::new();

    for i in 0..TOTAL {
        for (x, y) in [(i, 0), (i, TOTAL - 1), (0, i), (TOTAL - 1, i)] {
            if let Some(c) = idx(x, y) {
                if occ[c] != 2 && !outside[c] {
                    outside[c] = true;
                    stack.push((x, y));
                }
            }
        }
    }
    while let Some((x, y)) = stack.pop() {
        for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
            if let Some(c) = idx(nx, ny) {
                // Walls block the flood; structures do not, since a troop can
                // walk around a building but must break a wall.
                if occ[c] != 2 && !outside[c] {
                    outside[c] = true;
                    stack.push((nx, ny));
                }
            }
        }
    }
    outside.into_iter().map(|o| !o).collect()
}

/// Scores a layout under a weighting profile.
pub fn evaluate(layout: &Layout, w: &Weights) -> Metrics {
    let cov = coverage_map(layout);
    let enc = enclosed_map(layout);

    let mut total = 0u64;
    let mut covered = 0u64;
    let mut tiles = 0u64;
    let mut quad = [0u64; 4];
    let mut quad_tiles = [0u64; 4];
    let mid = ORIGIN + BUILDABLE / 2;

    for y in ORIGIN..ORIGIN + BUILDABLE {
        for x in ORIGIN..ORIGIN + BUILDABLE {
            let c = idx(x, y).expect("in bounds");
            let n = cov[c] as u64;
            total += n;
            tiles += 1;
            if n > 0 {
                covered += 1;
            }
            let q = ((x >= mid) as usize) | (((y >= mid) as usize) << 1);
            quad[q] += n;
            quad_tiles[q] += 1;
        }
    }

    let mean = total as f64 / tiles.max(1) as f64;
    // Six overlapping defences on a tile is already a strong core, so the
    // normaliser saturates there rather than rewarding unbounded stacking.
    let coverage = (mean / 6.0).min(1.0);
    let covered_fraction = covered as f64 / tiles.max(1) as f64;

    // Balance: the weakest quadrant relative to the strongest. A base that only
    // survives from one side scores badly here, which is the naive-fitness trap
    // the brief warns about.
    let dens: Vec<f64> = (0..4)
        .map(|i| quad[i] as f64 / quad_tiles[i].max(1) as f64)
        .collect();
    let hi = dens.iter().cloned().fold(0.0f64, f64::max);
    let lo = dens.iter().cloned().fold(f64::MAX, f64::min);
    let balance = if hi > 0.0 { lo / hi } else { 0.0 };

    // Town hall depth: distance from the nearest buildable edge, over the
    // maximum achievable distance.
    let th_depth = layout
        .town_hall()
        .map(|th| {
            let (cx2, cy2) = th.rect.centre2();
            let (cx, cy) = (cx2 as f64 / 2.0, cy2 as f64 / 2.0);
            let d = [
                cx - ORIGIN as f64,
                cy - ORIGIN as f64,
                (ORIGIN + BUILDABLE) as f64 - cx,
                (ORIGIN + BUILDABLE) as f64 - cy,
            ]
            .into_iter()
            .fold(f64::MAX, f64::min);
            (d / (BUILDABLE as f64 / 2.0)).clamp(0.0, 1.0)
        })
        .unwrap_or(0.0);

    let inside = |p: &crate::grid::Placement| {
        let (cx2, cy2) = p.rect.centre2();
        idx(cx2 / 2, cy2 / 2).is_some_and(|c| enc[c])
    };

    let structures: Vec<_> = layout.placements.iter().filter(|p| !p.is_trap).collect();
    let enclosed = if structures.is_empty() {
        0.0
    } else {
        structures.iter().filter(|p| inside(p)).count() as f64 / structures.len() as f64
    };

    let stores: Vec<_> = layout.placements.iter().filter(|p| p.is_storage()).collect();
    let loot_protected = if stores.is_empty() {
        1.0
    } else {
        stores.iter().filter(|p| inside(p)).count() as f64 / stores.len() as f64
    };

    // Perimeter safety: structures hugging the buildable edge are free damage.
    let edge = 3;
    let exposed = structures
        .iter()
        .filter(|p| {
            p.rect.x < ORIGIN + edge
                || p.rect.y < ORIGIN + edge
                || p.rect.right() > ORIGIN + BUILDABLE - edge
                || p.rect.bottom() > ORIGIN + BUILDABLE - edge
        })
        .count();
    let perimeter_safety = if structures.is_empty() {
        0.0
    } else {
        1.0 - exposed as f64 / structures.len() as f64
    };

    let score = w.coverage * coverage
        + w.covered_fraction * covered_fraction
        + w.th_depth * th_depth
        + w.enclosed * enclosed
        + w.loot_protected * loot_protected
        + w.balance * balance
        + w.perimeter_safety * perimeter_safety;

    Metrics {
        coverage,
        covered_fraction,
        th_depth,
        enclosed,
        loot_protected,
        balance,
        perimeter_safety,
        score,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::{Placement, Rect};

    fn def(name: &str, x: i32, y: i32, range: i32, min_range: i32) -> Placement {
        Placement {
            name: name.into(),
            level: 1,
            rect: Rect::new(x, y, 3, 3),
            class: "Defense".into(),
            range,
            min_range,
            is_trap: false,
        }
    }

    #[test]
    fn coverage_respects_minimum_range() {
        // A Mortar's 4-tile dead zone must be a real hole, not filled in.
        let mut l = Layout::new(17);
        l.placements.push(def("Mortar", 20, 20, 1100, 400));
        let cov = coverage_map(&l);
        let (cx, cy) = (21, 21);
        assert_eq!(cov[idx(cx, cy).unwrap()], 0, "centre is inside the dead zone");
        assert!(cov[idx(cx + 7, cy).unwrap()] > 0, "in the covered annulus");
        assert_eq!(cov[idx(cx + 20, cy).unwrap()], 0, "beyond range");
    }

    #[test]
    fn walls_enclose_and_structures_do_not() {
        let mut l = Layout::new(17);
        // A closed 6x6 wall box.
        for i in 18..24 {
            for (x, y) in [(i, 18), (i, 23), (18, i), (23, i)] {
                l.walls.push((x, y));
            }
        }
        let enc = enclosed_map(&l);
        assert!(enc[idx(20, 20).unwrap()], "inside the box is enclosed");
        assert!(!enc[idx(10, 10).unwrap()], "outside is not");

        // Open the box and the interior becomes reachable.
        l.walls.retain(|&(x, y)| !(x == 20 && y == 18));
        let enc2 = enclosed_map(&l);
        assert!(!enc2[idx(20, 20).unwrap()], "a gap breaks the enclosure");
    }

    #[test]
    fn town_hall_depth_rewards_the_centre() {
        let mk = |x, y| {
            let mut l = Layout::new(17);
            l.placements.push(Placement {
                name: "Town Hall".into(),
                level: 17,
                rect: Rect::new(x, y, 4, 4),
                class: "Town Hall".into(),
                range: 0,
                min_range: 0,
                is_trap: false,
            });
            evaluate(&l, &Weights::war()).th_depth
        };
        let centre = mk(20, 20);
        let corner = mk(ORIGIN, ORIGIN);
        assert!(centre > 0.9, "a centred town hall should be deep, got {centre}");
        // A 4x4 town hall jammed into the corner still has its centre two
        // tiles in, so 0.1 is the floor rather than zero.
        assert!(corner <= 0.1, "a cornered one should be at the floor, got {corner}");
        assert!(centre > corner * 8.0);
    }

    #[test]
    fn balance_punishes_a_one_sided_base() {
        let mut lop = Layout::new(17);
        for i in 0..6 {
            lop.placements.push(def("Cannon", 6 + i * 3, 6, 900, 0));
        }
        let mut even = Layout::new(17);
        for (x, y) in [(10, 10), (30, 10), (10, 30), (30, 30), (20, 20), (20, 10)] {
            even.placements.push(def("Cannon", x, y, 900, 0));
        }
        let a = evaluate(&lop, &Weights::war());
        let b = evaluate(&even, &Weights::war());
        assert!(
            b.balance > a.balance,
            "spread base should out-balance a one-sided one: {} vs {}",
            b.balance,
            a.balance
        );
    }

    #[test]
    fn every_component_stays_in_range() {
        let mut l = Layout::new(17);
        for i in 0..8 {
            l.placements.push(def("Cannon", 6 + i * 4, 20, 900, 0));
        }
        let m = evaluate(&l, &Weights::war());
        for (n, v) in [
            ("coverage", m.coverage),
            ("covered_fraction", m.covered_fraction),
            ("th_depth", m.th_depth),
            ("enclosed", m.enclosed),
            ("loot_protected", m.loot_protected),
            ("balance", m.balance),
            ("perimeter_safety", m.perimeter_safety),
            ("score", m.score),
        ] {
            assert!((0.0..=1.0).contains(&v), "{n} out of range: {v}");
        }
    }
}
