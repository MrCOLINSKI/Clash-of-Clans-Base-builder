//! The placement grid and layout representation.
//!
//! Two grids exist in this project and they are not interchangeable. This is
//! the **placement grid**: integer tiles, used by the editor, the legality
//! rules and the optimizer. The navigation grid used for pathing is finer
//! (subtiles) and lives in `coc-sim`.

use serde::{Deserialize, Serialize};

/// Total playfield extent in tiles, including the unbuildable border.
///
/// NOT from the game data — all 500 keys in `globals.csv` were searched and
/// contain no map dimension, and a base link is a 24-byte identifier with no
/// coordinates in it. See ASSUMPTIONS.md 3.1. Kept here as a single constant
/// so a better source replaces it in one place.
pub const TOTAL: i32 = 44;
/// Buildable interior extent in tiles.
pub const BUILDABLE: i32 = 40;
/// Offset of the buildable interior within the playfield.
pub const ORIGIN: i32 = 2;

/// An axis-aligned tile rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }

    /// Centre in tile coordinates, in halves to stay integral.
    ///
    /// Returned doubled so an even-sided building keeps an exact centre
    /// without introducing floats into the placement layer.
    pub fn centre2(&self) -> (i32, i32) {
        (2 * self.x + self.w, 2 * self.y + self.h)
    }

    pub fn overlaps(&self, other: &Rect) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }

    /// Whether the rectangle lies wholly inside the buildable interior.
    pub fn inside_buildable(&self) -> bool {
        self.x >= ORIGIN
            && self.y >= ORIGIN
            && self.right() <= ORIGIN + BUILDABLE
            && self.bottom() <= ORIGIN + BUILDABLE
    }

    pub fn tiles(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        (self.y..self.bottom()).flat_map(move |y| (self.x..self.right()).map(move |x| (x, y)))
    }
}

/// One placed structure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placement {
    pub name: String,
    /// Level this structure sits at, resolved for the layout's town hall.
    pub level: u32,
    pub rect: Rect,
    /// `BuildingClass` from the game data.
    pub class: String,
    /// Attack range in game distance units, zero for non-defences.
    pub range: i32,
    /// Minimum attack range in game distance units.
    pub min_range: i32,
    /// True for traps, which occupy a tile but are not destructible.
    pub is_trap: bool,
}

impl Placement {
    pub fn is_defense(&self) -> bool {
        self.class == "Defense" && self.range > 0
    }

    pub fn is_town_hall(&self) -> bool {
        self.name == "Town Hall"
    }

    /// Whether this holds loot worth protecting.
    pub fn is_storage(&self) -> bool {
        self.class == "Resource"
    }
}

/// A complete base layout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layout {
    pub townhall: u32,
    pub placements: Vec<Placement>,
    /// Wall tiles. Stored separately because walls are 1x1 and numerous.
    pub walls: Vec<(i32, i32)>,
    /// Wall level for this town hall.
    pub wall_level: u32,
}

impl Layout {
    pub fn new(townhall: u32) -> Layout {
        Layout {
            townhall,
            placements: Vec::new(),
            walls: Vec::new(),
            wall_level: 0,
        }
    }

    pub fn town_hall(&self) -> Option<&Placement> {
        self.placements.iter().find(|p| p.is_town_hall())
    }

    pub fn defenses(&self) -> impl Iterator<Item = &Placement> {
        self.placements.iter().filter(|p| p.is_defense())
    }

    /// Occupancy map over the whole playfield.
    ///
    /// 0 empty, 1 structure, 2 wall.
    pub fn occupancy(&self) -> Vec<u8> {
        let mut g = vec![0u8; (TOTAL * TOTAL) as usize];
        for p in &self.placements {
            for (x, y) in p.rect.tiles() {
                if let Some(c) = idx(x, y) {
                    g[c] = 1;
                }
            }
        }
        for &(x, y) in &self.walls {
            if let Some(c) = idx(x, y) {
                if g[c] == 0 {
                    g[c] = 2;
                }
            }
        }
        g
    }
}

/// Flat index into a playfield-sized buffer, or `None` when out of bounds.
pub fn idx(x: i32, y: i32) -> Option<usize> {
    if x < 0 || y < 0 || x >= TOTAL || y >= TOTAL {
        None
    } else {
        Some((y * TOTAL + x) as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlap_is_exclusive_of_touching_edges() {
        let a = Rect::new(0, 0, 3, 3);
        assert!(a.overlaps(&Rect::new(2, 2, 3, 3)), "sharing tiles overlaps");
        assert!(!a.overlaps(&Rect::new(3, 0, 3, 3)), "touching edges do not");
        assert!(!a.overlaps(&Rect::new(0, 3, 3, 3)));
    }

    #[test]
    fn buildable_bounds_exclude_the_border() {
        assert!(Rect::new(ORIGIN, ORIGIN, 3, 3).inside_buildable());
        assert!(!Rect::new(ORIGIN - 1, ORIGIN, 3, 3).inside_buildable());
        let last = ORIGIN + BUILDABLE - 3;
        assert!(Rect::new(last, last, 3, 3).inside_buildable());
        assert!(!Rect::new(last + 1, last, 3, 3).inside_buildable());
    }

    #[test]
    fn centre_of_even_and_odd_footprints_stays_integral() {
        // Doubled centres avoid floats: a 4x4 at (0,0) centres on (2,2).
        assert_eq!(Rect::new(0, 0, 4, 4).centre2(), (4, 4));
        // A 3x3 at (0,0) centres on (1.5, 1.5), i.e. (3,3) doubled.
        assert_eq!(Rect::new(0, 0, 3, 3).centre2(), (3, 3));
    }

    #[test]
    fn rect_enumerates_exactly_its_tiles() {
        let r = Rect::new(1, 2, 2, 3);
        let t: Vec<_> = r.tiles().collect();
        assert_eq!(t.len(), 6);
        assert!(t.contains(&(1, 2)) && t.contains(&(2, 4)));
        assert!(!t.contains(&(3, 2)));
    }
}
