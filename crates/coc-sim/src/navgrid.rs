//! The navigation grid.
//!
//! Distinct from the placement grid in `coc-core`, which is whole tiles. Ground
//! units path at **subtile** resolution because buildings do not fill their
//! footprint edge to edge — there is a gap around most hitboxes, and troops
//! walk through it. At one cell per tile those channels do not exist and
//! pathing is wrong in a way that stays invisible until you compare against a
//! replay.
//!
//! The resolution is not a constant here. It comes from
//! `config/pathing.toml`, like every other pathing number, so an experiment
//! that wants quarter-tile cells changes the config and nothing else.
//!
//! Costs are integers throughout. The simulator must produce identical results
//! on any machine, so no floating point reaches this layer: distance is in game
//! units (100 per tile) and the diagonal step uses a fixed-point sqrt(2).

use crate::config::PathingConfig;
use coc_core::grid::{Layout, TOTAL};

/// What occupies a subtile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cell {
    /// Walkable ground.
    Open,
    /// A wall: traversable, but only by paying the unit's wall cost.
    Wall,
    /// A structure: ground units path around it, never through.
    Blocked,
}

/// A navigation grid derived from a layout.
#[derive(Debug, Clone)]
pub struct NavGrid {
    cells: Vec<Cell>,
    /// Subtiles per tile along each axis.
    sub: i32,
    /// Grid extent in subtiles.
    nav: i32,
    step: i32,
    diag: i32,
}

impl NavGrid {
    /// Builds the grid for a layout at the configured resolution.
    ///
    /// Buildings block their footprint; walls are marked traversable-at-a-cost
    /// rather than blocked, which is what makes the wall cost meaningful at
    /// all — a blocked wall would be an infinite cost and the whole two-stage
    /// selection would collapse to "whatever is inside my compartment".
    pub fn build(layout: &Layout, cfg: &PathingConfig) -> NavGrid {
        let sub = cfg.subtiles_per_tile;
        let nav = cfg.nav_extent(TOTAL);
        let mut g = NavGrid {
            cells: vec![Cell::Open; (nav * nav) as usize],
            sub,
            nav,
            step: cfg.step_cost(),
            diag: cfg.diagonal_cost(),
        };
        for p in &layout.placements {
            // Traps do not obstruct movement; a unit walks over them, which is
            // the entire point of a trap.
            if p.is_trap {
                continue;
            }
            for (tx, ty) in p.rect.tiles() {
                g.mark(tx, ty, Cell::Blocked);
            }
        }
        for &(x, y) in &layout.walls {
            g.mark(x, y, Cell::Wall);
        }
        g
    }

    pub fn subtiles_per_tile(&self) -> i32 {
        self.sub
    }

    /// Grid extent in subtiles, along each axis.
    pub fn extent(&self) -> i32 {
        self.nav
    }

    pub fn step_cost(&self) -> i32 {
        self.step
    }

    pub fn diagonal_cost(&self) -> i32 {
        self.diag
    }

    pub fn at(&self, x: i32, y: i32) -> Cell {
        if !self.in_bounds(x, y) {
            return Cell::Blocked;
        }
        self.cells[(y * self.nav + x) as usize]
    }

    pub fn in_bounds(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && x < self.nav && y < self.nav
    }

    pub fn index(&self, x: i32, y: i32) -> Option<usize> {
        self.in_bounds(x, y)
            .then(|| (y * self.nav + x) as usize)
    }

    /// Whether a subtile can ever be entered by a ground unit, at any cost.
    pub fn passable(&self, x: i32, y: i32) -> bool {
        !matches!(self.at(x, y), Cell::Blocked)
    }

    /// The subtile at the centre of a tile.
    pub fn tile_centre(&self, tx: i32, ty: i32) -> (i32, i32) {
        (tx * self.sub + self.sub / 2, ty * self.sub + self.sub / 2)
    }

    /// The tile a subtile belongs to.
    pub fn to_tile(&self, sx: i32, sy: i32) -> (i32, i32) {
        (sx.div_euclid(self.sub), sy.div_euclid(self.sub))
    }

    /// Subtile coordinate in game distance units, taken at the cell's centre.
    ///
    /// Doubled, so a cell whose centre falls on a half-unit stays integral.
    pub fn position2(&self, sx: i32, sy: i32) -> (i32, i32) {
        (sx * self.step * 2 + self.step, sy * self.step * 2 + self.step)
    }

    /// Marks every subtile of one tile, if it lies on the grid.
    ///
    /// A structure placed over a wall wins: it is the harder obstacle, and a
    /// cell that is both would otherwise let a unit path through a building by
    /// paying the wall cost.
    fn mark(&mut self, tx: i32, ty: i32, what: Cell) {
        for sy in ty * self.sub..(ty + 1) * self.sub {
            for sx in tx * self.sub..(tx + 1) * self.sub {
                let Some(i) = self.index(sx, sy) else { continue };
                if what == Cell::Blocked || self.cells[i] == Cell::Open {
                    self.cells[i] = what;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{test_globals, PathingConfig};
    use coc_core::grid::{Placement, Rect};

    pub(crate) fn cfg() -> PathingConfig {
        PathingConfig::load_default(&test_globals()).expect("shipped config loads")
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
        }
    }

    #[test]
    fn resolution_is_finer_than_a_tile() {
        let c = cfg();
        let g = NavGrid::build(&Layout::new(17), &c);
        assert!(
            g.subtiles_per_tile() >= 2,
            "half-tile channels need at least two cells per tile"
        );
        assert_eq!(g.extent(), TOTAL * c.subtiles_per_tile);
        assert_eq!(g.step_cost(), 50);
    }

    #[test]
    fn buildings_block_and_walls_do_not() {
        let mut l = Layout::new(17);
        l.placements.push(building(10, 10, 3, 3));
        l.walls.push((20, 20));
        let g = NavGrid::build(&l, &cfg());

        let (bx, by) = g.tile_centre(11, 11);
        assert_eq!(g.at(bx, by), Cell::Blocked);
        assert!(!g.passable(bx, by));

        let (wx, wy) = g.tile_centre(20, 20);
        assert_eq!(g.at(wx, wy), Cell::Wall);
        assert!(g.passable(wx, wy), "walls are traversable at a cost");

        assert_eq!(g.at(g.tile_centre(30, 30).0, g.tile_centre(30, 30).1), Cell::Open);
    }

    #[test]
    fn traps_do_not_obstruct_movement() {
        let mut l = Layout::new(17);
        let mut t = building(15, 15, 1, 1);
        t.is_trap = true;
        t.name = "Bomb".into();
        l.placements.push(t);
        let g = NavGrid::build(&l, &cfg());
        let (x, y) = g.tile_centre(15, 15);
        assert_eq!(g.at(x, y), Cell::Open, "a unit walks over a trap");
    }

    #[test]
    fn a_structure_over_a_wall_blocks() {
        let mut l = Layout::new(17);
        l.placements.push(building(8, 8, 2, 2));
        l.walls.push((8, 8));
        let g = NavGrid::build(&l, &cfg());
        let (x, y) = g.tile_centre(8, 8);
        assert_eq!(g.at(x, y), Cell::Blocked);
    }

    #[test]
    fn tile_and_subtile_conversions_round_trip() {
        let g = NavGrid::build(&Layout::new(17), &cfg());
        for t in [0, 1, 17, 43] {
            let (sx, sy) = g.tile_centre(t, t);
            assert_eq!(g.to_tile(sx, sy), (t, t));
        }
    }

    #[test]
    fn out_of_bounds_reads_as_blocked() {
        let g = NavGrid::build(&Layout::new(17), &cfg());
        assert_eq!(g.at(-1, 0), Cell::Blocked);
        assert_eq!(g.at(0, g.extent()), Cell::Blocked);
        assert_eq!(g.index(-1, 0), None);
    }
}
