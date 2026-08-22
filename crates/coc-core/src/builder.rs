//! Seeding legal layouts from game data.
//!
//! The optimizer needs a legal starting point with the right structures at the
//! right levels. This produces one; making it *good* is the optimizer's job.

use crate::grid::{idx, Layout, Placement, Rect, BUILDABLE, ORIGIN, TOTAL};
use coc_data::GameData;
use rand::Rng;
use rand_chacha::rand_core::SeedableRng;
use rand_chacha::ChaCha8Rng;

/// Resolves the level a structure sits at for a given town hall.
///
/// Each level row carries its own `TownHallLevel` requirement, so the answer is
/// the last level whose requirement this hall meets. Returns `None` when the
/// structure is not unlocked at all.
pub fn level_for(data: &GameData, name: &str, th: u32) -> Option<u32> {
    let table = data.tables.get("buildings").or_else(|| data.tables.get("traps"))?;
    let _ = table;
    // Levels come from the typed model; the requirement column is per level.
    if let Some(b) = data.building(name) {
        return highest(b.levels.iter().map(|l| (l.level, req_th(data, name, l.level))), th);
    }
    if let Some(t) = data.trap(name) {
        return highest(t.levels.iter().map(|l| (l.level, req_th(data, name, l.level))), th);
    }
    None
}

/// Town hall required for a specific level of a structure.
fn req_th(data: &GameData, name: &str, level: u32) -> u32 {
    for table in ["buildings", "traps"] {
        let Some(t) = data.tables.get(table) else { continue };
        let Some(e) = t.entity(name) else { continue };
        let Some(row) = e.level(level as usize) else { continue };
        if let Ok(Some(v)) = row.get_i64("TownHallLevel") {
            return v as u32;
        }
    }
    u32::MAX
}

fn highest(levels: impl Iterator<Item = (u32, u32)>, th: u32) -> Option<u32> {
    let mut best = None;
    for (level, req) in levels {
        if req <= th {
            best = Some(level);
        }
    }
    best
}

/// Builds a legal, seeded layout for a town hall level.
pub fn seed(data: &GameData, th: u32, seed: u64) -> Layout {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut layout = Layout::new(th);
    let mut occ = vec![0u8; (TOTAL * TOTAL) as usize];

    let Some(thl) = data.townhall(th) else {
        return layout;
    };

    // Walls first: they define the compartments everything else fits around,
    // and they must be reserved before structures claim the tiles.
    let wall_budget = thl.count_of("Wall").max(0) as usize;
    layout.wall_level = level_for(data, "Wall", th).unwrap_or(0);
    let mut walls = Vec::new();
    let rings: &[(i32, i32)] = if wall_budget > 250 {
        &[(15, 28), (10, 33), (6, 37)]
    } else if wall_budget > 150 {
        &[(14, 29), (9, 34)]
    } else if wall_budget > 60 {
        &[(15, 28), (11, 32)]
    } else if wall_budget > 0 {
        &[(16, 27)]
    } else {
        &[]
    };
    'outer: for &(a, b) in rings {
        for x in a..=b {
            for y in [a, b] {
                if walls.len() >= wall_budget {
                    break 'outer;
                }
                // Leave gaps so compartments have doors, as real bases do.
                if x % 13 != 0 {
                    walls.push((x, y));
                }
            }
        }
        for y in a + 1..b {
            for x in [a, b] {
                if walls.len() >= wall_budget {
                    break 'outer;
                }
                if y % 13 != 0 {
                    walls.push((x, y));
                }
            }
        }
    }
    for &(x, y) in &walls {
        if let Some(c) = idx(x, y) {
            occ[c] = 2;
        }
    }
    layout.walls = walls;

    // Structures, most important first so the centre goes to what matters.
    let mut names: Vec<(&String, i64)> = thl
        .building_counts
        .iter()
        .filter(|(n, c)| **c > 0 && n.as_str() != "Wall")
        .map(|(n, c)| (n, *c))
        .collect();
    names.sort_by_key(|(n, _)| (priority(n), (*n).clone()));

    for (name, count) in names {
        let Some(level) = level_for(data, name, th) else {
            continue;
        };
        let (w, h, class, range, min_range, is_trap) = dims(data, name);
        if w == 0 {
            continue;
        }
        for _ in 0..count {
            let target = ring_for(name);
            if let Some(rect) = find_spot(&occ, w, h, target, &mut rng) {
                for (x, y) in rect.tiles() {
                    if let Some(c) = idx(x, y) {
                        occ[c] = 1;
                    }
                }
                layout.placements.push(Placement {
                    name: name.clone(),
                    level,
                    rect,
                    class: class.clone(),
                    range,
                    min_range,
                    is_trap,
                });
            }
        }
    }
    layout
}

fn dims(data: &GameData, name: &str) -> (i32, i32, String, i32, i32, bool) {
    if let Some(b) = data.building(name) {
        let lv = b.levels.first();
        return (
            b.width,
            b.height,
            b.class.clone(),
            lv.and_then(|l| l.attack_range).unwrap_or(0),
            lv.and_then(|l| l.min_attack_range).unwrap_or(0),
            false,
        );
    }
    if let Some(t) = data.trap(name) {
        return (t.width, t.height, "Trap".into(), 0, 0, true);
    }
    (0, 0, String::new(), 0, 0, false)
}

/// Placement priority: lower goes down first, closer to the centre.
fn priority(name: &str) -> u8 {
    match name {
        "Town Hall" => 0,
        "Eagle Artillery" | "Hero Hall" | "Clan Castle" | "Scattershot" | "Monolith" => 1,
        "Inferno Tower" | "X-Bow" | "Spell Tower" | "Multi Gear Tower" => 2,
        "Dark Elixir Storage" | "Gold Storage" | "Elixir Storage" => 3,
        _ => 4,
    }
}

/// Preferred distance from centre, in tiles.
fn ring_for(name: &str) -> f64 {
    match name {
        "Town Hall" => 0.0,
        "Eagle Artillery" | "Hero Hall" | "Clan Castle" => 4.0,
        "Scattershot" | "Inferno Tower" | "Monolith" | "X-Bow" => 6.0,
        "Gold Storage" | "Elixir Storage" | "Dark Elixir Storage" => 7.0,
        "Wizard Tower" | "Air Defense" | "Mortar" | "Bomb Tower" => 9.0,
        "Cannon" | "Archer Tower" | "Hidden Tesla" => 12.0,
        "Gold Mine" | "Elixir Collector" | "Builders Hut" | "Army Camp" => 16.0,
        _ => 13.0,
    }
}

/// Finds a free rectangle near a target radius from the centre.
fn find_spot(occ: &[u8], w: i32, h: i32, radius: f64, rng: &mut ChaCha8Rng) -> Option<Rect> {
    let c = TOTAL as f64 / 2.0;
    let start = rng.gen_range(0.0..std::f64::consts::TAU);
    for grow in 0..26 {
        let r = radius + grow as f64;
        let steps = (12 + grow * 6).max(1);
        for s in 0..steps {
            let t = start + (s as f64 / steps as f64) * std::f64::consts::TAU;
            let x = (c + t.cos() * r).round() as i32 - w / 2;
            let y = (c + t.sin() * r).round() as i32 - h / 2;
            let rect = Rect::new(x, y, w, h);
            if crate::legality::can_place(&rect, occ) {
                return Some(rect);
            }
        }
    }
    // Last resort: scan the whole buildable area.
    for y in ORIGIN..ORIGIN + BUILDABLE - h {
        for x in ORIGIN..ORIGIN + BUILDABLE - w {
            let rect = Rect::new(x, y, w, h);
            if crate::legality::can_place(&rect, occ) {
                return Some(rect);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_are_deterministic() {
        let Ok(d) = GameData::load_default() else { return };
        let a = seed(&d, 12, 42);
        let b = seed(&d, 12, 42);
        assert_eq!(a, b, "same seed must produce the same layout");
        let c = seed(&d, 12, 43);
        assert_ne!(a, c, "a different seed should differ");
    }

    #[test]
    fn seeded_layouts_are_legal() {
        let Ok(d) = GameData::load_default() else { return };
        for th in [3, 8, 12, 17] {
            let l = seed(&d, th, 7);
            let v = crate::legality::validate(&l, &d);
            assert!(
                v.is_empty(),
                "TH{th} seed illegal: {:?}",
                v.iter().take(5).map(|x| x.to_string()).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn levels_match_the_town_hall() {
        let Ok(d) = GameData::load_default() else { return };
        // At TH4 a Cannon is level 5 and an X-Bow does not exist.
        assert_eq!(level_for(&d, "Cannon", 4), Some(5));
        assert_eq!(level_for(&d, "Archer Tower", 4), Some(4));
        assert_eq!(level_for(&d, "Wall", 4), Some(4));
        assert_eq!(level_for(&d, "X-Bow", 4), None);
        assert_eq!(level_for(&d, "Cannon", 12), Some(17));
    }
}
