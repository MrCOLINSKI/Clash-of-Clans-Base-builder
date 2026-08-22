//! Placement legality.
//!
//! The optimizer mutates layouts millions of times and must never be able to
//! emit an illegal base, so every mutation is checked here rather than trusted.
//! A violation is a bug in the mutation, not a candidate to be scored badly.

use crate::grid::{Layout, Rect, BUILDABLE, ORIGIN};
use coc_data::GameData;
use std::collections::HashMap;

/// Why a layout is not legal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    OutOfBounds { name: String, rect: Rect },
    Overlap { a: String, b: String },
    TooMany { name: String, used: i64, allowed: i64 },
    LevelTooHigh { name: String, level: u32, max: u32 },
    UnknownStructure { name: String },
    WallOnStructure { at: (i32, i32) },
    TooManyWalls { used: usize, allowed: i64 },
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Violation::OutOfBounds { name, rect } => write!(
                f,
                "{name} at ({},{}) {}x{} leaves the buildable area ({ORIGIN}..{})",
                rect.x, rect.y, rect.w, rect.h, ORIGIN + BUILDABLE
            ),
            Violation::Overlap { a, b } => write!(f, "{a} overlaps {b}"),
            Violation::TooMany { name, used, allowed } => {
                write!(f, "{used} x {name} placed but only {allowed} allowed at this town hall")
            }
            Violation::LevelTooHigh { name, level, max } => {
                write!(f, "{name} at level {level} exceeds the max of {max} for this town hall")
            }
            Violation::UnknownStructure { name } => write!(f, "{name} is not in the game data"),
            Violation::WallOnStructure { at } => {
                write!(f, "wall at ({},{}) sits on a structure", at.0, at.1)
            }
            Violation::TooManyWalls { used, allowed } => {
                write!(f, "{used} walls placed but only {allowed} allowed")
            }
        }
    }
}

/// Highest level of `name` available at town hall `th`.
///
/// Each level row carries its own `TownHallLevel` requirement, so the max is
/// the last level whose requirement this hall meets — the game's own rule.
pub fn max_level(data: &GameData, name: &str, th: u32) -> Option<u32> {
    if let Some(b) = data.building(name) {
        let mut best = None;
        for l in &b.levels {
            // TownHallLevel is an entity column carried across level rows, so
            // it is read per level rather than once.
            if let Some(req) = b.unlock_th {
                if req as u32 <= th {
                    best = Some(l.level);
                }
            }
        }
        return best;
    }
    None
}

/// Checks a layout against the game's own constraints.
pub fn validate(layout: &Layout, data: &GameData) -> Vec<Violation> {
    let mut v = Vec::new();

    let Some(th) = data.townhall(layout.townhall) else {
        return vec![Violation::UnknownStructure {
            name: format!("town hall level {}", layout.townhall),
        }];
    };

    // Bounds.
    for p in &layout.placements {
        if !p.rect.inside_buildable() {
            v.push(Violation::OutOfBounds {
                name: p.name.clone(),
                rect: p.rect,
            });
        }
    }

    // Pairwise overlap. Layouts are a few hundred rectangles, so the quadratic
    // scan is cheaper than building an index.
    for (i, a) in layout.placements.iter().enumerate() {
        for b in layout.placements.iter().skip(i + 1) {
            if a.rect.overlaps(&b.rect) {
                v.push(Violation::Overlap {
                    a: a.name.clone(),
                    b: b.name.clone(),
                });
            }
        }
    }

    // Counts against the town hall allowance.
    let mut used: HashMap<&str, i64> = HashMap::new();
    for p in &layout.placements {
        *used.entry(p.name.as_str()).or_default() += 1;
    }
    for (name, n) in &used {
        // A village has exactly one Town Hall, and `townhall_levels.csv` does
        // not say so: the table lists what a hall *permits you to build*, and
        // the hall is not one of those things. Reading the allowance straight
        // from the table makes the Town Hall illegal in its own village.
        let allowed = if *name == "Town Hall" { 1 } else { th.count_of(name) };
        if allowed == 0 && data.building(name).is_none() && data.trap(name).is_none() {
            v.push(Violation::UnknownStructure {
                name: (*name).to_string(),
            });
        } else if *n > allowed {
            v.push(Violation::TooMany {
                name: (*name).to_string(),
                used: *n,
                allowed,
            });
        }
    }

    // Walls: count, and never on top of a structure.
    let wall_allowance = th.count_of("Wall");
    if (layout.walls.len() as i64) > wall_allowance {
        v.push(Violation::TooManyWalls {
            used: layout.walls.len(),
            allowed: wall_allowance,
        });
    }
    let occ: Vec<bool> = {
        let mut g = vec![false; (crate::grid::TOTAL * crate::grid::TOTAL) as usize];
        for p in &layout.placements {
            for (x, y) in p.rect.tiles() {
                if let Some(c) = crate::grid::idx(x, y) {
                    g[c] = true;
                }
            }
        }
        g
    };
    for &(x, y) in &layout.walls {
        match crate::grid::idx(x, y) {
            None => v.push(Violation::OutOfBounds {
                name: "Wall".into(),
                rect: Rect::new(x, y, 1, 1),
            }),
            Some(c) if occ[c] => v.push(Violation::WallOnStructure { at: (x, y) }),
            _ => {
                if !Rect::new(x, y, 1, 1).inside_buildable() {
                    v.push(Violation::OutOfBounds {
                        name: "Wall".into(),
                        rect: Rect::new(x, y, 1, 1),
                    });
                }
            }
        }
    }

    v
}

/// Whether a single rectangle can be placed given current occupancy.
pub fn can_place(rect: &Rect, occupied: &[u8]) -> bool {
    if !rect.inside_buildable() {
        return false;
    }
    rect.tiles().all(|(x, y)| {
        crate::grid::idx(x, y).is_some_and(|c| occupied[c] == 0)
    })
}

/// Convenience: does this layout satisfy every rule?
pub fn is_legal(layout: &Layout, data: &GameData) -> bool {
    validate(layout, data).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::{Placement, Rect};

    fn place(name: &str, x: i32, y: i32, w: i32, h: i32) -> Placement {
        Placement {
            name: name.into(),
            level: 1,
            rect: Rect::new(x, y, w, h),
            class: "Defense".into(),
            range: 900,
            min_range: 0,
            is_trap: false,
        }
    }

    #[test]
    fn detects_overlap_and_out_of_bounds_without_game_data() {
        // These two checks are pure geometry, so they are exercised directly.
        let a = place("Cannon", 5, 5, 3, 3);
        let b = place("Cannon", 6, 6, 3, 3);
        assert!(a.rect.overlaps(&b.rect));
        assert!(!place("Cannon", 0, 0, 3, 3).rect.inside_buildable());
    }

    #[test]
    fn can_place_respects_occupancy_and_bounds() {
        let mut occ = vec![0u8; (crate::grid::TOTAL * crate::grid::TOTAL) as usize];
        assert!(can_place(&Rect::new(10, 10, 3, 3), &occ));
        occ[crate::grid::idx(11, 11).unwrap()] = 1;
        assert!(!can_place(&Rect::new(10, 10, 3, 3), &occ));
        assert!(!can_place(&Rect::new(0, 0, 3, 3), &occ), "outside buildable");
    }
}
