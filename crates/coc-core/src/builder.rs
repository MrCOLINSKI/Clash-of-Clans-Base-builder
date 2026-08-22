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
    // The Town Hall's level *is* the town hall level — that is what "TH12"
    // means. Running it through the requirement column instead gives level 2 at
    // TH1, because the column answers "what hall do you need to build this",
    // which for the hall itself is not a question with a useful answer.
    if name == "Town Hall" {
        let max = data.building(name)?.levels.iter().map(|l| l.level).max()?;
        return Some(th.min(max));
    }
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

    let wall_budget = thl.count_of("Wall").max(0) as usize;
    layout.wall_level = level_for(data, "Wall", th).unwrap_or(0);

    // Home village only. townhall_levels.csv covers both villages, so without
    // this a TH17 base picks up 255 Builder Base structures including 180 BB
    // Walls, which is what produced 359 structures instead of 156.
    //
    // The Town Hall is prepended rather than read from the table, because
    // `townhall_levels.csv` lists what a town hall *permits you to build* and
    // the hall is not in its own list. Reading the table alone produced bases
    // with no Town Hall at all — legal by every check, and the single most
    // important building in the game missing from all of them.
    let mut names: Vec<(String, i64)> = vec![("Town Hall".to_string(), 1)];
    names.extend(
        data.home_counts(th)
            .into_iter()
            .filter(|(n, _)| *n != "Wall" && *n != "Town Hall")
            .map(|(n, c)| (n.to_string(), c)),
    );
    // Within a priority tier, larger footprints go down first. Placing them
    // last starved the 4x4s: at TH14+ the Siege Workshop and a fourth Army
    // Camp had nowhere left to fit once the 3x3s had packed the space.
    names.sort_by_key(|(n, _)| {
        let (w, h, ..) = dims(data, n);
        (priority(n), std::cmp::Reverse(w * h), n.clone())
    });

    // Walls define the compartments everything else fits around, so they are
    // reserved before any structure claims a tile. Crucially the lattice is
    // sized to what actually goes *inside* it, not to whatever spends the
    // budget: a lattice sized to burn 325 walls leaves 784 tiles of interior
    // for 1004 tiles of structures, so a third of the base ends up outside
    // the walls and the compartments fill with collectors.
    let core_area: i32 = names
        .iter()
        .map(|(n, c)| {
            let (w, h, class, ..) = dims(data, n);
            if goes_inside(n, &class) {
                (*c as i32) * w * h
            } else {
                0
            }
        })
        .sum();
    let (walls, cells, box_) = wall_lattice(wall_budget, core_area);
    // What each compartment already holds, so same-type defences can be split
    // across rooms rather than stacked in one.
    let mut held: Vec<std::collections::HashMap<String, usize>> =
        vec![std::collections::HashMap::new(); cells.len()];
    for &(x, y) in &walls {
        if let Some(c) = idx(x, y) {
            occ[c] = 2;
        }
    }
    layout.walls = walls;

    for (name, count) in &names {
        let name = name.as_str();
        let Some(level) = level_for(data, name, th) else {
            continue;
        };
        let (w, h, class, range, min_range, is_trap) = dims(data, name);
        if w == 0 {
            continue;
        }
        let inside = goes_inside(name, &class);
        for _ in 0..*count {
            let rect = if is_trap {
                // Traps go in the leftovers, wherever those are — they are what
                // the gaps between compartments are for.
                place_trap(&occ, &box_, w, h, &mut rng)
            } else if inside {
                place_in_cells(&occ, &cells, &mut held, name, w, h, priority(name))
                    .or_else(|| place_outside(&occ, &box_, w, h))
            } else {
                // Collectors, camps and huts live outside the walls, as they do
                // in every real base. Sending them through the compartments
                // first is what pushed the defences out.
                place_outside(&occ, &box_, w, h)
                    .or_else(|| place_in_cells(&occ, &cells, &mut held, name, w, h, 4))
            };
            let rect = rect.or_else(|| find_spot(&occ, w, h, ring_for(name), &mut rng));
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

/// Whether a structure belongs inside the walls.
///
/// This is the split every real base makes, and getting it wrong is what made
/// the generated ones look like a car park. Walls are scarce — 325 tiles at
/// TH17 against 1004 tiles of structures — so they cannot enclose everything,
/// and the choice of what they *do* enclose is most of what makes a layout
/// good or bad.
///
/// Inside: the town hall, every defence, the storages, the clan castle and
/// hero hall. Outside: collectors and mines (they hold little and are meant to
/// be raided), army camps, huts, and the army buildings, which have no defensive
/// value and would otherwise eat compartments the defences need.
fn goes_inside(name: &str, class: &str) -> bool {
    match name {
        "Army Camp" | "Builders Hut" | "BOBs Hut" | "Helper Hut" | "Gold Mine"
        | "Elixir Collector" | "Dark Elixir Drill" | "Barracks" | "Dark Barracks"
        | "Laboratory" | "Spell Factory" | "Dark Spell Factory" | "Siege Workshop"
        | "Blacksmith" => false,
        "Clan Castle" | "Hero Hall" | "Pet House" => true,
        _ => class == "Defense" || class == "Town Hall" || name.ends_with("Storage"),
    }
}

/// Finds room inside the wall compartments, working outward from the centre.
///
/// `rank` biases where a structure lands: the town hall and heavy defences take
/// the core cells, everything else fills outward. Falling back to the open
/// ground outside the lattice is the caller's job.
///
/// Within that bias, a structure prefers the compartment holding **fewest of
/// its own kind**. First-fit put all four X-Bows in one room and both
/// Scattershots side by side, and that is the single most punished mistake in
/// current war base design: one freeze or one Ice Golem stall takes out the
/// whole splash core at once. Spreading them forces an attacker to pick a side.
fn place_in_cells(
    occ: &[u8],
    cells: &[Rect],
    held: &mut [std::collections::HashMap<String, usize>],
    name: &str,
    w: i32,
    h: i32,
    rank: u8,
) -> Option<Rect> {
    // Low-rank structures start at the centre; high-rank ones skip past the
    // core so they do not squat in the compartments the defences need.
    let skip = match rank {
        0 | 1 => 0,
        2 => 1,
        3 => 2,
        _ => cells.len() / 3,
    };
    let usable = skip.min(cells.len().saturating_sub(1));

    // Order candidate compartments by how many of this structure they already
    // hold, then by the centre-first order the lattice was built in.
    let mut order: Vec<usize> = (usable..cells.len()).collect();
    order.sort_by_key(|&i| (held[i].get(name).copied().unwrap_or(0), i));

    for i in order {
        let cell = &cells[i];
        for y in cell.y..cell.bottom().saturating_sub(h - 1) {
            for x in cell.x..cell.right().saturating_sub(w - 1) {
                let r = Rect::new(x, y, w, h);
                if crate::legality::can_place(&r, occ) {
                    *held[i].entry(name.to_string()).or_default() += 1;
                    return Some(r);
                }
            }
        }
    }
    None
}

/// Builds a lattice of closed wall compartments.
///
/// Returns the wall tiles, the interior rectangle of each compartment, and the
/// lattice's bounding box, which is what tells everything else where "outside
/// the walls" starts.
///
/// `needed` is the footprint area that has to fit inside. The lattice is sized
/// to hold it and no more: **the smallest compartments that still do the job**.
/// That direction matters. Bigger compartments hold more but defend worse — one
/// Jump Spell into an eleven-wide cell opens the whole base — so spare wall
/// budget is better spent on subdividing than on enclosing empty ground.
///
/// The previous version maximised interior area instead, which is why bases
/// came out with 784 tiles of compartment for 1004 tiles of structure: a third
/// of the base spilled outside and the compartments filled with collectors.
fn wall_lattice(budget: usize, needed: i32) -> (Vec<(i32, i32)>, Vec<Rect>, Rect) {
    let whole = Rect::new(ORIGIN, ORIGIN, BUILDABLE, BUILDABLE);
    if budget < 24 {
        return (Vec::new(), vec![whole], Rect::new(ORIGIN, ORIGIN, 0, 0));
    }
    // What a compartment holds is not its area. Almost every structure in the
    // game is 3x3, so a 7-wide interior fits two per axis and wastes the rest —
    // 36 tiles of 49, not 49. Sizing on raw area is what left TH9 with a
    // lattice that looked big enough on paper and put a third of its defences
    // outside the walls.
    let capacity = |cell: i32, lines: i32| {
        let per_axis = (cell - 1) / 3;
        (lines - 1) * (lines - 1) * 9 * per_axis * per_axis
    };

    // Cells are at least 6 wide so a 4x4 still fits in one.
    let mut best: Option<(i32, i32, i32)> = None;
    let mut fallback: Option<(i32, i32, i32)> = None;
    for cell in 6..=13i32 {
        for lines in 2..=7i32 {
            let span = (lines - 1) * cell;
            if span + 1 > BUILDABLE - 2 {
                continue;
            }
            let tiles = (2 * lines * (span + 1) - lines * lines) as usize;
            if tiles > budget {
                continue;
            }
            let cap = capacity(cell, lines);
            // The same headroom the subdivision pass uses. The capacity model
            // counts 3x3s; the handful of 4x4s — town hall, Eagle Artillery,
            // Hero Hall — each waste most of a compartment's remainder. Sizing
            // to an exact fit left TH5 with four 6x6 rooms for a core that
            // needed every tile of them, and six defences outside the walls.
            if cap >= needed + needed / 6 {
                // Among lattices that hold the core, take the **smallest cell**.
                // Compartment size is the defensive variable: one Jump Spell
                // into a 12-wide cell opens the base, where the same spell into
                // a 6-wide cell opens one room. Capacity breaks ties.
                if best.is_none_or(|(c, _, cp)| (cell, cap) < (c, cp)) {
                    best = Some((cell, lines, cap));
                }
            } else if fallback.is_none_or(|(_, _, cp)| cap > cp) {
                // Nothing holds the whole core; take the most capacity going.
                fallback = Some((cell, lines, cap));
            }
        }
    }
    let Some((cell, lines, _)) = best.or(fallback) else {
        return (Vec::new(), vec![whole], Rect::new(ORIGIN, ORIGIN, 0, 0));
    };

    let span = (lines - 1) * cell;
    let x0 = ORIGIN + (BUILDABLE - span) / 2;
    let y0 = x0;

    // Line offsets along each axis, starting uniform.
    let mut xs: Vec<i32> = (0..lines).map(|i| i * cell).collect();
    let mut ys = xs.clone();

    // Spend what is left of the budget subdividing the widest compartments.
    //
    // The uniform lattice that holds the core rarely spends the whole
    // allowance — at TH17 it used 232 walls of 325, and 93 unspent walls is a
    // third of a base's defensive value sitting in the bank. Each extra line
    // splits every compartment along one axis, so the compartments stop being
    // uniform: which is both better defensively and what real bases look like.
    //
    // Splitting costs capacity as well as walls (a 9-wide interior holds three
    // 3x3s per axis, two 4-wide halves hold one each), so it stops as soon as
    // the core would no longer fit.
    loop {
        let used = lattice_cost(&xs, &ys);
        let (Some(ax), Some(ay)) = (widest_gap(&xs), widest_gap(&ys)) else {
            break;
        };
        // Split whichever axis currently has the widest compartment, so the
        // lattice stays roughly square rather than turning into corridors.
        let (axis, at) = if xs[ax + 1] - xs[ax] >= ys[ay + 1] - ys[ay] {
            (0, ax)
        } else {
            (1, ay)
        };
        let target = if axis == 0 { &xs } else { &ys };
        let gap = target[at + 1] - target[at];
        // A split has to leave both halves able to hold something.
        if gap < 9 {
            break;
        }
        let cut = target[at] + gap / 2;
        let (mut nx, mut ny) = (xs.clone(), ys.clone());
        if axis == 0 {
            nx.insert(at + 1, cut);
        } else {
            ny.insert(at + 1, cut);
        }
        let cost = lattice_cost(&nx, &ny);
        // Headroom, not just a fit. The capacity model counts 3x3s, and the
        // few 4x4s — town hall, Eagle Artillery, Hero Hall — waste more of a
        // compartment than it allows for. Splitting until capacity exactly
        // equals the core pushed four defences back outside the walls.
        if cost > budget || grid_capacity(&nx, &ny) < needed + needed / 8 {
            break;
        }
        debug_assert!(cost > used, "a split must consume budget");
        xs = nx;
        ys = ny;
    }

    let mut walls = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for &dx in &xs {
        for t in 0..=span {
            if seen.insert((x0 + dx, y0 + t)) {
                walls.push((x0 + dx, y0 + t));
            }
        }
    }
    for &dy in &ys {
        for t in 0..=span {
            if seen.insert((x0 + t, y0 + dy)) {
                walls.push((x0 + t, y0 + dy));
            }
        }
    }

    // Interiors are the open rectangles between the lines.
    let mut cells = Vec::new();
    for r in 0..ys.len() - 1 {
        for c in 0..xs.len() - 1 {
            cells.push(Rect::new(
                x0 + xs[c] + 1,
                y0 + ys[r] + 1,
                xs[c + 1] - xs[c] - 1,
                ys[r + 1] - ys[r] - 1,
            ));
        }
    }
    // Centre-first, so the town hall and the heavy defences claim the core.
    //
    // Measured from each compartment's own centre rather than its lattice
    // index, because the compartments are no longer uniform once the leftover
    // budget has subdivided some of them.
    let (mx, my) = (x0 + span / 2, y0 + span / 2);
    cells.sort_by_key(|r| {
        let cx = 2 * r.x + r.w - 2 * mx;
        let cy = 2 * r.y + r.h - 2 * my;
        // Squared distance in half-tiles: integral, so the order is stable.
        (cx * cx + cy * cy, r.y, r.x)
    });
    (walls, cells, Rect::new(x0, y0, span + 1, span + 1))
}

/// Wall tiles a lattice with these line offsets consumes.
///
/// Lines cross, so the naive sum double-counts every intersection.
fn lattice_cost(xs: &[i32], ys: &[i32]) -> usize {
    let span = *xs.last().unwrap_or(&0).max(ys.last().unwrap_or(&0));
    let n = (span + 1) as usize;
    xs.len() * n + ys.len() * n - xs.len() * ys.len()
}

/// How many 3x3 structures a lattice's compartments hold in total.
fn grid_capacity(xs: &[i32], ys: &[i32]) -> i32 {
    let per = |v: &[i32]| -> i32 {
        v.windows(2).map(|w| (w[1] - w[0] - 1) / 3).sum()
    };
    // Rows and columns multiply out: every column band meets every row band.
    9 * per(xs) * per(ys)
}

/// Index of the widest interval in a sorted line list.
fn widest_gap(v: &[i32]) -> Option<usize> {
    (0..v.len().checked_sub(1)?).max_by_key(|&i| (v[i + 1] - v[i], std::cmp::Reverse(i)))
}

/// Places a structure outside the wall lattice, keeping a tile of air where it
/// can.
///
/// The gap is what stops the outer ring reading as one solid brick. It is tried
/// first and dropped if the base is too full to afford it — a structure placed
/// touching its neighbour is still better than one that could not be placed.
fn place_outside(occ: &[u8], box_: &Rect, w: i32, h: i32) -> Option<Rect> {
    // Widest margin first. Packing the outer ring solid against the walls is
    // what made the high town halls look cramped: at TH17 there are 35
    // structures and 49 traps outside the lattice, and first-fit filled the
    // ring nearest the wall completely before moving outward, leaving a dense
    // band against the wall and bare ground beyond it. The real game spreads
    // them over the field.
    for margin in [2, 1, 0] {
        // Work outward from the lattice edge, so the outer ring hugs the walls
        // rather than scattering against the map border.
        for pad in 0..BUILDABLE {
            for r in ring_positions(box_, pad, w, h) {
                if !r.inside_buildable() || !crate::legality::can_place(&r, occ) {
                    continue;
                }
                if margin > 0 && !clear_margin(occ, &r, margin) {
                    continue;
                }
                return Some(r);
            }
        }
    }
    None
}

/// Candidate rectangles at a given distance outside the lattice box.
fn ring_positions(box_: &Rect, pad: i32, w: i32, h: i32) -> Vec<Rect> {
    let mut out = Vec::new();
    let (l, t) = (box_.x - pad - w, box_.y - pad - h);
    let (r, b) = (box_.right() + pad, box_.bottom() + pad);
    for k in t..=b {
        out.push(Rect::new(l, k, w, h));
        out.push(Rect::new(r, k, w, h));
    }
    for k in l..=r {
        out.push(Rect::new(k, t, w, h));
        out.push(Rect::new(k, b, w, h));
    }
    out
}

/// Whether a rectangle has a clear border of `m` tiles on every side.
///
/// Tiles off the playfield count as clear, so a structure against the map edge
/// is not penalised for the edge itself.
fn clear_margin(occ: &[u8], r: &Rect, m: i32) -> bool {
    for y in r.y - m..r.bottom() + m {
        for x in r.x - m..r.right() + m {
            match idx(x, y) {
                Some(c) if occ[c] != 0 => return false,
                _ => {}
            }
        }
    }
    true
}

/// Places a trap in whatever space is left.
///
/// Traps are the one thing that genuinely belongs in the leftovers: they are
/// 1x1 or 2x2, they do not obstruct movement, and their value comes from
/// sitting on the routes between compartments rather than from being protected.
fn place_trap(occ: &[u8], box_: &Rect, w: i32, h: i32, rng: &mut ChaCha8Rng) -> Option<Rect> {
    // Inside the lattice first, where the troops will be.
    let inner = Rect::new(box_.x, box_.y, box_.w, box_.h);
    for _ in 0..160 {
        let x = rng.gen_range(inner.x - 4..inner.right() + 4);
        let y = rng.gen_range(inner.y - 4..inner.bottom() + 4);
        let r = Rect::new(x, y, w, h);
        if r.inside_buildable() && crate::legality::can_place(&r, occ) {
            return Some(r);
        }
    }
    None
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
    fn every_base_has_exactly_one_town_hall() {
        // The hall is not listed in townhall_levels.csv, because that table
        // says what a hall permits and a hall does not permit itself. Reading
        // the table alone produced bases with no Town Hall at all — legal by
        // every check that existed, and missing the one building the whole
        // game is about.
        let Ok(d) = GameData::load_default() else { return };
        for th in 1..=d.max_townhall() {
            let l = seed(&d, th, 42);
            let halls: Vec<_> = l.placements.iter().filter(|p| p.is_town_hall()).collect();
            assert_eq!(halls.len(), 1, "TH{th} has {} town halls", halls.len());
            assert_eq!(halls[0].level, th, "the hall should sit at its own level");
        }
    }

    #[test]
    fn the_town_hall_sits_near_the_middle() {
        let Ok(d) = GameData::load_default() else { return };
        for th in [8, 12, 17] {
            let l = seed(&d, th, 42);
            let r = l.town_hall().expect("placed").rect;
            let c = TOTAL / 2;
            let off = (r.x + r.w / 2 - c).abs().max((r.y + r.h / 2 - c).abs());
            assert!(off <= 6, "TH{th} hall is {off} tiles off centre");
        }
    }

    #[test]
    fn defences_and_storages_go_inside_the_walls() {
        // The split that makes a layout a layout. Walls are scarce — 325 tiles
        // at TH17 against 1004 tiles of structures — so what they enclose is
        // most of what makes a base good. Sending everything through the
        // compartments first filled them with collectors and pushed a third of
        // the defences outside.
        let Ok(d) = GameData::load_default() else { return };
        for th in [9, 12, 15, 17] {
            let l = seed(&d, th, 42);
            let enc = crate::metrics::enclosed_map(&l);
            let inside = |p: &Placement| {
                p.rect
                    .tiles()
                    .all(|(x, y)| idx(x, y).is_some_and(|c| enc[c]))
            };
            let defs: Vec<_> = l.placements.iter().filter(|p| p.is_defense()).collect();
            let walled = defs.iter().filter(|p| inside(p)).count();
            assert!(
                walled * 10 >= defs.len() * 9,
                "TH{th}: only {walled} of {} defences are behind walls",
                defs.len()
            );

            let stores: Vec<_> = l.placements.iter().filter(|p| p.is_storage()).collect();
            assert!(!stores.is_empty(), "TH{th} has no storages");
            assert!(
                stores.iter().all(|p| inside(p)),
                "TH{th} leaves a storage outside the walls"
            );
        }
    }

    #[test]
    fn same_type_defences_are_split_across_compartments() {
        // The most punished mistake in current war base design: stacking both
        // Scattershots, or all four X-Bows, in one room means a single freeze or
        // one Ice Golem stall removes the whole splash core at once. First-fit
        // placement did exactly that.
        let Ok(d) = GameData::load_default() else { return };
        for th in [13, 15, 17] {
            let l = seed(&d, th, 0xC0FFEE);
            let mut per_room: std::collections::HashMap<(String, i32, i32), usize> =
                std::collections::HashMap::new();
            for p in l.placements.iter().filter(|p| p.is_defense()) {
                // A coarse 10-tile bucket stands in for "the same compartment".
                *per_room
                    .entry((p.name.clone(), p.rect.x / 10, p.rect.y / 10))
                    .or_default() += 1;
            }
            for name in ["Scattershot", "Eagle Artillery", "Monolith"] {
                let total = l.placements.iter().filter(|p| p.name == name).count();
                if total < 2 {
                    continue;
                }
                let worst = per_room
                    .iter()
                    .filter(|((n, _, _), _)| n == name)
                    .map(|(_, c)| *c)
                    .max()
                    .unwrap_or(0);
                assert!(
                    worst < total,
                    "TH{th}: all {total} {name} sit in one compartment"
                );
            }
        }
    }

    #[test]
    fn collectors_and_camps_stay_outside_the_walls() {
        let Ok(d) = GameData::load_default() else { return };
        let l = seed(&d, 17, 42);
        let enc = crate::metrics::enclosed_map(&l);
        for p in &l.placements {
            if !matches!(
                p.name.as_str(),
                "Gold Mine" | "Elixir Collector" | "Army Camp" | "Builders Hut"
            ) {
                continue;
            }
            let walled = p
                .rect
                .tiles()
                .all(|(x, y)| idx(x, y).is_some_and(|c| enc[c]));
            assert!(!walled, "{} took a compartment a defence needs", p.name);
        }
    }

    #[test]
    fn a_storage_is_a_storage_and_a_mine_is_not() {
        // BuildingClass "Resource" covers both, and at TH17 that is 17
        // collectors against 9 storages — so testing the class made
        // `loot_protected` mostly a measure of how well the mines were
        // defended, which is not a thing anyone wants.
        let Ok(d) = GameData::load_default() else { return };
        let l = seed(&d, 17, 42);
        let named = |n: &str| l.placements.iter().find(|p| p.name == n);
        assert!(named("Gold Storage").is_some_and(|p| p.is_storage()));
        assert!(named("Gold Mine").is_some_and(|p| !p.is_storage()));
        assert!(named("Elixir Collector").is_some_and(|p| !p.is_storage()));
    }

    #[test]
    fn the_wall_budget_is_not_left_sitting_in_the_bank() {
        let Ok(d) = GameData::load_default() else { return };
        for th in [9, 12, 15, 17] {
            let l = seed(&d, th, 42);
            let budget = d.townhall(th).map(|t| t.count_of("Wall")).unwrap_or(0);
            assert!(l.walls.len() as i64 <= budget, "TH{th} overspent its walls");
            assert!(
                l.walls.len() as i64 * 10 >= budget * 6,
                "TH{th} used only {} of {budget} walls",
                l.walls.len()
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
