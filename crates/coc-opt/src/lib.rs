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

use coc_core::grid::{idx, Layout, Rect, BUILDABLE, ORIGIN, TOTAL};
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
    match rng.gen_range(0..100) {
        0..=54 => translate(&mut next, rng)?,
        55..=84 => swap_pair(&mut next, rng)?,
        _ => nudge_wall(&mut next, rng)?,
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

/// Moves one wall segment to an adjacent free tile.
fn nudge_wall(layout: &mut Layout, rng: &mut ChaCha8Rng) -> Option<()> {
    if layout.walls.is_empty() {
        return None;
    }
    let i = rng.gen_range(0..layout.walls.len());
    let (x, y) = layout.walls[i];
    let mut occ = vec![0u8; (TOTAL * TOTAL) as usize];
    for p in &layout.placements {
        for (px, py) in p.rect.tiles() {
            if let Some(c) = idx(px, py) {
                occ[c] = 1;
            }
        }
    }
    for (j, &(wx, wy)) in layout.walls.iter().enumerate() {
        if j != i {
            if let Some(c) = idx(wx, wy) {
                occ[c] = 2;
            }
        }
    }
    for _ in 0..6 {
        let nx = x + rng.gen_range(-1..=1);
        let ny = y + rng.gen_range(-1..=1);
        if (nx, ny) == (x, y) {
            continue;
        }
        if nx < ORIGIN || ny < ORIGIN || nx >= ORIGIN + BUILDABLE || ny >= ORIGIN + BUILDABLE {
            continue;
        }
        if idx(nx, ny).is_some_and(|c| occ[c] == 0) {
            layout.walls[i] = (nx, ny);
            return Some(());
        }
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
