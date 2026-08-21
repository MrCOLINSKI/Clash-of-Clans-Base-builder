//! The data validation gate.
//!
//! This fails the build rather than letting a bad extraction reach the
//! optimizer. A silently-wrong constant does not produce an obviously wrong
//! base; it produces a confidently wrong one, which is worse.

use crate::model::UNITS_PER_TILE;
use crate::{GameData, provenance::Freshness};

/// One failed or passed check.
#[derive(Debug, Clone)]
pub struct Finding {
    pub check: String,
    pub detail: String,
    pub severity: Severity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Pass,
    Warn,
    Fail,
}

/// Outcome of running the full gate.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub findings: Vec<Finding>,
}

impl Report {
    fn pass(&mut self, check: &str, detail: impl Into<String>) {
        self.findings.push(Finding {
            check: check.into(),
            detail: detail.into(),
            severity: Severity::Pass,
        });
    }
    fn warn(&mut self, check: &str, detail: impl Into<String>) {
        self.findings.push(Finding {
            check: check.into(),
            detail: detail.into(),
            severity: Severity::Warn,
        });
    }
    fn fail(&mut self, check: &str, detail: impl Into<String>) {
        self.findings.push(Finding {
            check: check.into(),
            detail: detail.into(),
            severity: Severity::Fail,
        });
    }

    pub fn failed(&self) -> bool {
        self.findings.iter().any(|f| f.severity == Severity::Fail)
    }

    pub fn counts(&self) -> (usize, usize, usize) {
        let p = self.findings.iter().filter(|f| f.severity == Severity::Pass).count();
        let w = self.findings.iter().filter(|f| f.severity == Severity::Warn).count();
        let f = self.findings.iter().filter(|f| f.severity == Severity::Fail).count();
        (p, w, f)
    }

    pub fn render(&self) -> String {
        let mut s = String::new();
        for f in &self.findings {
            let tag = match f.severity {
                Severity::Pass => "PASS",
                Severity::Warn => "WARN",
                Severity::Fail => "FAIL",
            };
            s.push_str(&format!("[{tag}] {:<34} {}\n", f.check, f.detail));
        }
        let (p, w, fl) = self.counts();
        s.push_str(&format!("\n{p} passed, {w} warnings, {fl} failures"));
        s
    }
}

/// Runs every gate check.
///
/// `allow_stale` downgrades the freshness failure to a warning, for working
/// offline against a known-old extraction on purpose.
pub fn run(data: &GameData, allow_stale: bool) -> Report {
    let mut r = Report::default();
    check_freshness(data, allow_stale, &mut r);
    check_required_globals(data, &mut r);
    check_unit_scale(data, &mut r);
    check_no_null_required_fields(data, &mut r);
    check_townhall_consistency(data, &mut r);
    check_defense_completeness(data, &mut r);
    r
}

/// Fails when the cached extraction is behind the live game version.
fn check_freshness(data: &GameData, allow_stale: bool, r: &mut Report) {
    let p = &data.provenance;
    match p.freshness {
        Freshness::Current => r.pass(
            "data freshness",
            format!("version {} confirmed current", p.version),
        ),
        Freshness::Stale => {
            let detail = format!(
                "cached version {} is behind live {}; re-run `clashsim extract`",
                p.version,
                p.live_version.as_deref().unwrap_or("?")
            );
            if allow_stale {
                r.warn("data freshness", detail);
            } else {
                r.fail("data freshness", detail);
            }
        }
        Freshness::Unverified => {
            let detail = format!(
                "version {} has no recorded live check; currency unknown",
                p.version
            );
            if allow_stale {
                r.warn("data freshness", detail);
            } else {
                r.fail("data freshness", detail);
            }
        }
    }
}

/// The globals the pathfinder depends on must all be present.
fn check_required_globals(data: &GameData, r: &mut Report) {
    let p = &data.pathing;
    // Reaching here means extraction already succeeded; report the values so
    // they are visible in the log rather than buried in config.
    r.pass(
        "pathing globals",
        format!(
            "TARGET_LIST_SIZE={} MAX={} WALL_COST_BASE={} ({} tiles)",
            p.target_list_size,
            p.max_target_list_size,
            p.wall_cost_base,
            p.wall_cost_tiles()
        ),
    );

    if p.target_list_size < 1 {
        r.fail(
            "target list size",
            format!("TARGET_LIST_SIZE={} is not a usable candidate count", p.target_list_size),
        );
    }
    if p.target_list_size > p.max_target_list_size {
        r.fail(
            "target list size",
            format!(
                "TARGET_LIST_SIZE={} exceeds MAX_TARGET_LIST_SIZE={}",
                p.target_list_size, p.max_target_list_size
            ),
        );
    }
}

/// Confirms that 100 game distance units still span one tile.
///
/// The tile scale is not stated anywhere in the data, so it is recovered from
/// defences whose tile ranges are directly observable in game. If Supercell
/// ever rescales these, every distance in the simulator silently changes
/// meaning — so this check exists to make that a loud failure.
pub fn check_unit_scale(data: &GameData, r: &mut Report) {
    // (building, level, column value, expected tiles)
    let anchors: &[(&str, i32, i32)] = &[
        ("Cannon", 900, 9),
        ("Archer Tower", 1000, 10),
        ("Mortar", 1100, 11),
        ("X-Bow", 1400, 14),
    ];
    let mut bad = Vec::new();
    let mut checked = 0;
    for (name, expect_units, expect_tiles) in anchors {
        let Some(b) = data.building(name) else {
            bad.push(format!("{name}: not present in buildings.csv"));
            continue;
        };
        let Some(range) = b.levels.first().and_then(|l| l.attack_range) else {
            bad.push(format!("{name}: level 1 has no AttackRange"));
            continue;
        };
        checked += 1;
        if range != *expect_units {
            bad.push(format!(
                "{name}: AttackRange {range} != expected {expect_units} \
                 ({expect_tiles} tiles at {UNITS_PER_TILE}/tile)"
            ));
        }
    }
    if bad.is_empty() {
        r.pass(
            "distance unit scale",
            format!("{UNITS_PER_TILE} units/tile confirmed against {checked} defences"),
        );
    } else {
        r.fail(
            "distance unit scale",
            format!(
                "tile scale anchors disagree, every distance is suspect: {}",
                bad.join("; ")
            ),
        );
    }
}

/// Every level of every combat entity must have its required fields resolved.
///
/// This is what catches a broken carry-over parse: level 1 looks fine while
/// levels 2+ read back as null.
fn check_no_null_required_fields(data: &GameData, r: &mut Report) {
    let mut problems = Vec::new();

    for b in data.buildings.iter() {
        if b.levels.is_empty() {
            problems.push(format!("{}: no levels", b.name));
            continue;
        }
        for lvl in &b.levels {
            if lvl.hitpoints <= 0 && !b.class.is_empty() && b.class != "Deco" {
                problems.push(format!("{} lvl{}: hitpoints {}", b.name, lvl.level, lvl.hitpoints));
            }
        }
        if b.width <= 0 || b.height <= 0 {
            problems.push(format!("{}: footprint {}x{}", b.name, b.width, b.height));
        }
        // A damage-dealing defence must carry range and cooldown at every
        // level, not just the first: this is exactly where carry-over bugs
        // surface. Some Defense-class buildings deal no damage of their own
        // (Spell Tower casts a spell, Crafting Station is pure support), so
        // the requirement is keyed on having DPS rather than on the class.
        let deals_damage = b.levels.iter().any(|l| l.dps.is_some_and(|d| d > 0));
        if b.is_defense() && deals_damage {
            for lvl in &b.levels {
                if lvl.attack_range.is_none() {
                    problems.push(format!("{} lvl{}: null AttackRange", b.name, lvl.level));
                }
                if lvl.attack_speed_ms.is_none() {
                    problems.push(format!("{} lvl{}: null AttackSpeed", b.name, lvl.level));
                }
            }
        }
    }

    // Traps carry no hitpoints; what they must have is a trigger footprint and
    // a trigger radius at every level, or they can never fire in the sim.
    for t in data.traps.iter().filter(|t| t.is_home_village() && !t.disabled) {
        if t.levels.is_empty() {
            problems.push(format!("trap {}: no levels", t.name));
            continue;
        }
        if t.width <= 0 || t.height <= 0 {
            problems.push(format!("trap {}: footprint {}x{}", t.name, t.width, t.height));
        }
        if !t.air_trigger && !t.ground_trigger {
            problems.push(format!("trap {}: triggers on neither air nor ground", t.name));
        }
        for lvl in &t.levels {
            if lvl.trigger_radius.is_none() {
                problems.push(format!("trap {} lvl{}: null TriggerRadius", t.name, lvl.level));
            }
        }
    }

    for c in data.characters.iter() {
        for lvl in &c.levels {
            if lvl.hitpoints <= 0 {
                problems.push(format!("troop {} lvl{}: hitpoints {}", c.name, lvl.level, lvl.hitpoints));
            }
        }
        if c.speed.is_none() {
            problems.push(format!("troop {}: null Speed", c.name));
        }
    }

    if problems.is_empty() {
        let levels: usize = data.buildings.iter().map(|b| b.levels.len()).sum::<usize>()
            + data.traps.iter().map(|t| t.levels.len()).sum::<usize>()
            + data.characters.iter().map(|c| c.levels.len()).sum::<usize>();
        r.pass(
            "carry-over resolution",
            format!("{levels} entity-levels fully resolved, no nulls"),
        );
    } else {
        let shown: Vec<_> = problems.iter().take(12).cloned().collect();
        r.fail(
            "carry-over resolution",
            format!(
                "{} null/invalid required fields: {}{}",
                problems.len(),
                shown.join("; "),
                if problems.len() > shown.len() { " ..." } else { "" }
            ),
        );
    }
}

/// Per-town-hall counts must be self-consistent with the buildings table.
fn check_townhall_consistency(data: &GameData, r: &mut Report) {
    let mut problems = Vec::new();

    if data.townhall_levels.is_empty() {
        r.fail("townhall counts", "townhall_levels.csv produced no levels");
        return;
    }

    // The Town Hall itself defines how many levels exist.
    let th_building_levels = data
        .building("Town Hall")
        .map(|b| b.max_level())
        .unwrap_or(0);
    let th_table_levels = data.max_townhall();
    if th_building_levels != th_table_levels {
        problems.push(format!(
            "Town Hall has {th_building_levels} levels in buildings.csv but \
             townhall_levels.csv defines {th_table_levels}"
        ));
    }

    // Counts must never decrease as town hall level rises: a building you
    // could place at TH(n) cannot become unplaceable at TH(n+1).
    let mut names: Vec<&String> = data
        .townhall_levels
        .iter()
        .flat_map(|t| t.building_counts.keys())
        .collect();
    names.sort();
    names.dedup();

    for name in &names {
        let mut prev = 0i64;
        let mut prev_level = 0u32;
        for th in &data.townhall_levels {
            let c = th.count_of(name);
            if c < prev {
                problems.push(format!(
                    "{name}: count drops from {prev} at TH{prev_level} to {c} at TH{}",
                    th.level
                ));
            }
            prev = c;
            prev_level = th.level;
        }
    }

    // A building's unlock TH must be the first level where its count is > 0.
    // Builder Base entries follow their own hall progression and are not
    // governed by the home village town hall table, so they are excluded.
    let unlocks: Vec<(&str, i64)> = data
        .buildings
        .iter()
        .filter(|b| b.is_home_village())
        .filter_map(|b| b.unlock_th.map(|u| (b.name.as_str(), u)))
        .chain(
            data.traps
                .iter()
                .filter(|t| t.is_home_village())
                .filter_map(|t| t.unlock_th.map(|u| (t.name.as_str(), u))),
        )
        .collect();
    let mut early_slots = Vec::new();
    for (name, unlock) in unlocks {
        let first_allowed = data
            .townhall_levels
            .iter()
            .find(|t| t.count_of(name) > 0)
            .map(|t| t.level as i64);
        if let Some(first) = first_allowed {
            if first < unlock {
                // Not a corruption: `TownHallLevel` is the town hall needed to
                // build or repair the structure, while the count table says how
                // many slots exist. The Clan Castle occupies a slot from TH1 as
                // rubble but cannot be repaired until TH3.
                early_slots.push(format!(
                    "{name}: slot exists from TH{first} but buildable only from TH{unlock}"
                ));
            }
        }
    }

    if !early_slots.is_empty() {
        r.warn(
            "slot vs buildable town hall",
            format!(
                "{} building(s) occupy a slot before they can be built: {}",
                early_slots.len(),
                early_slots.join("; ")
            ),
        );
    }

    if problems.is_empty() {
        let total = data
            .townhall(th_table_levels)
            .map(|t| t.total_buildings())
            .unwrap_or(0);
        r.pass(
            "townhall counts",
            format!(
                "TH1-{th_table_levels} monotonic across {} entries; TH{th_table_levels} \
                 permits {total} buildings",
                names.len()
            ),
        );
    } else {
        let shown: Vec<_> = problems.iter().take(10).cloned().collect();
        r.fail(
            "townhall counts",
            format!("{} inconsistencies: {}", problems.len(), shown.join("; ")),
        );
    }
}

/// Defences must expose enough to be simulated at all.
fn check_defense_completeness(data: &GameData, r: &mut Report) {
    let defenses: Vec<&crate::model::Building> =
        data.buildings.iter().filter(|b| b.is_defense()).collect();
    if defenses.is_empty() {
        r.fail("defense coverage", "no buildings with BuildingClass=Defense");
        return;
    }

    let mut no_dps = Vec::new();
    for b in &defenses {
        let has_dps = b.levels.iter().any(|l| l.dps.is_some_and(|d| d > 0));
        if !has_dps {
            no_dps.push(b.name.as_str());
        }
    }

    // Some "defences" are support structures with no damage output of their
    // own, so this is a warning rather than a failure.
    if no_dps.is_empty() {
        r.pass(
            "defense coverage",
            format!("{} defences, all with DPS", defenses.len()),
        );
    } else {
        r.warn(
            "defense coverage",
            format!(
                "{} of {} defences have no DPS at any level (support structures?): {}",
                no_dps.len(),
                defenses.len(),
                no_dps.join(", ")
            ),
        );
    }

    let alt = defenses.iter().filter(|b| b.alt_attack_mode).count();
    r.pass(
        "mode-switching defences",
        format!("{alt} defences flagged AltAttackMode, driven from data"),
    );
}
