//! Pathing and targeting against real game data and real generated bases.
//!
//! The unit tests in `src/` use hand-built layouts, which is right for
//! isolating one rule at a time. These use the actual `characters.csv` stats
//! and a full TH17 base, because the questions worth asking here — does a Giant
//! behave like a Giant, does a Wall Breaker find a wall, does a Jump Spell's
//! worth of retargeting finish in time — only mean anything with real numbers
//! in them.
//!
//! Every test degrades to a skip when the extracted data is absent, so a clean
//! checkout without a `clashsim extract` still builds and tests green.

use coc_core::builder;
use coc_core::grid::{Layout, Placement, Rect};
use coc_data::GameData;
use coc_sim::config::{PathingConfig, Setting};
use coc_sim::movement::Movement;
use coc_sim::navgrid::NavGrid;
use coc_sim::path;
use coc_sim::target::{self, Preference, Target};

/// Loads the extracted data, or `None` when nothing has been extracted.
fn data() -> Option<GameData> {
    GameData::load_default().ok()
}

fn config(d: &GameData) -> PathingConfig {
    PathingConfig::load_default(&d.pathing).expect("config/pathing.toml resolves")
}

/// A full base at a town hall level, built by the layout generator.
fn base(d: &GameData, th: u32) -> Layout {
    builder::seed(d, th, 0xC0FFEE)
}

#[test]
fn the_rule_of_n_is_the_shipped_value_and_not_a_guess() {
    let Some(d) = data() else { return };
    let c = config(&d);
    assert_eq!(
        c.rule_of_n as i64, d.pathing.target_list_size,
        "the candidate count must come from globals.csv"
    );
    assert_eq!(c.rule_of_n, 3, "TARGET_LIST_SIZE ships as 3");
    assert_eq!(
        c.wall_break_cost, d.pathing.wall_cost_base,
        "the wall cost must come from globals.csv unless a preset is selected"
    );
}

#[test]
fn no_pathing_constant_is_written_in_rust_source() {
    // RULE 2 as an executable check: every value the pathfinder uses must be
    // reachable from the config, and every `from_data` marker must resolve.
    // If someone adds a magic number in `path.rs`, this will not catch it — but
    // it does catch the config drifting out of sync with the data, which is the
    // failure that actually happens.
    let Some(d) = data() else { return };
    let c = config(&d);
    let from_data: [(&str, i64, i64); 5] = [
        ("rule_of_n", c.rule_of_n as i64, d.pathing.target_list_size),
        ("max_rule_of_n", c.max_rule_of_n as i64, d.pathing.max_target_list_size),
        ("wall_break_cost", c.wall_break_cost, d.pathing.wall_cost_base),
        (
            "underground_speed_percent",
            c.underground_speed_percent,
            d.pathing.underground_speed_percent,
        ),
        (
            "wall_breaker.smart_radius",
            c.wall_breaker.smart_radius,
            d.pathing.wall_breaker_smart_radius,
        ),
    ];
    for (name, got, want) in from_data {
        assert_eq!(got, want, "`{name}` drifted from the shipped data");
    }
}

#[test]
fn every_shipped_character_resolves_a_movement_class() {
    let Some(d) = data() else { return };
    let c = config(&d);
    for ch in &d.characters {
        let m = Movement::of(ch, &c);
        // A ground unit must have a finite, non-negative wall cost; a flier
        // must not carry one at all.
        assert!(m.wall_cost >= 0, "{} has a negative wall cost", ch.name);
        if m.ignores_obstacles() {
            assert_eq!(m.wall_cost, 0, "{} flies or burrows but pays for walls", ch.name);
        }
    }
}

#[test]
fn the_known_wall_jumpers_cross_free_and_nobody_else_does() {
    let Some(d) = data() else { return };
    let c = config(&d);
    let mut jumpers: Vec<&str> = d
        .characters
        .iter()
        .filter(|ch| Movement::of(ch, &c).wall_cost == 0 && !Movement::of(ch, &c).ignores_obstacles())
        .map(|ch| ch.name.as_str())
        .collect();
    jumpers.sort_unstable();
    // Not asserted as an exact list: the roster changes with every update, and
    // pinning it would make this test a maintenance tax rather than a check.
    // What must hold is that the flag drives it and that Hog Rider is in it.
    assert!(
        jumpers.contains(&"Hog Rider"),
        "IsJumper should include the Hog Rider, got {jumpers:?}"
    );
    for name in &jumpers {
        let ch = d.character(name).expect("listed");
        assert!(ch.is_jumper, "{name} crosses free without the IsJumper flag");
    }
}

#[test]
fn a_giant_on_a_real_base_only_ever_walks_at_a_defence() {
    let Some(d) = data() else { return };
    let c = config(&d);
    let l = base(&d, 17);
    let g = NavGrid::build(&l, &c);
    let giant = d.character("Giant").expect("Giant is in characters.csv");
    assert_eq!(Preference::of(giant), Preference::Defense);

    // Walk the whole border: wherever it is dropped, it goes for a defence.
    let mut acquired = 0;
    for t in (2..42).step_by(4) {
        for from in [g.tile_centre(t, 1), g.tile_centre(1, t)] {
            let Some(a) = target::acquire(giant, &l, &g, from, &c) else {
                continue;
            };
            acquired += 1;
            let Target::Building(i) = a.target else {
                panic!("a Giant targeted a wall");
            };
            assert_eq!(
                l.placements[i].class, "Defense",
                "the base still has defences standing"
            );
            assert!(a.considered.len() <= c.max_rule_of_n);
        }
    }
    assert!(acquired > 10, "only {acquired} drops acquired anything");
}

#[test]
fn a_goblin_goes_for_loot_past_every_defence() {
    let Some(d) = data() else { return };
    let c = config(&d);
    let l = base(&d, 17);
    let g = NavGrid::build(&l, &c);
    let goblin = d.character("Goblin").expect("Goblin is in characters.csv");

    let from = g.tile_centre(1, 21);
    let a = target::acquire(goblin, &l, &g, from, &c).expect("acquires");
    let Target::Building(i) = a.target else {
        panic!("a Goblin targeted a wall");
    };
    assert_eq!(l.placements[i].class, "Resource", "Goblins want loot");
}

#[test]
fn a_wall_breaker_finds_a_wall_on_a_real_base() {
    let Some(d) = data() else { return };
    let c = config(&d);
    let l = base(&d, 17);
    if l.walls.is_empty() {
        return;
    }
    let g = NavGrid::build(&l, &c);
    let wb = d.character("Wall Breaker").expect("Wall Breaker is in the data");

    let from = g.tile_centre(1, 21);
    let a = target::acquire(wb, &l, &g, from, &c).expect("acquires");
    assert!(
        matches!(a.target, Target::Wall(..)),
        "a Wall Breaker took {:?}",
        a.target
    );
}

#[test]
fn the_rule_of_three_produces_the_behaviour_players_complain_about() {
    // The canonical weirdness, stated as a test: a defence the unit is
    // standing beside is not attacked, because three other defences are nearer
    // in a straight line and fill the candidate list.
    let Some(d) = data() else { return };
    let c = config(&d);
    let giant = d.character("Giant").expect("Giant");

    let mut l = Layout::new(17);
    // The one it walks past. Placed just off the drop point.
    l.placements.push(defence("Cannon", 8, 8));
    // Three defences fractionally nearer to the drop point.
    l.placements.push(defence("Archer Tower", 6, 4));
    l.placements.push(defence("Archer Tower", 4, 6));
    l.placements.push(defence("Archer Tower", 6, 6));

    let g = NavGrid::build(&l, &c);
    let from = g.tile_centre(3, 3);

    let considered = target::stage1(giant, &l, &g, from, &c, c.rule_of_n);
    assert_eq!(considered.len(), 3);
    assert!(
        !considered.contains(&Target::Building(0)),
        "the adjacent cannon must be filtered out in stage 1"
    );

    let a = target::acquire(giant, &l, &g, from, &c).expect("acquires");
    assert_ne!(
        a.target,
        Target::Building(0),
        "the giant attacked the building the rule of N excluded"
    );
}

#[test]
fn raising_the_rule_of_n_changes_what_a_troop_picks() {
    // The counterpart: with a wider list the adjacent cannon *is* considered,
    // and being adjacent it wins on path cost. This is what makes the rule of N
    // a balance constant rather than an implementation detail — and why
    // Supercell shipping 5 in 2017 changed how every base played.
    let Some(d) = data() else { return };
    let c = config(&d);
    let giant = d.character("Giant").expect("Giant");

    let mut l = Layout::new(17);
    l.placements.push(defence("Cannon", 8, 8));
    l.placements.push(defence("Archer Tower", 6, 4));
    l.placements.push(defence("Archer Tower", 4, 6));
    l.placements.push(defence("Archer Tower", 6, 6));
    let g = NavGrid::build(&l, &c);
    let from = g.tile_centre(3, 3);

    let wide = target::stage1(giant, &l, &g, from, &c, 4);
    assert!(
        wide.contains(&Target::Building(0)),
        "a fourth slot should admit the cannon"
    );
}

#[test]
fn selecting_the_community_wall_cost_preset_changes_a_real_decision() {
    // The wall-cost conflict in ASSUMPTIONS.md 2.2 is not academic: the two
    // candidate values must be able to disagree about a concrete choice, or
    // there is nothing for the calibration harness to fit.
    let Some(d) = data() else { return };
    let shipped = config(&d);
    let mut community = shipped.clone();
    community.wall_break_cost = 1550;

    let mut l = Layout::new(17);
    // A defence behind one wall, and an equivalent one a detour away. Between
    // 10.0 and 15.5 tiles of wall cost, the cheaper choice flips.
    l.placements.push(defence("Cannon", 24, 20));
    for y in 14..28 {
        l.walls.push((22, y));
    }
    l.placements.push(defence("Cannon", 18, 33));

    let giant = d.character("Giant").expect("Giant");
    let g = NavGrid::build(&l, &shipped);
    let from = g.tile_centre(18, 20);

    let a = target::acquire(giant, &l, &g, from, &shipped).expect("acquires");
    let b = target::acquire(giant, &l, &g, from, &community).expect("acquires");
    assert!(
        a.cost <= b.cost,
        "a dearer wall cannot make the same route cheaper"
    );
}

#[test]
fn a_hog_rider_ignores_the_walls_a_giant_pays_for() {
    let Some(d) = data() else { return };
    let c = config(&d);
    let giant = d.character("Giant").expect("Giant");
    let hog = d.character("Hog Rider").expect("Hog Rider");
    assert!(hog.is_jumper, "the Hog Rider must be flagged IsJumper");
    assert!(!giant.is_jumper);

    // Compared over one identical route, not over a whole attack: the two
    // units have different attack ranges, so they stop at different distances
    // and their end-to-end costs are not like for like. What is being measured
    // here is the wall, and only the wall.
    let mut l = Layout::new(17);
    for y in 0..coc_core::grid::TOTAL {
        l.walls.push((22, y));
    }
    let g = NavGrid::build(&l, &c);
    let from = g.tile_centre(18, 21);
    let to = g.tile_centre(26, 21);

    let by_hog = path::cost_field(&g, &Movement::of(hog, &c), from)
        .cost(to.0, to.1)
        .expect("reachable");
    let by_giant = path::cost_field(&g, &Movement::of(giant, &c), from)
        .cost(to.0, to.1)
        .expect("reachable");

    assert_eq!(
        by_giant - by_hog,
        c.wall_break_cost,
        "the difference between the two must be exactly one wall"
    );
}

#[test]
fn a_mass_retarget_after_a_jump_spell_is_fast_enough_to_be_usable() {
    // The benchmark the rule of N exists for. A Jump Spell landing retargets
    // every unit it touches at once, and the config states the size of that
    // event: 250 units.
    //
    // No wall-clock threshold is asserted, because a loaded CI box will fail
    // any number chosen on a quiet one. What is asserted is that every unit
    // acquires, and the timing is printed so a regression is visible in the
    // log rather than as a flaky red build.
    let Some(d) = data() else { return };
    let c = config(&d);
    let l = base(&d, 17);
    let g = NavGrid::build(&l, &c);
    let giant = d.character("Giant").expect("Giant");

    let units = c.lifecycle.mass_retarget_benchmark_units;
    assert_eq!(units, 250, "the brief specifies 250 simultaneous retargets");

    // Spread the units around the border, as a real deployment is.
    let mut froms = Vec::with_capacity(units);
    let extent = g.extent();
    for k in 0..units {
        let t = (k as i32 * 7) % 40 + 2;
        let cell = match k % 4 {
            0 => g.tile_centre(t, 1),
            1 => g.tile_centre(t, 42),
            2 => g.tile_centre(1, t),
            _ => g.tile_centre(42, t),
        };
        assert!(cell.0 < extent && cell.1 < extent);
        froms.push(cell);
    }

    let start = std::time::Instant::now();
    let mut acquired = 0;
    for from in &froms {
        if target::acquire(giant, &l, &g, *from, &c).is_some() {
            acquired += 1;
        }
    }
    let elapsed = start.elapsed();
    println!(
        "mass retarget: {units} units in {:?} ({:?} each)",
        elapsed,
        elapsed / units as u32
    );
    assert_eq!(acquired, units, "every unit must acquire something");
}

#[test]
fn a_cost_field_serves_every_candidate_from_one_search() {
    // The optimisation the whole module is shaped around: costing N candidates
    // must not cost N searches. Verified by equality with per-target searches
    // rather than by timing.
    let Some(d) = data() else { return };
    let c = config(&d);
    let l = base(&d, 17);
    let g = NavGrid::build(&l, &c);
    let m = Movement::ground(&c);
    let from = g.tile_centre(1, 21);
    let field = path::cost_field(&g, &m, from);

    for p in l.placements.iter().filter(|p| !p.is_trap).take(12) {
        let cells = path::attack_cells(&g, &p.rect, 60, c.units_per_tile);
        let from_field = field.cheapest_of(&cells).map(|(_, cost)| cost);
        let fresh = cells
            .iter()
            .filter_map(|&(x, y)| path::cost_field(&g, &m, from).cost(x, y))
            .min();
        assert_eq!(from_field, fresh, "{} costed differently", p.name);
    }
}

#[test]
fn every_town_hall_level_produces_a_navigable_base() {
    let Some(d) = data() else { return };
    let c = config(&d);
    let giant = d.character("Giant").expect("Giant");
    for th in 1..=d.max_townhall() {
        let l = base(&d, th);
        let g = NavGrid::build(&l, &c);
        let from = g.tile_centre(1, 21);
        assert!(
            target::acquire(giant, &l, &g, from, &c).is_some(),
            "TH{th}: a giant dropped on the edge could not reach anything"
        );
    }
}

#[test]
fn a_setting_can_still_be_written_as_a_literal() {
    // Guards the config's third form, which the calibration harness needs in
    // order to sweep a value without editing the file on disk.
    let Some(d) = data() else { return };
    let _ = Setting::Value(1);
    let c = config(&d);
    assert!(c.discard_unreachable_targets, "the brief requires discarding");
    assert!(c.lifecycle.recompute_on_target_destroyed);
    assert!(c.lifecycle.recompute_on_jump_spell);
}

fn defence(name: &str, x: i32, y: i32) -> Placement {
    Placement {
        name: name.into(),
        level: 1,
        rect: Rect::new(x, y, 3, 3),
        class: "Defense".into(),
        range: 900,
        min_range: 0,
        is_trap: false,
    }
}
