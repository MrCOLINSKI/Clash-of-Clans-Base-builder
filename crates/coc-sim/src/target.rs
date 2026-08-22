//! Two-stage target selection: the "rule of N".
//!
//! This is the single most important behaviour in the whole simulator, and the
//! one most implementations get wrong by making it *better*. A troop does not
//! pick the cheapest building to reach. It picks the cheapest among the **three
//! nearest by straight-line distance**, and straight-line distance knows nothing
//! about walls.
//!
//! The consequence is the behaviour every player has sworn at:
//!
//! > A Giant stands next to a Cannon and walks away from it, around a wall, to
//! > a Mortar twelve tiles off.
//!
//! That is not a bug in the game and must not be a bug here. The Cannon was
//! never a candidate, because three other defences were closer *as the crow
//! flies*, so it was dropped in stage 1 and its short path was never costed. A
//! simulator whose troops always choose sensibly has implemented this wrong, and
//! every layout it evaluates will be scored against an attacker that does not
//! exist.
//!
//! ## Stage 1 — cheap filter
//!
//! Restrict to the buildings the unit's `PreferedTargetBuildingClass` makes
//! relevant, then take the `rule_of_n` nearest by straight-line distance,
//! **ignoring walls entirely**. `rule_of_n` is `TARGET_LIST_SIZE` from
//! `globals.csv`, which ships as 3.
//!
//! ## Stage 2 — expensive scoring
//!
//! Cost a real path to each survivor, walls traversable at the unit's wall
//! cost, and take the cheapest. One Dijkstra from the unit serves all of them.

use crate::config::PathingConfig;
use crate::movement::Movement;
use crate::navgrid::NavGrid;
use crate::path::{self, CostField};
use coc_core::grid::{Layout, Placement};
use coc_data::model::Character;

/// The building class a unit prefers, as the shipped data spells it.
///
/// These are the only values `PreferedTargetBuildingClass` takes across all 193
/// characters: blank, `Any Building`, `Defense`, `Resource`, `Wall`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preference {
    /// No stated preference: the nearest structure of any kind.
    None,
    /// `Any Building` — structures, in preference to walls.
    AnyBuilding,
    /// `Defense` — Giants, Hog Riders, Balloons.
    Defense,
    /// `Resource` — Goblins.
    Resource,
    /// `Wall` — Wall Breakers, and the wall-clearing siege machines.
    Wall,
}

impl Preference {
    /// Reads a unit's preference from its data column.
    ///
    /// An unrecognised value is treated as no preference rather than being
    /// rejected, because a new class added upstream should degrade to "attack
    /// the nearest thing" rather than stop the simulator.
    pub fn of(unit: &Character) -> Preference {
        match unit.preferred_target_class.as_deref() {
            Some("Defense") => Preference::Defense,
            Some("Resource") => Preference::Resource,
            Some("Wall") => Preference::Wall,
            Some("Any Building") => Preference::AnyBuilding,
            _ => Preference::None,
        }
    }

    /// Whether a placement satisfies this preference.
    fn matches(&self, p: &Placement) -> bool {
        match self {
            Preference::None | Preference::AnyBuilding => true,
            Preference::Defense => p.class == "Defense",
            Preference::Resource => p.class == "Resource",
            // Walls are not placements; they are handled separately.
            Preference::Wall => false,
        }
    }
}

/// What a unit can be sent at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Index into `Layout::placements`.
    Building(usize),
    /// A wall tile.
    Wall(i32, i32),
}

/// A stage-2 result: the chosen target, where to stand, and what it cost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Acquired {
    pub target: Target,
    /// The cell the unit walks to in order to attack.
    pub stand: (i32, i32),
    /// Path cost in game distance units, walls included.
    pub cost: i64,
    /// The stage-1 candidates, in the order they were considered.
    ///
    /// Retained because "why did it pick that" is the question this module
    /// exists to answer, and the answer is almost always in this list.
    pub considered: Vec<Target>,
}

/// Stage 1: the nearest `rule_of_n` relevant targets, ignoring walls.
///
/// Ties break on the target's index in the layout, so two buildings at exactly
/// the same distance always resolve the same way.
pub fn stage1(
    unit: &Character,
    layout: &Layout,
    grid: &NavGrid,
    from: (i32, i32),
    cfg: &PathingConfig,
    n: usize,
) -> Vec<Target> {
    let pref = Preference::of(unit);

    let mut scored: Vec<(i64, usize, Target)> = Vec::new();
    if pref == Preference::Wall {
        for (i, &(wx, wy)) in layout.walls.iter().enumerate() {
            let cell = grid.tile_centre(wx, wy);
            scored.push((path::straight_dist2(grid, from, cell), i, Target::Wall(wx, wy)));
        }
    } else {
        for (i, p) in layout.placements.iter().enumerate() {
            // Traps are not structures; nothing targets them.
            if p.is_trap || !pref.matches(p) {
                continue;
            }
            let d2 = nearest_dist2(grid, from, p, cfg);
            scored.push((d2, i, Target::Building(i)));
        }
    }

    // A preference is a preference, not a requirement. A Giant whose last
    // defence has fallen keeps attacking; it just stops being choosy.
    if scored.is_empty() && pref != Preference::None {
        for (i, p) in layout.placements.iter().enumerate() {
            if p.is_trap {
                continue;
            }
            scored.push((nearest_dist2(grid, from, p, cfg), i, Target::Building(i)));
        }
    }

    scored.sort_by_key(|&(d2, i, _)| (d2, i));
    scored.truncate(n);
    scored.into_iter().map(|(_, _, t)| t).collect()
}

/// Full two-stage selection.
///
/// Returns `None` only when nothing at all can be reached — every candidate
/// walled off behind buildings, or the layout empty.
///
/// When every stage-1 candidate turns out to be unreachable the list is grown,
/// up to `max_rule_of_n`. ASSUMED: `MAX_TARGET_LIST_SIZE` ships as 6 and the
/// client plainly grows the list under some condition, but which condition is
/// not identified — see ASSUMPTIONS.md 4.1. Unreachability is the one trigger
/// that is certainly needed, since without it a unit sealed off from its three
/// nearest targets would simply stop.
pub fn acquire(
    unit: &Character,
    layout: &Layout,
    grid: &NavGrid,
    from: (i32, i32),
    cfg: &PathingConfig,
) -> Option<Acquired> {
    let movement = Movement::of(unit, cfg);
    let field = (!movement.ignores_obstacles())
        .then(|| path::cost_field(grid, &movement, from));

    let mut n = cfg.rule_of_n;
    loop {
        let candidates = stage1(unit, layout, grid, from, cfg, n);
        if candidates.is_empty() {
            return None;
        }
        if let Some(a) = stage2(unit, layout, grid, from, cfg, &field, &candidates) {
            return Some(a);
        }
        // Everything in the list was unreachable. Widen, or give up.
        if n >= cfg.max_rule_of_n || candidates.len() < n {
            return None;
        }
        n = (n + 1).min(cfg.max_rule_of_n);
    }
}

/// Stage 2: cost a real path to each survivor and take the cheapest.
fn stage2(
    unit: &Character,
    layout: &Layout,
    grid: &NavGrid,
    from: (i32, i32),
    cfg: &PathingConfig,
    field: &Option<CostField>,
    candidates: &[Target],
) -> Option<Acquired> {
    let range = unit.attack_range.unwrap_or(0);
    let mut best: Option<(i64, Target, (i32, i32))> = None;

    for (order, &t) in candidates.iter().enumerate() {
        let rect = match t {
            Target::Building(i) => layout.placements.get(i)?.rect,
            Target::Wall(x, y) => coc_core::grid::Rect::new(x, y, 1, 1),
        };
        let stops = path::attack_cells(grid, &rect, range.max(1), cfg.units_per_tile);
        if stops.is_empty() {
            continue;
        }

        let scored = match field {
            // Ground: real path cost, walls charged.
            Some(f) => f.cheapest_of(&stops).map(|(cell, c)| (c, cell)),
            // Air and burrowing units: geometry, not a search.
            None => stops
                .iter()
                .map(|&cell| (path::straight_cost(grid, from, cell), cell))
                .min_by_key(|&(c, cell)| (c, cell.1, cell.0)),
        };
        let Some((cost, cell)) = scored else {
            // Unreachable. The config says discard it and try the next
            // candidate rather than walk at a target it can never hit.
            debug_assert!(cfg.discard_unreachable_targets);
            continue;
        };
        // Ties go to the candidate stage 1 ranked first, i.e. the nearer one.
        if best.as_ref().is_none_or(|&(b, _, _)| cost < b) {
            best = Some((cost, t, cell));
        }
        let _ = order;
    }

    best.map(|(cost, target, stand)| Acquired {
        target,
        stand,
        cost,
        considered: candidates.to_vec(),
    })
}

/// Squared straight-line distance from a cell to a placement's footprint.
fn nearest_dist2(
    grid: &NavGrid,
    from: (i32, i32),
    p: &Placement,
    cfg: &PathingConfig,
) -> i64 {
    path::dist2_to_rect(grid, from, &p.rect, cfg.units_per_tile)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::test_globals;
    use coc_core::grid::Rect;
    use coc_data::model::CharacterLevel;

    fn cfg() -> PathingConfig {
        PathingConfig::load_default(&test_globals()).expect("config loads")
    }

    fn unit(name: &str, pref: Option<&str>, range: i32) -> Character {
        Character {
            name: name.into(),
            is_flying: false,
            is_jumper: false,
            is_underground: false,
            triggers_traps: true,
            speed: Some(16),
            attack_range: Some(range),
            attack_speed_ms: Some(1000),
            air_targets: false,
            ground_targets: true,
            housing_space: Some(5),
            preferred_target_class: pref.map(str::to_string),
            preferred_target_damage_mod: None,
            wall_movement_cost: None,
            levels: vec![CharacterLevel {
                level: 1,
                hitpoints: 500,
                dps: Some(20),
            }],
        }
    }

    fn place(name: &str, class: &str, x: i32, y: i32, w: i32, h: i32) -> Placement {
        Placement {
            name: name.into(),
            level: 1,
            rect: Rect::new(x, y, w, h),
            class: class.into(),
            range: 900,
            min_range: 0,
            is_trap: false,
            air_targets: true,
            ground_targets: true,
        }
    }

    #[test]
    fn preference_reads_every_value_the_data_uses() {
        assert_eq!(Preference::of(&unit("Giant", Some("Defense"), 60)), Preference::Defense);
        assert_eq!(Preference::of(&unit("Goblin", Some("Resource"), 60)), Preference::Resource);
        assert_eq!(Preference::of(&unit("WB", Some("Wall"), 60)), Preference::Wall);
        assert_eq!(
            Preference::of(&unit("Electromite", Some("Any Building"), 60)),
            Preference::AnyBuilding
        );
        assert_eq!(Preference::of(&unit("Barbarian", None, 60)), Preference::None);
    }

    #[test]
    fn a_giant_ignores_everything_that_is_not_a_defence() {
        let c = cfg();
        let mut l = Layout::new(17);
        l.placements.push(place("Gold Storage", "Resource", 21, 20, 3, 3));
        l.placements.push(place("Cannon", "Defense", 30, 20, 3, 3));
        let g = NavGrid::build(&l, &c);

        let from = g.tile_centre(18, 21);
        let picked = stage1(&unit("Giant", Some("Defense"), 60), &l, &g, from, &c, c.rule_of_n);
        assert_eq!(picked, vec![Target::Building(1)], "the storage is nearer, and irrelevant");
    }

    #[test]
    fn a_preference_with_nothing_left_falls_back_rather_than_stalling() {
        let c = cfg();
        let mut l = Layout::new(17);
        l.placements.push(place("Gold Storage", "Resource", 21, 20, 3, 3));
        let g = NavGrid::build(&l, &c);
        let from = g.tile_centre(18, 21);
        let picked = stage1(&unit("Giant", Some("Defense"), 60), &l, &g, from, &c, c.rule_of_n);
        assert_eq!(picked, vec![Target::Building(0)], "no defences left; attack anything");
    }

    #[test]
    fn traps_are_never_targets() {
        let c = cfg();
        let mut l = Layout::new(17);
        let mut t = place("Giant Bomb", "Defense", 20, 20, 2, 2);
        t.is_trap = true;
        l.placements.push(t);
        l.placements.push(place("Cannon", "Defense", 30, 20, 3, 3));
        let g = NavGrid::build(&l, &c);
        let from = g.tile_centre(18, 21);
        let picked = stage1(&unit("Giant", Some("Defense"), 60), &l, &g, from, &c, c.rule_of_n);
        assert_eq!(picked, vec![Target::Building(1)]);
    }

    #[test]
    fn stage1_keeps_exactly_n_and_ranks_by_straight_line() {
        let c = cfg();
        let mut l = Layout::new(17);
        for (k, x) in [22, 25, 28, 31, 34].iter().enumerate() {
            l.placements.push(place(&format!("Cannon {k}"), "Defense", *x, 20, 3, 3));
        }
        let g = NavGrid::build(&l, &c);
        let from = g.tile_centre(18, 21);
        let picked = stage1(&unit("Giant", Some("Defense"), 60), &l, &g, from, &c, c.rule_of_n);
        assert_eq!(picked.len(), 3, "TARGET_LIST_SIZE = 3");
        assert_eq!(
            picked,
            vec![Target::Building(0), Target::Building(1), Target::Building(2)]
        );
    }

    #[test]
    fn a_wall_breaker_considers_walls_and_nothing_else() {
        let c = cfg();
        let mut l = Layout::new(17);
        l.placements.push(place("Cannon", "Defense", 20, 20, 3, 3));
        l.walls.push((26, 21));
        l.walls.push((27, 21));
        let g = NavGrid::build(&l, &c);
        let from = g.tile_centre(18, 21);
        let picked = stage1(&unit("Wall Breaker", Some("Wall"), 60), &l, &g, from, &c, c.rule_of_n);
        assert_eq!(picked, vec![Target::Wall(26, 21), Target::Wall(27, 21)]);
    }

    #[test]
    fn stage2_takes_the_cheapest_path_among_the_survivors() {
        let c = cfg();
        let mut l = Layout::new(17);
        // Two defences at nearly the same straight-line distance; one is
        // behind a wall run and so costs far more to reach.
        l.placements.push(place("Walled", "Defense", 24, 20, 3, 3));
        l.placements.push(place("Open", "Defense", 18, 26, 3, 3));
        for y in 18..26 {
            l.walls.push((22, y));
        }
        let g = NavGrid::build(&l, &c);
        let from = g.tile_centre(18, 21);
        let a = acquire(&unit("Giant", Some("Defense"), 60), &l, &g, from, &c)
            .expect("something is reachable");
        assert_eq!(a.target, Target::Building(1), "the wall made the near one dear");
        assert_eq!(a.considered.len(), 2);
    }

    #[test]
    fn a_troop_walks_past_an_adjacent_cannon_it_never_considered() {
        // The behaviour the rule of N exists to reproduce, and the reason this
        // module has a doc comment three times its own length.
        let c = cfg();
        let mut l = Layout::new(17);
        // The Giant is standing on top of this one.
        l.placements.push(place("Adjacent Cannon", "Defense", 20, 20, 3, 3));
        // Three defences nearer to the spawn than the adjacent one, as the
        // crow flies, which therefore fill the candidate list.
        l.placements.push(place("A", "Defense", 6, 5, 3, 3));
        l.placements.push(place("B", "Defense", 10, 5, 3, 3));
        l.placements.push(place("C", "Defense", 6, 9, 3, 3));

        let g = NavGrid::build(&l, &c);
        let from = g.tile_centre(4, 4);
        let giant = unit("Giant", Some("Defense"), 60);

        let considered = stage1(&giant, &l, &g, from, &c, c.rule_of_n);
        assert!(
            !considered.contains(&Target::Building(0)),
            "the nearby cannon must never reach stage 2"
        );

        let a = acquire(&giant, &l, &g, from, &c).expect("acquires");
        assert_ne!(a.target, Target::Building(0));
        assert_eq!(a.considered.len(), 3);
    }

    #[test]
    fn stage1_ignores_walls_even_when_they_dominate_the_real_cost() {
        // Stage 1 is a straight-line filter. A target sealed behind eight wall
        // tiles is still "near", and still takes a candidate slot from an open
        // target further away. Getting this wrong makes troops look clever.
        let c = cfg();
        let mut l = Layout::new(17);
        l.placements.push(place("Sealed", "Defense", 24, 20, 3, 3));
        l.placements.push(place("Open", "Defense", 34, 20, 3, 3));
        for y in 16..28 {
            l.walls.push((22, y));
        }
        let g = NavGrid::build(&l, &c);
        let from = g.tile_centre(18, 21);
        let picked = stage1(&unit("Giant", Some("Defense"), 60), &l, &g, from, &c, 1);
        assert_eq!(picked, vec![Target::Building(0)], "walls must not enter stage 1");
    }

    #[test]
    fn an_unreachable_candidate_is_discarded_and_the_list_grows() {
        let c = cfg();
        let mut l = Layout::new(17);
        // A defence sealed inside a solid ring of buildings: no cost reaches it.
        l.placements.push(place("Sealed", "Defense", 20, 20, 1, 1));
        for (x, y) in [
            (19, 19), (20, 19), (21, 19),
            (19, 20), (21, 20),
            (19, 21), (20, 21), (21, 21),
        ] {
            l.placements.push(place("Wall of buildings", "NonFunctional", x, y, 1, 1));
        }
        // A reachable defence, further away.
        l.placements.push(place("Reachable", "Defense", 34, 20, 3, 3));

        let g = NavGrid::build(&l, &c);
        let from = g.tile_centre(12, 20);
        let a = acquire(&unit("Giant", Some("Defense"), 60), &l, &g, from, &c)
            .expect("the far defence is reachable");
        assert_eq!(a.target, Target::Building(9), "the sealed one was discarded");
    }

    #[test]
    fn a_flier_ignores_walls_and_buildings_alike() {
        let c = cfg();
        let mut l = Layout::new(17);
        l.placements.push(place("Behind walls", "Defense", 24, 20, 3, 3));
        for y in 10..34 {
            l.walls.push((22, y));
            l.walls.push((23, y));
        }
        let g = NavGrid::build(&l, &c);
        let from = g.tile_centre(18, 21);

        let mut balloon = unit("Balloon", Some("Defense"), 60);
        balloon.is_flying = true;
        let air = acquire(&balloon, &l, &g, from, &c).expect("acquires");

        let giant = unit("Giant", Some("Defense"), 60);
        let ground = acquire(&giant, &l, &g, from, &c).expect("acquires");

        assert_eq!(air.target, ground.target);
        assert!(
            air.cost < ground.cost,
            "the flier paid {} and the giant {}",
            air.cost,
            ground.cost
        );
    }

    #[test]
    fn a_ranged_unit_stops_short_of_its_target() {
        let c = cfg();
        let mut l = Layout::new(17);
        l.placements.push(place("Cannon", "Defense", 24, 20, 3, 3));
        let g = NavGrid::build(&l, &c);
        let from = g.tile_centre(12, 21);

        let melee = acquire(&unit("Barbarian", None, 60), &l, &g, from, &c).expect("acquires");
        let archer = acquire(&unit("Archer", None, 350), &l, &g, from, &c).expect("acquires");

        assert_eq!(melee.target, archer.target);
        assert!(
            archer.cost < melee.cost,
            "range should shorten the walk: {} vs {}",
            archer.cost,
            melee.cost
        );
        assert!(archer.stand.0 < melee.stand.0, "the archer stops further out");
    }

    #[test]
    fn selection_is_deterministic_across_repeated_runs() {
        let c = cfg();
        let mut l = Layout::new(17);
        for k in 0..8 {
            let (x, y) = (14 + (k % 4) * 6, 14 + (k / 4) * 6);
            l.placements.push(place(&format!("D{k}"), "Defense", x, y, 3, 3));
        }
        for y in 12..30 {
            l.walls.push((20, y));
        }
        let g = NavGrid::build(&l, &c);
        let from = g.tile_centre(6, 20);
        let giant = unit("Giant", Some("Defense"), 60);
        let first = acquire(&giant, &l, &g, from, &c);
        for _ in 0..10 {
            assert_eq!(first, acquire(&giant, &l, &g, from, &c));
        }
    }

    #[test]
    fn an_empty_layout_acquires_nothing() {
        let c = cfg();
        let l = Layout::new(17);
        let g = NavGrid::build(&l, &c);
        assert!(acquire(&unit("Giant", Some("Defense"), 60), &l, &g, (20, 20), &c).is_none());
    }
}
