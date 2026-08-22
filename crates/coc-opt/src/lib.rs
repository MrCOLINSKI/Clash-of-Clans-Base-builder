//! Layout search.
//!
//! Simulated annealing over legal layouts, scored by the geometric metrics in
//! `coc-core`.
//!
//! # The honest caveat
//!
//! The project's rule is that real fitness comes from running battles, and that
//! optimizing against anything else risks confidently wrong bases. That rule
//! stands: this searches a **geometric proxy**, not simulated destruction, and
//! its output is "measurably better arranged", not "measurably harder to
//! three-star". When the simulator is calibrated, [`Objective`] is where a
//! battle-based fitness slots in without touching the search.

use coc_core::grid::{idx, Layout, Rect, TOTAL};
use coc_core::legality;
use coc_core::metrics::{evaluate, Metrics, Weights};
use rand::Rng;
use rand_chacha::rand_core::SeedableRng;
use rand_chacha::ChaCha8Rng;

/// What the search is trying to maximise.
pub trait Objective: Sync {
    fn score(&self, layout: &Layout) -> f64;
    fn detail(&self, layout: &Layout) -> Metrics;
}

/// Geometric objective backed by `coc-core`'s metrics.
pub struct Geometric {
    pub weights: Weights,
}

impl Objective for Geometric {
    fn score(&self, layout: &Layout) -> f64 {
        evaluate(layout, &self.weights).score
    }
    fn detail(&self, layout: &Layout) -> Metrics {
        evaluate(layout, &self.weights)
    }
}

/// Search settings.
#[derive(Debug, Clone, Copy)]
pub struct Config {
    pub iterations: u32,
    /// Starting temperature for the acceptance rule.
    pub t_start: f64,
    pub t_end: f64,
    pub seed: u64,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            iterations: 12_000,
            t_start: 0.020,
            t_end: 0.0005,
            seed: 42,
        }
    }
}

/// Outcome of a search.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub layout: Layout,
    pub start_score: f64,
    pub final_score: f64,
    pub start: Metrics,
    pub end: Metrics,
    pub accepted: u32,
    pub improved: u32,
    pub rejected_illegal: u32,
}

impl Outcome {
    /// Relative improvement over the seed, as a percentage.
    pub fn gain_percent(&self) -> f64 {
        if self.start_score <= 0.0 {
            0.0
        } else {
            (self.final_score - self.start_score) / self.start_score * 100.0
        }
    }
}

/// Anneals a layout towards a higher objective score.
///
/// Every candidate passes through the legality check before it is scored, so
/// the search cannot emit an illegal base however it mutates.
pub fn anneal(seed_layout: &Layout, obj: &dyn Objective, cfg: Config) -> Outcome {
    let mut rng = ChaCha8Rng::seed_from_u64(cfg.seed);
    let mut current = seed_layout.clone();
    let mut cur_score = obj.score(&current);

    let start = obj.detail(&current);
    let start_score = cur_score;
    let mut best = current.clone();
    let mut best_score = cur_score;
    let (mut accepted, mut improved, mut rejected) = (0u32, 0u32, 0u32);

    for i in 0..cfg.iterations {
        let t = cfg.t_start * (cfg.t_end / cfg.t_start).powf(i as f64 / cfg.iterations as f64);

        let Some(candidate) = mutate(&current, &mut rng) else {
            rejected += 1;
            continue;
        };

        let s = obj.score(&candidate);
        let delta = s - cur_score;
        // Uphill always; downhill with a temperature-dependent probability, so
        // the search can leave a local optimum early and settles later.
        let accept = delta > 0.0 || rng.gen::<f64>() < (delta / t).exp();
        if accept {
            if delta > 0.0 {
                improved += 1;
            }
            accepted += 1;
            current = candidate;
            cur_score = s;
            if cur_score > best_score {
                best_score = cur_score;
                best = current.clone();
            }
        }
    }

    Outcome {
        start,
        end: obj.detail(&best),
        layout: best,
        start_score,
        final_score: best_score,
        accepted,
        improved,
        rejected_illegal: rejected,
    }
}

/// One mutation. Returns `None` when the result would be illegal.
fn mutate(layout: &Layout, rng: &mut ChaCha8Rng) -> Option<Layout> {
    if layout.placements.is_empty() {
        return None;
    }
    let mut next = layout.clone();
    // Walls are left exactly as the builder laid them.
    //
    // There used to be a third mutation that moved one wall tile to a random
    // adjacent square. A single-tile nudge can only ever break a lattice — it
    // cannot discover a better one — and that is what it did: the seed's wall
    // network is a single connected component at every town hall, and annealing
    // was shredding TH15's into 111 loose blocks while reporting a *higher*
    // score, because scattered tiles still interrupt the enclosure flood fill.
    //
    // `metrics::wall_integrity` now prices that damage, which stopped the worst
    // of it, but the mutation remained a move whose entire reachable
    // neighbourhood is worse than where it started. The builder sizes the
    // lattice from what has to fit inside it and subdivides with the leftover
    // budget; the optimizer's job is to arrange structures within that.
    match rng.gen_range(0..100) {
        0..=63 => translate(&mut next, rng)?,
        _ => swap_pair(&mut next, rng)?,
    }
    Some(next)
}

/// Occupancy excluding one placement, so it can be tested in a new position.
fn occupancy_without(layout: &Layout, skip: usize) -> Vec<u8> {
    let mut g = vec![0u8; (TOTAL * TOTAL) as usize];
    for (i, p) in layout.placements.iter().enumerate() {
        if i == skip {
            continue;
        }
        for (x, y) in p.rect.tiles() {
            if let Some(c) = idx(x, y) {
                g[c] = 1;
            }
        }
    }
    for &(x, y) in &layout.walls {
        if let Some(c) = idx(x, y) {
            if g[c] == 0 {
                g[c] = 2;
            }
        }
    }
    g
}

/// Moves one structure a short distance.
fn translate(layout: &mut Layout, rng: &mut ChaCha8Rng) -> Option<()> {
    let i = rng.gen_range(0..layout.placements.len());
    let occ = occupancy_without(layout, i);
    let r = layout.placements[i].rect;
    for _ in 0..8 {
        let dx = rng.gen_range(-3..=3);
        let dy = rng.gen_range(-3..=3);
        if dx == 0 && dy == 0 {
            continue;
        }
        let moved = Rect::new(r.x + dx, r.y + dy, r.w, r.h);
        if legality::can_place(&moved, &occ) {
            layout.placements[i].rect = moved;
            return Some(());
        }
    }
    None
}

/// Exchanges the positions of two same-sized structures.
///
/// Restricted to equal footprints so the swap is always geometrically valid,
/// which makes it a cheap way to reorder a base without disturbing packing.
fn swap_pair(layout: &mut Layout, rng: &mut ChaCha8Rng) -> Option<()> {
    let n = layout.placements.len();
    if n < 2 {
        return None;
    }
    for _ in 0..12 {
        let a = rng.gen_range(0..n);
        let b = rng.gen_range(0..n);
        if a == b {
            continue;
        }
        let (ra, rb) = (layout.placements[a].rect, layout.placements[b].rect);
        if ra.w != rb.w || ra.h != rb.h || layout.placements[a].name == layout.placements[b].name {
            continue;
        }
        layout.placements[a].rect = rb;
        layout.placements[b].rect = ra;
        return Some(());
    }
    None
}


#[cfg(test)]
mod tests {
    use super::*;
    use coc_core::grid::Placement;

    fn base() -> Layout {
        let mut l = Layout::new(17);
        for i in 0..10 {
            l.placements.push(Placement {
                name: format!("Cannon{i}"),
                level: 1,
                rect: Rect::new(4 + (i % 5) * 4, 4 + (i / 5) * 4, 3, 3),
                class: "Defense".into(),
                range: 900,
                min_range: 0,
                is_trap: false,
                air_targets: true,
                ground_targets: true,
            });
        }
        l
    }

    #[test]
    fn annealing_never_produces_overlaps() {
        let obj = Geometric { weights: Weights::war() };
        let out = anneal(&base(), &obj, Config { iterations: 2000, ..Default::default() });
        for (i, a) in out.layout.placements.iter().enumerate() {
            for b in out.layout.placements.iter().skip(i + 1) {
                assert!(!a.rect.overlaps(&b.rect), "{} overlaps {}", a.name, b.name);
            }
            assert!(a.rect.inside_buildable(), "{} left the buildable area", a.name);
        }
    }

    #[test]
    fn annealing_improves_or_holds_the_score() {
        let obj = Geometric { weights: Weights::war() };
        let out = anneal(&base(), &obj, Config { iterations: 3000, ..Default::default() });
        assert!(
            out.final_score >= out.start_score,
            "best-so-far must never regress: {} -> {}",
            out.start_score,
            out.final_score
        );
    }

    #[test]
    fn same_seed_gives_an_identical_result() {
        let obj = Geometric { weights: Weights::war() };
        let cfg = Config { iterations: 1500, seed: 99, ..Default::default() };
        let a = anneal(&base(), &obj, cfg);
        let b = anneal(&base(), &obj, cfg);
        assert_eq!(a.layout, b.layout);
        assert_eq!(a.final_score.to_bits(), b.final_score.to_bits());
    }

    #[test]
    fn structure_count_is_conserved() {
        let obj = Geometric { weights: Weights::war() };
        let start = base();
        let out = anneal(&start, &obj, Config { iterations: 2000, ..Default::default() });
        assert_eq!(out.layout.placements.len(), start.placements.len());
    }
}

#[cfg(test)]
mod wall_preservation {
    use super::*;
    use coc_core::grid::Layout;

    /// Number of 4-connected components in a layout's wall network.
    fn components(l: &Layout) -> usize {
        let set: std::collections::HashSet<(i32, i32)> = l.walls.iter().copied().collect();
        let mut seen = std::collections::HashSet::new();
        let mut n = 0;
        for &p in &l.walls {
            if !seen.insert(p) {
                continue;
            }
            n += 1;
            let mut stack = vec![p];
            while let Some((x, y)) = stack.pop() {
                for q in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
                    if set.contains(&q) && seen.insert(q) {
                        stack.push(q);
                    }
                }
            }
        }
        n
    }

    #[test]
    fn annealing_never_breaks_up_the_wall_network() {
        // The regression: annealing used to shred a single connected lattice
        // into as many as 111 loose blocks — TH15 — while reporting a better
        // score, because scattered tiles still interrupt the enclosure flood
        // fill. Walls now come out of the optimizer exactly as they went in.
        let Ok(d) = coc_data::GameData::load_default() else { return };
        let obj = Geometric { weights: coc_core::Weights::war() };
        for th in [3, 9, 15, 17] {
            let seed = coc_core::builder::seed(&d, th, 0xC0FFEE);
            let before = components(&seed);
            let out = anneal(&seed, &obj, Config { iterations: 4_000, ..Config::default() });
            assert_eq!(
                out.layout.walls, seed.walls,
                "TH{th}: the optimizer moved wall tiles"
            );
            assert_eq!(components(&out.layout), before, "TH{th}: wall network changed");
            assert_eq!(before, 1, "TH{th}: the seed lattice should be one piece");
        }
    }

    #[test]
    fn annealing_still_improves_the_layout() {
        // Removing a mutation must not leave the search unable to do anything.
        let Ok(d) = coc_data::GameData::load_default() else { return };
        let obj = Geometric { weights: coc_core::Weights::war() };
        let seed = coc_core::builder::seed(&d, 11, 0xC0FFEE);
        let out = anneal(&seed, &obj, Config { iterations: 6_000, ..Config::default() });
        assert!(
            out.final_score >= out.start_score,
            "annealing went backwards: {:.4} -> {:.4}",
            out.start_score,
            out.final_score
        );
        assert!(out.improved > 0, "no candidate was ever an improvement");
    }
}
