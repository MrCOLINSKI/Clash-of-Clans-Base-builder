//! Carry-over parser correctness against the real shipped logic tables.
//!
//! The unit tests in `src/csv.rs` cover the rule on synthetic input. These
//! cover it on the actual files, which is where it matters: a carry-over bug
//! does not crash, it silently nulls every level above the first, and every
//! stat downstream is then wrong in a way that still looks plausible.

use coc_data::model::{MovementClass, UNITS_PER_TILE};
use coc_data::GameData;

fn data() -> GameData {
    GameData::load_default().expect("cached game data loads")
}

#[test]
fn every_building_level_resolves_its_entity_columns() {
    let d = data();
    // Entity-level columns appear only on a block's first row. If carry-over
    // is broken these read back as zero or null from level 2 onward.
    for b in d.buildings.iter().filter(|b| b.is_home_village()) {
        assert!(
            b.width > 0 && b.height > 0,
            "{} has no footprint ({}x{})",
            b.name,
            b.width,
            b.height
        );
        assert!(!b.class.is_empty(), "{} has no BuildingClass", b.name);
    }
}

#[test]
fn defense_range_survives_to_the_top_level() {
    let d = data();
    let cannon = d.building("Cannon").expect("Cannon present");
    assert!(
        cannon.max_level() > 1,
        "Cannon should have many levels, got {}",
        cannon.max_level()
    );

    // AttackRange is written once, on level 1. Every level must still see it.
    let ranges: Vec<Option<i32>> = cannon.levels.iter().map(|l| l.attack_range).collect();
    assert!(
        ranges.iter().all(|r| *r == Some(9 * UNITS_PER_TILE)),
        "Cannon range should carry to every level, got {ranges:?}"
    );
}

#[test]
fn per_level_hitpoints_are_distinct_and_increasing() {
    let d = data();
    let cannon = d.building("Cannon").expect("Cannon present");
    let hp: Vec<i64> = cannon.levels.iter().map(|l| l.hitpoints).collect();

    // If carry-over over-applied, every level would share level 1's value.
    assert!(
        hp.windows(2).all(|w| w[1] >= w[0]),
        "Cannon hitpoints should be non-decreasing, got {hp:?}"
    );
    assert!(
        hp.last() > hp.first(),
        "Cannon hitpoints should grow with level, got {hp:?}"
    );
    assert!(
        hp.iter().collect::<std::collections::HashSet<_>>().len() > 1,
        "Cannon hitpoints identical at every level: carry-over over-applied"
    );
}

#[test]
fn carry_over_does_not_leak_between_entities() {
    let d = data();
    let cannon = d.building("Cannon").expect("Cannon");
    let mortar = d.building("Mortar").expect("Mortar");

    let cannon_range = cannon.levels[0].attack_range.expect("cannon range");
    let mortar_range = mortar.levels[0].attack_range.expect("mortar range");
    assert_ne!(
        cannon_range, mortar_range,
        "distinct defences should not share a range; carry leaked across blocks"
    );

    // Mortar is the clearest case: it is the only common defence with a
    // minimum range, so a leak from the preceding block is visible.
    assert!(
        mortar.levels.iter().all(|l| l.min_attack_range.is_some()),
        "Mortar should have MinAttackRange at every level"
    );
    assert!(
        cannon.levels.iter().all(|l| l.min_attack_range.is_none()),
        "Cannon has no MinAttackRange; a value here means carry leaked in"
    );
}

#[test]
fn troop_flags_resolve_from_data_not_names() {
    let d = data();

    // These are read purely from IsFlying / IsJumper / IsUnderground.
    let cases: &[(&str, MovementClass)] = &[
        ("Barbarian", MovementClass::Ground),
        ("Archer", MovementClass::Ground),
        ("Balloon", MovementClass::Air),
        ("Dragon", MovementClass::Air),
        ("Hog Rider", MovementClass::WallJumper),
        ("Miner", MovementClass::Underground),
    ];
    for (name, expect) in cases {
        let c = d.character(name).unwrap_or_else(|| panic!("{name} present"));
        assert_eq!(c.movement_class(), *expect, "{name} movement class");
    }
}

#[test]
fn wall_costs_come_from_data_with_the_global_as_fallback() {
    let d = data();
    let g = &d.pathing;

    // Wall Breakers carry an explicit per-unit override.
    let wb = d.character("Wall Breaker").expect("Wall Breaker");
    assert_eq!(
        wb.wall_movement_cost,
        Some(128),
        "Wall Breaker should carry an explicit WallMovementCost"
    );

    // Ordinary troops have none and fall back to WALL_COST_BASE.
    let barb = d.character("Barbarian").expect("Barbarian");
    assert_eq!(barb.wall_movement_cost, None);
    assert_eq!(barb.wall_cost(g), g.wall_cost_base);

    // Wall jumpers cross for free regardless.
    let hog = d.character("Hog Rider").expect("Hog Rider");
    assert_eq!(hog.wall_cost(g), 0);
}

#[test]
fn townhall_counts_carry_forward_across_levels() {
    let d = data();

    // Cannon is written at TH1=1, TH2=2, then blank through TH3 and TH4.
    // Blank means unchanged, so TH3 and TH4 must still allow 2.
    let counts: Vec<i64> = (1..=5)
        .map(|th| d.townhall(th).expect("th").count_of("Cannon"))
        .collect();
    assert_eq!(
        counts,
        vec![1, 2, 2, 2, 3],
        "blank town hall counts must carry forward, not read as zero"
    );

    // And no building may ever become unplaceable at a higher town hall.
    for name in ["Cannon", "Archer Tower", "Wall", "Air Defense", "Mortar"] {
        let series: Vec<i64> = (1..=d.max_townhall())
            .map(|th| d.townhall(th).expect("th").count_of(name))
            .collect();
        assert!(
            series.windows(2).all(|w| w[1] >= w[0]),
            "{name} counts must be non-decreasing across town halls, got {series:?}"
        );
    }
}

#[test]
fn town_hall_reaches_the_expected_level_range() {
    let d = data();
    let th = d.building("Town Hall").expect("Town Hall present");
    assert_eq!(
        th.max_level(),
        d.max_townhall(),
        "buildings.csv and townhall_levels.csv must agree on the level count"
    );
    assert!(
        d.max_townhall() >= 18,
        "extraction looks stale: only TH{} present",
        d.max_townhall()
    );
}

#[test]
fn pathing_globals_are_read_from_the_shipped_table() {
    let d = data();
    let g = &d.pathing;

    // The rule-of-N candidate count is shipped, not assumed.
    assert_eq!(
        g.target_list_size, 3,
        "TARGET_LIST_SIZE changed; the two-stage target selection depends on it"
    );
    assert!(g.max_target_list_size >= g.target_list_size);
    assert!(
        g.wall_cost_base > 0,
        "WALL_COST_BASE must be positive to penalise wall crossings"
    );
}

#[test]
fn traps_parse_with_their_own_schema() {
    let d = data();

    // The Spring Trap removes its victims outright rather than relocating
    // them: EjectVictims is false, EjectWhenKilling is true. Units it catches
    // leave the battle, so no retarget follows for them.
    let spring = d.trap("Spring Trap").expect("Spring Trap present");
    assert!(spring.ground_trigger, "Spring Trap triggers on ground units");
    assert!(!spring.air_trigger, "Spring Trap does not trigger on air");
    assert!(
        spring.removes_victims(),
        "Spring Trap should remove victims from the battle"
    );
    assert!(
        !spring.causes_displacement(),
        "Spring Trap does not relocate survivors; it deletes them"
    );
    // Housing space decides who is heavy enough to survive it.
    assert_eq!(spring.immune_by_housing(5), Some(false), "a Hog Rider is caught");
    assert_eq!(spring.immune_by_housing(20), Some(true), "a Dragon is too heavy");

    // Pushback is the mechanic that relocates a surviving unit, and so is the
    // one that forces a path recompute.
    let giant_bomb = d.trap("Giant Bomb").expect("Giant Bomb present");
    assert!(
        giant_bomb.causes_displacement(),
        "Giant Bomb pushes survivors back, which must force a retarget"
    );
    assert!(!giant_bomb.removes_victims());

    let air_bomb = d.trap("Air Bomb").expect("Air Bomb present");
    assert!(air_bomb.air_trigger, "Air Bomb triggers on air units");
    assert!(!air_bomb.ground_trigger);

    // Trigger radius must resolve at every level, not only the first.
    for t in d.traps.iter().filter(|t| t.is_home_village() && !t.disabled) {
        for lvl in &t.levels {
            assert!(
                lvl.trigger_radius.is_some(),
                "{} lvl{} has null TriggerRadius",
                t.name,
                lvl.level
            );
        }
    }
}

#[test]
fn validation_gate_passes_on_the_committed_extraction() {
    let d = data();
    let report = coc_data::validate::run(&d, false);
    assert!(
        !report.failed(),
        "committed extraction must pass the gate:\n{}",
        report.render()
    );
}
