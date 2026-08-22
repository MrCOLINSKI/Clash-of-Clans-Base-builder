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
    let _ = &thl;
    layout.wall_level = level_for(data, "Wall", th).unwrap_or(0);
    // A real base is a lattice of closed compartments, not concentric rings.
    // Rings with gaps punched in them leave one big open interior, which both
    // looks wrong and scores badly on enclosure, because a troop can walk in.
    //
    // The lattice is sized to spend the wall budget: pick the cell size and
    // line count whose wall tile count comes closest without exceeding it.
    let (walls, cells) = wall_lattice(wall_budget);
    for &(x, y) in &walls {
        if let Some(c) = idx(x, y) {
            occ[c] = 2;
        }
    }
    layout.walls = walls;

    // Structures, most important first so the centre goes to what matters.
    // Home village only. townhall_levels.csv covers both villages, so without
    // this a TH17 base picks up 255 Builder Base structures including 180 BB
    // Walls, which is what produced 359 structures instead of 156.
    let mut names: Vec<(String, i64)> = data
        .home_counts(th)
        .into_iter()
        .filter(|(n, _)| *n != "Wall")
        .map(|(n, c)| (n.to_string(), c))
        .collect();
    // Within a priority tier, larger footprints go down first. Placing them
    // last starved the 4x4s: at TH14+ the Siege Workshop and a fourth Army
    // Camp had nowhere left to fit once the 3x3s had packed the space.
    names.sort_by_key(|(n, _)| {
        let (w, h, ..) = dims(data, n);
        (priority(n), std::cmp::Reverse(w * h), n.clone())
    });

    for (name, count) in &names {
        let name = name.as_str();
        let Some(level) = level_for(data, name, th) else {
            continue;
        };
        let (w, h, class, range, min_range, is_trap) = dims(data, name);
        if w == 0 {
            continue;
        }
        for _ in 0..*count {
            let rect = place_in_cells(&occ, &cells, w, h, priority(name))
                .or_else(|| find_spot(&occ, w, h, ring_for(name), &mut rng));
            if let Some(rect) = rect {
                for (x, y) in rect.tiles() {
                    if let Some(c) = idx(x, y) {
                        occ[c] = 1;
                    }
                }
                layout.placements.push(Placement {
                    name: name.to_string(),
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

/// Finds room inside the wall compartments, working outward from the centre.
///
/// `rank` biases where a structure lands: the town hall and heavy defences
/// take the core cells, everything else fills outward. Falling back to the
/// open ground outside the lattice is the caller's job.
fn place_in_cells(occ: &[u8], cells: &[Rect], w: i32, h: i32, rank: u8) -> Option<Rect> {
    // Low-rank structures start at the centre; high-rank ones skip past the
    // core so they do not squat in the compartments the defences need.
    let skip = match rank {
        0 => 0,
        1 => 0,
        2 => 1,
        3 => 2,
        _ => cells.len() / 3,
    };
    for cell in cells.iter().skip(skip.min(cells.len())) {
        for y in cell.y..cell.bottom().saturating_sub(h - 1) {
            for x in cell.x..cell.right().saturating_sub(w - 1) {
                let r = Rect::new(x, y, w, h);
                if crate::legality::can_place(&r, occ) {
                    return Some(r);
                }
            }
        }
    }
    None
}

/// Builds a lattice of closed wall compartments, returning the wall tiles and
/// the interior rectangle of each cell.
///
/// Cell size and extent are chosen so the wall count lands as close under the
/// budget as possible: a fixed lattice either overspends and gets truncated
/// mid-line, leaving a gap that breaks every compartment, or underspends and
/// leaves the base open.
fn wall_lattice(budget: usize) -> (Vec<(i32, i32)>, Vec<Rect>) {
    if budget < 24 {
        return (Vec::new(), vec![Rect::new(ORIGIN, ORIGIN, BUILDABLE, BUILDABLE)]);
    }
    // Search for the lattice that yields the most *usable interior*, not the
    // one that burns the most wall. Maximising wall tiles picks many tiny
    // compartments: the budget is spent but almost nothing fits inside, and
    // the buildings end up outside the walls.
    //
    // Cells are at least 6 wide so a 4x4 still fits in one.
    let mut best: Option<(i32, i32, i32)> = None;
    for cell in 6..=9i32 {
        for lines in 2..=7i32 {
            let span = (lines - 1) * cell;
            if span + 1 > BUILDABLE - 2 {
                continue;
            }
            let tiles = (2 * lines * (span + 1) - lines * lines) as usize;
            if tiles > budget {
                continue;
            }
            let interior = (lines - 1) * (lines - 1) * (cell - 1) * (cell - 1);
            if best.is_none_or(|(_, _, area)| interior > area) {
                best = Some((cell, lines, interior));
            }
        }
    }
    let Some((cell, lines, _)) = best else {
        return (Vec::new(), vec![Rect::new(ORIGIN, ORIGIN, BUILDABLE, BUILDABLE)]);
    };

    let span = (lines - 1) * cell;
    let x0 = ORIGIN + (BUILDABLE - span) / 2;
    let y0 = x0;
    let mut walls = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for i in 0..lines {
        let cx = x0 + i * cell;
        let cy = y0 + i * cell;
        for t in 0..=span {
            for p in [(cx, y0 + t), (x0 + t, cy)] {
                if seen.insert(p) {
                    walls.push(p);
                }
            }
        }
    }

    // Interiors are the open squares between the lines.
    let mut cells = Vec::new();
    for r in 0..lines - 1 {
        for c in 0..lines - 1 {
            cells.push(Rect::new(
                x0 + c * cell + 1,
                y0 + r * cell + 1,
                cell - 1,
                cell - 1,
            ));
        }
    }
    // Centre-first, so the town hall and the heavy defences claim the core.
    let mid = (lines - 1) as f64 / 2.0 - 0.5;
    cells.sort_by(|a, b| {
        let d = |r: &Rect| {
            let cx = (r.x - x0) as f64 / cell as f64;
            let cy = (r.y - y0) as f64 / cell as f64;
            ((cx - mid).powi(2) + (cy - mid).powi(2)).sqrt()
        };
        d(a).partial_cmp(&d(b)).unwrap_or(std::cmp::Ordering::Equal)
    });
    (walls, cells)
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
    fn wall_compartments_actually_enclose() {
        let Ok(d) = GameData::load_default() else { return };
        // A lattice of closed cells should hold most of the base inside it.
        // Concentric rings with gaps scored around 0.3 here, because a troop
        // could walk straight in through a gap.
        for th in [10, 14, 17] {
            let l = seed(&d, th, 42);
            let m = crate::metrics::evaluate(&l, &crate::Weights::war());
            assert!(
                m.enclosed > 0.5,
                "TH{th} encloses only {:.2} of its structures",
                m.enclosed
            );
        }
    }

    #[test]
    fn every_permitted_structure_finds_a_place() {
        let Ok(d) = GameData::load_default() else { return };
        for th in 1..=d.max_townhall() {
            let l = seed(&d, th, 42);
            for (name, allowed) in d.home_counts(th) {
                if name == "Wall" || level_for(&d, name, th).is_none() {
                    continue;
                }
                let placed = l.placements.iter().filter(|p| p.name == name).count() as i64;
                assert_eq!(
                    placed, allowed,
                    "TH{th}: {name} placed {placed} of {allowed}; the grid ran out of room"
                );
            }
        }
    }

    #[test]
    fn builder_base_structures_never_enter_a_home_village() {
        let Ok(d) = GameData::load_default() else { return };
        let l = seed(&d, 17, 7);
        let bb: Vec<&str> = l
            .placements
            .iter()
            .map(|p| p.name.as_str())
            .filter(|n| !d.is_home_village(n))
            .collect();
        assert!(bb.is_empty(), "builder base structures placed: {bb:?}");
        // TH17 home village is around 150 structures; 359 meant both villages.
        assert!(
            (100..=200).contains(&l.placements.len()),
            "TH17 should hold roughly 150 structures, got {}",
            l.placements.len()
        );
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
