//! Path costing over the navigation grid.
//!
//! # Why a cost field and not a path
//!
//! Stage 2 of target selection costs a path to each of N candidates. Running N
//! separate searches would be the obvious implementation and the wrong one: a
//! single-source Dijkstra from the unit produces the cost to *every* reachable
//! cell in one sweep, so all N candidates are read off the same field. That
//! turns the 250-unit mass retarget after a Jump Spell into 250 searches rather
//! than 750, and it is why [`CostField`] rather than a `path()` function is the
//! primary API.
//!
//! # Wall cost is charged per wall tile, not per cell
//!
//! A wall is one tile, which is several nav cells. Charging on every wall cell
//! entered would make one wall cost `subtiles_per_tile` times too much, and
//! would make the cost depend on the grid resolution — a configuration knob
//! quietly changing the game's balance. So the cost is charged when a step
//! enters a wall cell belonging to a *different tile* than the cell it came
//! from: once per wall tile crossed, and twice for a double layer, which is
//! what a double layer should cost.
//!
//! # Diagonals
//!
//! A diagonal step is allowed only when both of its orthogonal neighbours are
//! passable, so a unit cannot squeeze through the corner gap between two
//! buildings placed corner to corner. ASSUMED: the client's behaviour here is
//! not documented and no shipped constant describes it. See ASSUMPTIONS.md 2.9.

use crate::navgrid::{Cell, NavGrid};
use crate::movement::Movement;
use coc_core::grid::Rect;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Sentinel for "no predecessor", i.e. the source or an unvisited cell.
const NO_PREV: u32 = u32::MAX;

/// Path costs from one source cell to every reachable cell.
#[derive(Debug, Clone)]
pub struct CostField {
    dist: Vec<i64>,
    prev: Vec<u32>,
    nav: i32,
    source: (i32, i32),
}

impl CostField {
    /// Cost of the cheapest path from the source to a cell, or `None` when the
    /// cell cannot be reached at any cost.
    pub fn cost(&self, x: i32, y: i32) -> Option<i64> {
        let i = self.index(x, y)?;
        (self.dist[i] != i64::MAX).then_some(self.dist[i])
    }

    pub fn source(&self) -> (i32, i32) {
        self.source
    }

    /// The cheapest path to a cell, source first, or `None` if unreachable.
    pub fn path_to(&self, x: i32, y: i32) -> Option<Vec<(i32, i32)>> {
        let mut i = self.index(x, y)?;
        if self.dist[i] == i64::MAX {
            return None;
        }
        let mut out = vec![(x, y)];
        while self.prev[i] != NO_PREV {
            i = self.prev[i] as usize;
            out.push(((i as i32) % self.nav, (i as i32) / self.nav));
        }
        out.reverse();
        Some(out)
    }

    /// The cheapest reachable cell among a set, with its cost.
    ///
    /// Ties break on the cell's grid index so the result does not depend on the
    /// order the caller happened to generate the set in.
    pub fn cheapest_of(&self, cells: &[(i32, i32)]) -> Option<((i32, i32), i64)> {
        cells
            .iter()
            .filter_map(|&(x, y)| {
                let i = self.index(x, y)?;
                (self.dist[i] != i64::MAX).then_some(((x, y), self.dist[i], i))
            })
            .min_by_key(|&(_, c, i)| (c, i))
            .map(|(cell, c, _)| (cell, c))
    }

    fn index(&self, x: i32, y: i32) -> Option<usize> {
        (x >= 0 && y >= 0 && x < self.nav && y < self.nav).then(|| (y * self.nav + x) as usize)
    }
}

/// The eight neighbours, orthogonals first.
///
/// Orthogonals lead so that a diagonal step's two guard cells are always
/// already known when it is considered.
const NEIGHBOURS: [(i32, i32); 8] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (1, -1),
    (-1, 1),
    (-1, -1),
];

/// Runs a single-source Dijkstra from a cell.
///
/// The source may itself be blocked — a unit dropped on a building's footprint
/// is a real case — in which case the field is empty rather than an error.
pub fn cost_field(grid: &NavGrid, movement: &Movement, from: (i32, i32)) -> CostField {
    let nav = grid.extent();
    let n = (nav * nav) as usize;
    let mut field = CostField {
        dist: vec![i64::MAX; n],
        prev: vec![NO_PREV; n],
        nav,
        source: from,
    };

    let Some(start) = grid.index(from.0, from.1) else {
        return field;
    };
    if !grid.passable(from.0, from.1) {
        return field;
    }

    let step = grid.step_cost() as i64;
    let diag = grid.diagonal_cost() as i64;
    let sub = grid.subtiles_per_tile();

    field.dist[start] = 0;
    // The index rides in the key so equal costs pop in a fixed order; without
    // it two runs of the same search can produce different paths.
    let mut heap = BinaryHeap::new();
    heap.push(Reverse((0i64, start)));

    while let Some(Reverse((d, i))) = heap.pop() {
        if d > field.dist[i] {
            continue;
        }
        let x = (i as i32) % nav;
        let y = (i as i32) / nav;
        for (k, (dx, dy)) in NEIGHBOURS.iter().enumerate() {
            let (nx, ny) = (x + dx, y + dy);
            let Some(j) = grid.index(nx, ny) else { continue };
            if !grid.passable(nx, ny) {
                continue;
            }
            let diagonal = k >= 4;
            if diagonal && !(grid.passable(x + dx, y) && grid.passable(x, y + dy)) {
                // No squeezing through a corner gap between two buildings.
                continue;
            }
            let mut cost = if diagonal { diag } else { step };
            let entered_new_tile =
                (nx.div_euclid(sub), ny.div_euclid(sub)) != (x.div_euclid(sub), y.div_euclid(sub));
            if grid.at(nx, ny) == Cell::Wall && entered_new_tile {
                cost += movement.wall_cost;
            }
            let nd = d + cost;
            if nd < field.dist[j] {
                field.dist[j] = nd;
                field.prev[j] = i as u32;
                heap.push(Reverse((nd, j)));
            }
        }
    }
    field
}

/// Straight-line cost between two cells, in game distance units.
///
/// What a flier or a burrowing unit pays: the layout is irrelevant to both, so
/// there is nothing to search. Rounded down to stay integral.
pub fn straight_cost(grid: &NavGrid, a: (i32, i32), b: (i32, i32)) -> i64 {
    let (ax, ay) = grid.position2(a.0, a.1);
    let (bx, by) = grid.position2(b.0, b.1);
    // Positions are doubled, so the distance is too; halve it at the end.
    let d2 = sq(bx as i64 - ax as i64) + sq(by as i64 - ay as i64);
    isqrt(d2) / 2
}

/// Squared straight-line distance between two cells, in doubled units.
///
/// Stage 1 of target selection sorts on this. Squared and doubled so it stays
/// exact: taking a root would round, and rounding changes which candidates
/// survive when two targets are nearly equidistant.
pub fn straight_dist2(grid: &NavGrid, a: (i32, i32), b: (i32, i32)) -> i64 {
    let (ax, ay) = grid.position2(a.0, a.1);
    let (bx, by) = grid.position2(b.0, b.1);
    sq(bx as i64 - ax as i64) + sq(by as i64 - ay as i64)
}

/// Squared distance from a cell to the nearest point of a tile rectangle, in
/// doubled game units.
///
/// Zero when the cell lies inside the rectangle. This is the measure a unit
/// uses against a building: range is to the building's edge, not its centre,
/// which is why a large building is easier to hit than a small one.
pub fn dist2_to_rect(grid: &NavGrid, cell: (i32, i32), rect: &Rect, units_per_tile: i32) -> i64 {
    let (px, py) = grid.position2(cell.0, cell.1);
    let u2 = (units_per_tile * 2) as i64;
    let (left, right) = (rect.x as i64 * u2, rect.right() as i64 * u2);
    let (top, bottom) = (rect.y as i64 * u2, rect.bottom() as i64 * u2);
    let dx = (left - px as i64).max(px as i64 - right).max(0);
    let dy = (top - py as i64).max(py as i64 - bottom).max(0);
    dx * dx + dy * dy
}

/// Cells a unit can attack a rectangle from: passable, and within range of it.
///
/// `range` is in game distance units. A melee unit's range is well under a
/// tile, so this yields the ring of cells touching the footprint; a Mortar-range
/// unit gets a much wider annulus and will stop far short of the building.
///
/// The returned cells are in grid order, which keeps downstream tie-breaking
/// deterministic.
pub fn attack_cells(
    grid: &NavGrid,
    rect: &Rect,
    range: i32,
    units_per_tile: i32,
) -> Vec<(i32, i32)> {
    let sub = grid.subtiles_per_tile();
    // Search a box around the footprint wide enough to hold the range, plus a
    // cell of slack for the half-cell offset of a cell centre.
    let pad = (range / units_per_tile) + 2;
    let (x0, y0) = ((rect.x - pad) * sub, (rect.y - pad) * sub);
    let (x1, y1) = ((rect.right() + pad) * sub, (rect.bottom() + pad) * sub);
    let limit = sq(range as i64 * 2);

    let mut out = Vec::new();
    for y in y0..y1 {
        for x in x0..x1 {
            if !grid.passable(x, y) {
                continue;
            }
            if dist2_to_rect(grid, (x, y), rect, units_per_tile) <= limit {
                out.push((x, y));
            }
        }
    }
    out
}

fn sq(v: i64) -> i64 {
    v * v
}

/// Integer square root, exact for every non-negative input.
///
/// Written out rather than taken from `f64::sqrt` because the simulator must
/// not depend on the host's floating-point rounding.
fn isqrt(v: i64) -> i64 {
    if v < 2 {
        return v.max(0);
    }
    let mut lo = 0i64;
    let mut hi = 3_037_000_499i64.min(v);
    while lo < hi {
        let mid = lo + (hi - lo + 1) / 2;
        if mid.saturating_mul(mid) <= v {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    lo
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{test_globals, PathingConfig};
    use coc_core::grid::{Layout, Placement};

    fn cfg() -> PathingConfig {
        PathingConfig::load_default(&test_globals()).expect("config loads")
    }

    fn building(x: i32, y: i32, w: i32, h: i32) -> Placement {
        Placement {
            name: "Cannon".into(),
            level: 1,
            rect: Rect::new(x, y, w, h),
            class: "Defense".into(),
            range: 900,
            min_range: 0,
            is_trap: false,
            air_targets: true,
            ground_targets: true,
        }
    }

    #[test]
    fn integer_sqrt_is_exact() {
        for v in [0i64, 1, 2, 3, 4, 15, 16, 17, 9_999, 1_000_000] {
            let r = isqrt(v);
            assert!(r * r <= v && (r + 1) * (r + 1) > v, "isqrt({v}) = {r}");
        }
    }

    #[test]
    fn cost_on_open_ground_is_distance() {
        let c = cfg();
        let g = NavGrid::build(&Layout::new(17), &c);
        let f = cost_field(&g, &Movement::ground(&c), (10, 10));
        // Ten orthogonal steps.
        assert_eq!(f.cost(20, 10), Some(10 * c.step_cost() as i64));
        // Ten diagonal steps cost the diagonal, not the orthogonal.
        assert_eq!(f.cost(20, 20), Some(10 * c.diagonal_cost() as i64));
        assert_eq!(f.cost(10, 10), Some(0));
    }

    #[test]
    fn a_building_is_walked_around_and_never_through() {
        let c = cfg();
        let mut l = Layout::new(17);
        l.placements.push(building(10, 8, 1, 8));
        let g = NavGrid::build(&l, &c);
        let (sx, sy) = g.tile_centre(8, 12);
        let f = cost_field(&g, &Movement::ground(&c), (sx, sy));

        let (tx, ty) = g.tile_centre(12, 12);
        let cost = f.cost(tx, ty).expect("the far side is reachable around");
        let straight = straight_cost(&g, (sx, sy), (tx, ty));
        assert!(cost > straight, "a detour must cost more than the line");

        let path = f.path_to(tx, ty).expect("path exists");
        for &(x, y) in &path {
            assert_ne!(g.at(x, y), Cell::Blocked, "path entered a building");
        }
    }

    #[test]
    fn a_wall_is_crossed_at_a_price_charged_once_per_tile() {
        let c = cfg();
        // One lone wall tile is simply walked around; it never costs anything.
        let mut lone = Layout::new(17);
        lone.walls.push((10, 10));
        let g1 = NavGrid::build(&lone, &c);
        let f1 = cost_field(&g1, &Movement::ground(&c), (16, 21));
        assert!(
            !f1.path_to(24, 21)
                .expect("reachable")
                .iter()
                .any(|&(x, y)| g1.at(x, y) == Cell::Wall),
            "no unit pays 10 tiles of cost to save a two-cell detour"
        );

        // Force a crossing: wall the whole column so there is no way round.
        let mut walled = Layout::new(17);
        for y in 0..coc_core::grid::TOTAL {
            walled.walls.push((10, y));
        }
        let g2 = NavGrid::build(&walled, &c);
        let f2 = cost_field(&g2, &Movement::ground(&c), (16, 21));
        let crossed = f2.cost(24, 21).expect("walls are traversable");
        let free = cost_field(&g2, &Movement { wall_cost: 0, ..Movement::ground(&c) }, (16, 21))
            .cost(24, 21)
            .expect("reachable");
        assert_eq!(
            crossed - free,
            c.wall_break_cost,
            "one wall tile should be charged exactly once"
        );
    }

    #[test]
    fn a_double_wall_costs_twice() {
        let c = cfg();
        let mut l = Layout::new(17);
        for y in 0..coc_core::grid::TOTAL {
            l.walls.push((10, y));
            l.walls.push((11, y));
        }
        let g = NavGrid::build(&l, &c);
        let paid = cost_field(&g, &Movement::ground(&c), (16, 21))
            .cost(26, 21)
            .expect("reachable");
        let free = cost_field(&g, &Movement { wall_cost: 0, ..Movement::ground(&c) }, (16, 21))
            .cost(26, 21)
            .expect("reachable");
        assert_eq!(paid - free, 2 * c.wall_break_cost);
    }

    #[test]
    fn wall_cost_does_not_depend_on_grid_resolution() {
        // The bug this guards: charging per cell rather than per tile makes a
        // wall cost more simply because the nav grid got finer.
        let mut c = cfg();
        let mut l = Layout::new(17);
        for y in 0..coc_core::grid::TOTAL {
            l.walls.push((10, y));
        }
        let mut paid = Vec::new();
        for sub in [2, 4] {
            c.subtiles_per_tile = sub;
            let g = NavGrid::build(&l, &c);
            let from = g.tile_centre(8, 21);
            let to = g.tile_centre(12, 21);
            let a = cost_field(&g, &Movement::ground(&c), from)
                .cost(to.0, to.1)
                .unwrap();
            let b = cost_field(&g, &Movement { wall_cost: 0, ..Movement::ground(&c) }, from)
                .cost(to.0, to.1)
                .unwrap();
            paid.push(a - b);
        }
        assert_eq!(paid[0], paid[1], "wall cost changed with the grid");
        assert_eq!(paid[0], c.wall_break_cost);
    }

    #[test]
    fn diagonal_corners_between_two_buildings_are_not_squeezed_through() {
        let c = cfg();
        let mut l = Layout::new(17);
        // Two buildings meeting at a corner, with the rest of the map open.
        l.placements.push(building(10, 10, 2, 2));
        l.placements.push(building(12, 12, 2, 2));
        let g = NavGrid::build(&l, &c);

        // The two cells diagonally across the shared corner.
        let a = (12 * g.subtiles_per_tile() - 1, 12 * g.subtiles_per_tile());
        let b = (12 * g.subtiles_per_tile(), 12 * g.subtiles_per_tile() - 1);
        assert!(g.passable(a.0, a.1) && g.passable(b.0, b.1));

        let f = cost_field(&g, &Movement::ground(&c), a);
        let direct = c.diagonal_cost() as i64;
        assert!(
            f.cost(b.0, b.1).expect("reachable the long way") > direct,
            "a unit slipped through the corner gap"
        );
    }

    #[test]
    fn an_unreachable_cell_has_no_cost() {
        let c = cfg();
        let mut l = Layout::new(17);
        // Fully enclose one tile in buildings.
        for (x, y) in [(9, 9), (10, 9), (11, 9), (9, 10), (11, 10), (9, 11), (10, 11), (11, 11)] {
            l.placements.push(building(x, y, 1, 1));
        }
        let g = NavGrid::build(&l, &c);
        let f = cost_field(&g, &Movement::ground(&c), (2, 2));
        let (tx, ty) = g.tile_centre(10, 10);
        assert_eq!(f.cost(tx, ty), None, "sealed by buildings, not walls");
        assert_eq!(f.path_to(tx, ty), None);
    }

    #[test]
    fn a_source_inside_a_building_yields_an_empty_field() {
        let c = cfg();
        let mut l = Layout::new(17);
        l.placements.push(building(10, 10, 3, 3));
        let g = NavGrid::build(&l, &c);
        let (x, y) = g.tile_centre(11, 11);
        let f = cost_field(&g, &Movement::ground(&c), (x, y));
        assert_eq!(f.cost(x, y), None);
        assert_eq!(f.cost(2, 2), None);
    }

    #[test]
    fn melee_attack_cells_hug_the_footprint() {
        let c = cfg();
        let g = NavGrid::build(&Layout::new(17), &c);
        let r = Rect::new(10, 10, 3, 3);
        // A Barbarian's range in the shipped data is well under one tile.
        let cells = attack_cells(&g, &r, 60, c.units_per_tile);
        assert!(!cells.is_empty());
        for &(x, y) in &cells {
            let d2 = dist2_to_rect(&g, (x, y), &r, c.units_per_tile);
            assert!(d2 <= sq(120), "melee cell too far: {d2}");
        }
        // Two cells clear of the building is already out of melee range.
        let far = (10 * g.subtiles_per_tile() - 3, 11 * g.subtiles_per_tile());
        assert!(!cells.contains(&far));
    }

    #[test]
    fn ranged_attack_cells_stand_well_back() {
        let c = cfg();
        let g = NavGrid::build(&Layout::new(17), &c);
        let r = Rect::new(20, 20, 3, 3);
        let melee = attack_cells(&g, &r, 60, c.units_per_tile);
        // An Archer reaches 3.5 tiles.
        let ranged = attack_cells(&g, &r, 350, c.units_per_tile);
        assert!(
            ranged.len() > melee.len() * 4,
            "range should widen the firing ring a lot: {} vs {}",
            ranged.len(),
            melee.len()
        );
        for cell in &melee {
            assert!(ranged.contains(cell), "range only ever adds positions");
        }
    }

    #[test]
    fn attack_cells_exclude_the_building_itself() {
        let c = cfg();
        let mut l = Layout::new(17);
        l.placements.push(building(20, 20, 3, 3));
        let g = NavGrid::build(&l, &c);
        let cells = attack_cells(&g, &Rect::new(20, 20, 3, 3), 350, c.units_per_tile);
        for &(x, y) in &cells {
            assert_ne!(g.at(x, y), Cell::Blocked);
        }
    }

    #[test]
    fn the_search_is_deterministic() {
        let c = cfg();
        let mut l = Layout::new(17);
        l.placements.push(building(10, 10, 4, 4));
        l.placements.push(building(16, 10, 4, 4));
        for y in 8..18 {
            l.walls.push((15, y));
        }
        let g = NavGrid::build(&l, &c);
        let first = cost_field(&g, &Movement::ground(&c), (6, 24)).path_to(50, 24);
        for _ in 0..8 {
            let again = cost_field(&g, &Movement::ground(&c), (6, 24)).path_to(50, 24);
            assert_eq!(first, again, "identical inputs must give an identical path");
        }
    }
}
