//! Typed views over the shipped logic tables.
//!
//! Every value here is read from the extracted CSVs. Nothing in this module
//! carries a balance constant of its own — that is a hard project rule, since
//! the game rebalances continuously and any baked-in number silently corrupts
//! every simulation downstream.
//!
//! # Units
//!
//! Distances in the logic tables are in **game distance units**, where
//! [`UNITS_PER_TILE`] units span one tile. That scale is not stated in the
//! data; it is recovered from defences whose tile ranges are directly
//! observable in game, and is re-checked by [`crate::validate`] on every load
//! so a silent rescale by Supercell fails the build rather than the results.
//!
//! Times are milliseconds. `Dps` is damage per second, so damage per hit is
//! `dps * attack_speed_ms / 1000`.

use crate::csv::{CsvError, Table};
use std::collections::HashMap;

/// Game distance units per tile.
///
/// Verified against the shipped tables by [`crate::validate::check_unit_scale`]:
/// Cannon `AttackRange` 900 = 9 tiles, Archer Tower 1000 = 10, Mortar 1100 = 11
/// with `MinAttackRange` 400 = 4, X-Bow 1400 = 14. See ASSUMPTIONS.md.
pub const UNITS_PER_TILE: i32 = 100;

/// A key/value row from `globals.csv`.
#[derive(Debug, Clone, Default)]
pub struct Globals {
    numbers: HashMap<String, i64>,
    booleans: HashMap<String, bool>,
    texts: HashMap<String, String>,
}

impl Globals {
    pub fn number(&self, key: &str) -> Option<i64> {
        self.numbers.get(key).copied()
    }

    pub fn boolean(&self, key: &str) -> Option<bool> {
        self.booleans.get(key).copied()
    }

    pub fn text(&self, key: &str) -> Option<&str> {
        self.texts.get(key).map(|s| s.as_str())
    }

    /// Number of keys parsed, for the provenance banner.
    pub fn len(&self) -> usize {
        self.numbers.len() + self.booleans.len() + self.texts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn parse(table: &Table) -> Result<Globals, CsvError> {
        let mut g = Globals::default();
        for entity in &table.entities {
            let Some(row) = entity.level(1) else { continue };
            if let Some(v) = row.get("NumberValue") {
                if let Ok(n) = v.parse::<i64>() {
                    g.numbers.insert(entity.name.clone(), n);
                }
            }
            if let Some(v) = row.get("BooleanValue") {
                match v.to_ascii_lowercase().as_str() {
                    "true" => {
                        g.booleans.insert(entity.name.clone(), true);
                    }
                    "false" => {
                        g.booleans.insert(entity.name.clone(), false);
                    }
                    _ => {}
                }
            }
            if let Some(v) = row.get("TextValue") {
                g.texts.insert(entity.name.clone(), v.to_string());
            }
        }
        Ok(g)
    }
}

/// The globals that drive pathfinding and targeting.
///
/// These are read from `globals.csv` rather than configured, because the game
/// ships them. `config/pathing.toml` may override each one, but the extracted
/// value is always the default and is always reported.
#[derive(Debug, Clone, Copy)]
pub struct PathingGlobals {
    /// `TARGET_LIST_SIZE` — the "rule of N" candidate count for stage-2 pathing.
    pub target_list_size: i64,
    /// `MAX_TARGET_LIST_SIZE` — upper bound the client will grow the list to.
    pub max_target_list_size: i64,
    /// `WALL_COST_BASE` — default additive path cost of crossing a wall, in
    /// game distance units.
    pub wall_cost_base: i64,
    /// `RETARGET_AFTER_DESTROYING_WALL`.
    pub retarget_after_destroying_wall: bool,
    /// `USE_WALL_WEIGHTS_FOR_JUMP_SPELL`.
    pub use_wall_weights_for_jump_spell: bool,
    /// `UNDERGROUND_UNIT_GROUND_SPEED_PERCENTAGE` — Miner-class speed scaling.
    pub underground_speed_percent: i64,
    /// `USE_HEAT_MAP_IN_ATTACK_POSITION_SELECTION`.
    pub heat_map_attack_position: bool,
    /// `WALL_BREAKER_SMART_RADIUS` — search radius, in game distance units,
    /// within which a Wall Breaker looks for a wall worth breaking.
    pub wall_breaker_smart_radius: i64,
    /// `WALL_BREAKER_SMART_CNT_LIMIT` — how many walls that search examines.
    pub wall_breaker_smart_cnt_limit: i64,
    /// `WALL_BREAKER_SMART_RETARGET_LIMIT` — distance beyond which a Wall
    /// Breaker gives up its current wall and looks again.
    pub wall_breaker_smart_retarget_limit: i64,
    /// `WALL_BREAKER_USE_ROOMS` — whether the search is compartment-aware.
    pub wall_breaker_use_rooms: bool,
}

impl PathingGlobals {
    /// Extracts the pathing globals, failing if any is absent.
    ///
    /// Absence means the schema moved and the pathfinder would silently fall
    /// back to guessed constants, so it is an error rather than a default.
    pub fn from_globals(g: &Globals) -> Result<PathingGlobals, DataError> {
        let num = |k: &str| {
            g.number(k)
                .ok_or_else(|| DataError::MissingGlobal(k.to_string()))
        };
        let boolean = |k: &str| {
            g.boolean(k)
                .ok_or_else(|| DataError::MissingGlobal(k.to_string()))
        };
        Ok(PathingGlobals {
            target_list_size: num("TARGET_LIST_SIZE")?,
            max_target_list_size: num("MAX_TARGET_LIST_SIZE")?,
            wall_cost_base: num("WALL_COST_BASE")?,
            retarget_after_destroying_wall: boolean("RETARGET_AFTER_DESTROYING_WALL")?,
            use_wall_weights_for_jump_spell: boolean("USE_WALL_WEIGHTS_FOR_JUMP_SPELL")?,
            underground_speed_percent: num("UNDERGROUND_UNIT_GROUND_SPEED_PERCENTAGE")?,
            heat_map_attack_position: boolean("USE_HEAT_MAP_IN_ATTACK_POSITION_SELECTION")?,
            wall_breaker_smart_radius: num("WALL_BREAKER_SMART_RADIUS")?,
            wall_breaker_smart_cnt_limit: num("WALL_BREAKER_SMART_CNT_LIMIT")?,
            wall_breaker_smart_retarget_limit: num("WALL_BREAKER_SMART_RETARGET_LIMIT")?,
            wall_breaker_use_rooms: boolean("WALL_BREAKER_USE_ROOMS")?,
        })
    }

    /// Default wall crossing cost expressed in tiles.
    pub fn wall_cost_tiles(&self) -> f64 {
        self.wall_cost_base as f64 / UNITS_PER_TILE as f64
    }
}

/// Per-level stats of a building.
#[derive(Debug, Clone)]
pub struct BuildingLevel {
    pub level: u32,
    pub hitpoints: i64,
    pub dps: Option<i64>,
    /// Attack range in game distance units.
    pub attack_range: Option<i32>,
    pub min_attack_range: Option<i32>,
    pub attack_speed_ms: Option<i32>,
    /// Splash radius in game distance units.
    pub damage_radius: Option<i32>,
    /// Alternate firing mode range, for mode-switching defences.
    pub alt_attack_range: Option<i32>,
    pub alt_dps: Option<i64>,
}

/// A building, trap, or wall, with all of its levels.
#[derive(Debug, Clone)]
pub struct Building {
    pub name: String,
    /// `BuildingClass`: `Defense`, `Wall`, `Town Hall`, `Resource`, ...
    pub class: String,
    /// Town hall level at which the building first unlocks.
    pub unlock_th: Option<i64>,
    /// Footprint in whole tiles.
    pub width: i32,
    pub height: i32,
    pub air_targets: bool,
    pub ground_targets: bool,
    /// `AltAttackMode` — this defence switches firing modes.
    pub alt_attack_mode: bool,
    pub alt_air_targets: bool,
    pub alt_ground_targets: bool,
    /// `AltMultiTargets` — alternate mode hits multiple targets.
    pub alt_multi_targets: bool,
    pub preferred_target: Option<String>,
    pub preferred_target_damage_mod: Option<i64>,
    pub projectile: Option<String>,
    /// `VillageType`: blank or 0 for the home village, 1 for the Builder Base.
    pub village_type: Option<i64>,
    pub levels: Vec<BuildingLevel>,
}

impl Building {
    /// Whether this belongs to the home village.
    ///
    /// The Builder Base is a separate game mode with its own hall progression;
    /// this project simulates the home village only, so Builder Base entries
    /// are filtered out rather than mixed into home village checks.
    pub fn is_home_village(&self) -> bool {
        self.village_type.unwrap_or(0) == 0
    }

    pub fn is_defense(&self) -> bool {
        self.class == "Defense"
    }

    pub fn is_wall(&self) -> bool {
        self.class == "Wall"
    }

    pub fn is_town_hall(&self) -> bool {
        self.class == "Town Hall"
    }

    /// Footprint area in tiles.
    pub fn tiles(&self) -> i32 {
        self.width * self.height
    }

    pub fn level(&self, level: u32) -> Option<&BuildingLevel> {
        self.levels.iter().find(|l| l.level == level)
    }

    pub fn max_level(&self) -> u32 {
        self.levels.iter().map(|l| l.level).max().unwrap_or(0)
    }
}

/// Per-level stats of a trap.
#[derive(Debug, Clone)]
pub struct TrapLevel {
    pub level: u32,
    pub damage: Option<i64>,
    /// Splash radius of the trap's damage, in game distance units.
    pub damage_radius: Option<i32>,
    /// Radius within which a valid unit sets the trap off.
    pub trigger_radius: Option<i32>,
    /// Effect duration for traps that apply a status rather than damage.
    pub duration_ms: Option<i32>,
    /// Movement speed multiplier applied by the effect, as a percentage.
    pub speed_mod: Option<i64>,
    pub damage_mod: Option<i64>,
    /// Number of separate hits the trap delivers.
    pub hit_count: Option<i64>,
    pub hit_delay_ms: Option<i32>,
    /// Units spawned, for the Skeleton Trap family.
    pub num_spawns: Option<i64>,
    pub spawn_level: Option<i64>,
}

/// A trap.
///
/// Traps do not share the buildings schema: they have no hitpoints, because
/// they are consumed on trigger rather than destroyed by damage. They are
/// modelled separately rather than forced into [`Building`].
#[derive(Debug, Clone)]
pub struct Trap {
    pub name: String,
    pub width: i32,
    pub height: i32,
    pub unlock_th: Option<i64>,
    /// Whether airborne units trigger it.
    pub air_trigger: bool,
    /// Whether ground units trigger it.
    pub ground_trigger: bool,
    /// Whether ground units can walk over the tile it occupies.
    pub passable: bool,
    /// `EjectVictims` — throws its victims a distance and leaves them alive.
    pub ejects_victims: bool,
    /// `EjectWhenKilling` — removes victims from the battle entirely rather
    /// than damaging them. This is how the Spring Trap works: caught units are
    /// gone, not relocated.
    pub ejects_when_killing: bool,
    /// Radius within which victims are ejected.
    pub eject_radius: Option<i32>,
    /// `Pushback` — shoves victims a short distance. Distinct from ejection:
    /// the unit survives in a new position, which forces it to recompute its
    /// path. Bombs and Giant Bombs carry this.
    pub pushback: Option<i64>,
    /// Housing space at or above which a unit is too heavy to be pushed back.
    pub pushback_housing_limit: Option<i64>,
    /// Housing space at or above which a unit is too heavy to be ejected.
    pub eject_housing_limit: Option<i64>,
    /// `ThrowDistance` — how far ejected victims are thrown.
    pub throw_distance: Option<i32>,
    /// Prefers the highest-housing unit in range rather than the nearest.
    pub target_highest_housing: bool,
    /// Unit spawned on trigger, if any.
    pub spawned_char_ground: Option<String>,
    pub spawned_char_air: Option<String>,
    /// Excluded from the home village.
    pub village_type: Option<i64>,
    pub disabled: bool,
    pub levels: Vec<TrapLevel>,
}

impl Trap {
    /// Whether this trap belongs to the home village rather than the builder
    /// base. `VillageType` is blank or 0 for home village entries.
    pub fn is_home_village(&self) -> bool {
        self.village_type.unwrap_or(0) == 0
    }

    /// Whether triggering this trap moves its victims without killing them,
    /// which forces a path recompute for every unit caught.
    ///
    /// Ejection and pushback are separate mechanics in the data and only these
    /// two relocate a surviving unit. A trap that removes its victims outright
    /// (see [`Trap::removes_victims`]) causes no retarget, because there is no
    /// longer a unit to retarget.
    pub fn causes_displacement(&self) -> bool {
        self.ejects_victims || self.pushback.is_some_and(|p| p > 0)
    }

    /// Whether triggering removes its victims from the battle outright.
    ///
    /// The Spring Trap is the canonical case: `EjectVictims` is false and
    /// `EjectWhenKilling` is true, so caught units are deleted rather than
    /// relocated, up to `eject_housing_limit` of housing space.
    pub fn removes_victims(&self) -> bool {
        self.ejects_when_killing
    }

    /// Whether a unit of the given housing space is heavy enough to ignore
    /// this trap's displacement or removal effect.
    ///
    /// Returns `None` when the trap has no such limit recorded.
    pub fn immune_by_housing(&self, housing_space: i32) -> Option<bool> {
        let limit = if self.removes_victims() {
            self.eject_housing_limit
        } else {
            self.pushback_housing_limit
        }?;
        Some(housing_space as i64 >= limit)
    }

    pub fn level(&self, level: u32) -> Option<&TrapLevel> {
        self.levels.iter().find(|l| l.level == level)
    }

    pub fn max_level(&self) -> u32 {
        self.levels.iter().map(|l| l.level).max().unwrap_or(0)
    }
}

/// Reads `traps.csv`.
pub fn parse_traps(table: &Table) -> Result<Vec<Trap>, DataError> {
    let mut out = Vec::with_capacity(table.entities.len());
    for entity in &table.entities {
        let Some(first) = entity.level(1) else { continue };

        let mut levels = Vec::with_capacity(entity.levels.len());
        for (i, row) in entity.levels.iter().enumerate() {
            levels.push(TrapLevel {
                level: (i + 1) as u32,
                damage: row.get_i64("Damage")?,
                damage_radius: row.get_i64("DamageRadius")?.map(|v| v as i32),
                trigger_radius: row.get_i64("TriggerRadius")?.map(|v| v as i32),
                duration_ms: row.get_i64("DurationMS")?.map(|v| v as i32),
                speed_mod: row.get_i64("SpeedMod")?,
                damage_mod: row.get_i64("DamageMod")?,
                hit_count: row.get_i64("HitCnt")?,
                hit_delay_ms: row.get_i64("HitDelayMS")?.map(|v| v as i32),
                num_spawns: row.get_i64("NumSpawns")?,
                spawn_level: row.get_i64("SpawnLvl")?,
            });
        }

        out.push(Trap {
            name: entity.name.clone(),
            width: first.i64_or("Width", 1)? as i32,
            height: first.i64_or("Height", 1)? as i32,
            unlock_th: first.get_i64("TownHallLevel")?,
            air_trigger: first.bool_or("AirTrigger", false)?,
            ground_trigger: first.bool_or("GroundTrigger", false)?,
            passable: first.bool_or("Passable", false)?,
            ejects_victims: first.bool_or("EjectVictims", false)?,
            ejects_when_killing: first.bool_or("EjectWhenKilling", false)?,
            eject_radius: first.get_i64("EjectRadius")?.map(|v| v as i32),
            pushback: first.get_i64("Pushback")?,
            pushback_housing_limit: first.get_i64("PushbackHousingLimit")?,
            eject_housing_limit: first.get_i64("EjectHousingLimit")?,
            throw_distance: first.get_i64("ThrowDistance")?.map(|v| v as i32),
            target_highest_housing: first.bool_or("TargetHighestHousing", false)?,
            spawned_char_ground: first.get("SpawnedCharGround").map(str::to_string),
            spawned_char_air: first.get("SpawnedCharAir").map(str::to_string),
            village_type: first.get_i64("VillageType")?,
            disabled: first.bool_or("Disabled", false)?,
            levels,
        });
    }
    Ok(out)
}

/// How a unit moves, resolved from data flags rather than from its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MovementClass {
    /// Straight line to target, walls irrelevant.
    Air,
    /// Ground pathing with free wall traversal (`IsJumper`).
    WallJumper,
    /// Ground pathing while burrowed (`IsUnderground`).
    Underground,
    /// Ordinary ground pathing.
    Ground,
}

/// Per-level stats of a troop or hero.
#[derive(Debug, Clone)]
pub struct CharacterLevel {
    pub level: u32,
    pub hitpoints: i64,
    pub dps: Option<i64>,
}

/// A troop, hero, or siege machine.
#[derive(Debug, Clone)]
pub struct Character {
    pub name: String,
    pub is_flying: bool,
    pub is_jumper: bool,
    pub is_underground: bool,
    pub triggers_traps: bool,
    /// Movement speed in the game's internal speed units.
    pub speed: Option<i32>,
    /// Attack range in game distance units. Melee units sit well under a tile.
    pub attack_range: Option<i32>,
    pub attack_speed_ms: Option<i32>,
    pub air_targets: bool,
    pub ground_targets: bool,
    pub housing_space: Option<i32>,
    pub preferred_target_class: Option<String>,
    pub preferred_target_damage_mod: Option<i64>,
    /// `WallMovementCost` — per-unit override of the global wall path cost.
    /// Blank for most units, meaning they use `WALL_COST_BASE`.
    pub wall_movement_cost: Option<i64>,
    pub levels: Vec<CharacterLevel>,
}

impl Character {
    /// Resolves the movement strategy purely from data flags.
    ///
    /// Flag precedence matters: a flying unit ignores walls regardless of any
    /// other flag, so `IsFlying` is checked first.
    pub fn movement_class(&self) -> MovementClass {
        if self.is_flying {
            MovementClass::Air
        } else if self.is_jumper {
            MovementClass::WallJumper
        } else if self.is_underground {
            MovementClass::Underground
        } else {
            MovementClass::Ground
        }
    }

    /// Whether this unit attacks from a distance and so must choose a firing
    /// position rather than walking onto its target.
    ///
    /// Threshold is one tile: melee units in the shipped data sit at 40-100
    /// units, ranged units at 225 and above.
    pub fn is_ranged(&self) -> bool {
        self.attack_range.is_some_and(|r| r > UNITS_PER_TILE)
    }

    /// Additive path cost of crossing a wall for this unit, in game distance
    /// units, given the global default.
    pub fn wall_cost(&self, globals: &PathingGlobals) -> i64 {
        if self.is_jumper {
            // Wall jumpers cross for free; the flag, not a cost column,
            // encodes that in the shipped data.
            0
        } else {
            self.wall_movement_cost
                .unwrap_or(globals.wall_cost_base)
        }
    }

    pub fn max_level(&self) -> u32 {
        self.levels.iter().map(|l| l.level).max().unwrap_or(0)
    }
}

/// Allowed building counts at one town hall level.
#[derive(Debug, Clone)]
pub struct TownHallLevel {
    /// Town hall level, 1-based.
    pub level: u32,
    /// Building name to permitted count at this TH.
    pub building_counts: HashMap<String, i64>,
}

impl TownHallLevel {
    pub fn count_of(&self, building: &str) -> i64 {
        self.building_counts.get(building).copied().unwrap_or(0)
    }

    /// Total placeable buildings at this TH, walls included.
    pub fn total_buildings(&self) -> i64 {
        self.building_counts.values().sum()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DataError {
    #[error("csv error: {0}")]
    Csv(#[from] CsvError),
    #[error("required global `{0}` is missing from globals.csv")]
    MissingGlobal(String),
    #[error("table `{table}` entity `{entity}` level {level}: required field `{field}` is null after carry-over resolution")]
    NullRequiredField {
        table: String,
        entity: String,
        level: u32,
        field: String,
    },
}

/// Reads `buildings.csv` (or `traps.csv`, which shares the schema).
pub fn parse_buildings(table: &Table, table_name: &str) -> Result<Vec<Building>, DataError> {
    let mut out = Vec::with_capacity(table.entities.len());
    for entity in &table.entities {
        let Some(first) = entity.level(1) else { continue };

        let class = first
            .get("BuildingClass")
            .unwrap_or_default()
            .to_string();

        let mut levels = Vec::with_capacity(entity.levels.len());
        for (i, row) in entity.levels.iter().enumerate() {
            let level = (i + 1) as u32;
            // Hitpoints is the one field every placeable entity must have;
            // without it the entity cannot participate in combat at all.
            let hitpoints = row.get_i64("Hitpoints")?.ok_or_else(|| {
                DataError::NullRequiredField {
                    table: table_name.to_string(),
                    entity: entity.name.clone(),
                    level,
                    field: "Hitpoints".into(),
                }
            })?;
            levels.push(BuildingLevel {
                level,
                hitpoints,
                dps: row.get_i64("DPS")?,
                attack_range: row.get_i64("AttackRange")?.map(|v| v as i32),
                min_attack_range: row.get_i64("MinAttackRange")?.map(|v| v as i32),
                attack_speed_ms: row.get_i64("AttackSpeed")?.map(|v| v as i32),
                damage_radius: row.get_i64("DamageRadius")?.map(|v| v as i32),
                alt_attack_range: row.get_i64("AltAttackRange")?.map(|v| v as i32),
                alt_dps: row.get_i64("AltDPS")?,
            });
        }

        out.push(Building {
            name: entity.name.clone(),
            class,
            unlock_th: first.get_i64("TownHallLevel")?,
            width: first.i64_or("Width", 1)? as i32,
            height: first.i64_or("Height", 1)? as i32,
            air_targets: first.bool_or("AirTargets", false)?,
            ground_targets: first.bool_or("GroundTargets", false)?,
            alt_attack_mode: first.bool_or("AltAttackMode", false)?,
            alt_air_targets: first.bool_or("AltAirTargets", false)?,
            alt_ground_targets: first.bool_or("AltGroundTargets", false)?,
            alt_multi_targets: first.bool_or("AltMultiTargets", false)?,
            preferred_target: first.get("PreferredTarget").map(str::to_string),
            preferred_target_damage_mod: first.get_i64("PreferredTargetDamageMod")?,
            projectile: first.get("Projectile").map(str::to_string),
            village_type: first.get_i64("VillageType")?,
            levels,
        })
    }
    Ok(out)
}

/// Reads `characters.csv` (or `heroes.csv`, which shares the relevant columns).
///
/// Note the shipped spelling `PreferedTarget*` with one `r`; the buildings
/// table spells the same concept `PreferredTarget*`. Both spellings are
/// accepted so a future correction upstream does not silently null the field.
pub fn parse_characters(table: &Table) -> Result<Vec<Character>, DataError> {
    let pref_class = if table.has_column("PreferedTargetBuildingClass") {
        "PreferedTargetBuildingClass"
    } else {
        "PreferredTargetBuildingClass"
    };
    let pref_mod = if table.has_column("PreferedTargetDamageMod") {
        "PreferedTargetDamageMod"
    } else {
        "PreferredTargetDamageMod"
    };

    let mut out = Vec::with_capacity(table.entities.len());
    for entity in &table.entities {
        let Some(first) = entity.level(1) else { continue };

        let mut levels = Vec::with_capacity(entity.levels.len());
        for (i, row) in entity.levels.iter().enumerate() {
            levels.push(CharacterLevel {
                level: (i + 1) as u32,
                hitpoints: row.i64_if_present("Hitpoints")?.unwrap_or(0),
                dps: row.i64_if_present("DPS")?,
            });
        }

        // `heroes.csv` omits several flags that `characters.csv` carries, so
        // those are read with the if-present accessors. The ones every unit
        // must have (Speed, Hitpoints) stay strict and are checked by the
        // validation gate.
        out.push(Character {
            name: entity.name.clone(),
            is_flying: first.bool_if_present("IsFlying", false)?,
            is_jumper: first.bool_if_present("IsJumper", false)?,
            is_underground: first.bool_if_present("IsUnderground", false)?,
            triggers_traps: first.bool_if_present("TriggersTraps", true)?,
            speed: first.i64_if_present("Speed")?.map(|v| v as i32),
            attack_range: first.i64_if_present("AttackRange")?.map(|v| v as i32),
            attack_speed_ms: first.i64_if_present("AttackSpeed")?.map(|v| v as i32),
            air_targets: first.bool_if_present("AirTargets", false)?,
            ground_targets: first.bool_if_present("GroundTargets", false)?,
            housing_space: first.i64_if_present("HousingSpace")?.map(|v| v as i32),
            preferred_target_class: first.str_if_present(pref_class).map(str::to_string),
            preferred_target_damage_mod: first.i64_if_present(pref_mod)?,
            wall_movement_cost: first.i64_if_present("WallMovementCost")?,
            levels,
        });
    }
    Ok(out)
}

/// Reads `townhall_levels.csv`.
///
/// Every column whose name matches a known building or trap is a permitted
/// count at that town hall level; all other columns are economy scalars and
/// are skipped. `known` supplies the building/trap names to match against.
///
/// # Cross-level carry-forward
///
/// This table carries a second, table-specific inheritance rule on top of the
/// usual block carry-over. Each town hall level is its own single-row entity,
/// so block carry-over resets at every level and does nothing here. Instead a
/// blank count means *unchanged from the previous town hall level*, not zero:
///
/// ```text
/// TH1  Cannon = 1
/// TH2  Cannon = 2
/// TH3  Cannon =        <- still 2, not 0
/// TH5  Cannon = 3
/// ```
///
/// Reading blanks as zero makes buildings appear to vanish and reappear as the
/// town hall rises, so counts are carried forward across levels in TH order.
pub fn parse_townhall_levels(
    table: &Table,
    known: &[String],
) -> Result<Vec<TownHallLevel>, DataError> {
    let known: std::collections::HashSet<&str> =
        known.iter().map(|s| s.as_str()).collect();
    let count_columns: Vec<&String> = table
        .columns
        .iter()
        .filter(|c| known.contains(c.as_str()))
        .collect();

    // Collect in file order first, then resolve carry-forward in TH order.
    let mut raw: Vec<(u32, HashMap<String, Option<i64>>)> = Vec::new();
    for entity in &table.entities {
        let Some(row) = entity.level(1) else { continue };
        // Entity names in this table are bare level numbers ("1".."18").
        let Ok(level) = entity.name.parse::<u32>() else {
            continue;
        };
        let mut cells = HashMap::new();
        for column in &count_columns {
            cells.insert((*column).clone(), row.get_i64(column)?);
        }
        raw.push((level, cells));
    }
    raw.sort_by_key(|(level, _)| *level);

    let mut out = Vec::with_capacity(raw.len());
    let mut carried: HashMap<String, i64> = HashMap::new();
    for (level, cells) in raw {
        for column in &count_columns {
            match cells.get(*column).copied().flatten() {
                Some(v) => {
                    carried.insert((*column).clone(), v);
                }
                // Blank: keep whatever the previous town hall level allowed.
                None => {
                    carried.entry((*column).clone()).or_insert(0);
                }
            }
        }
        out.push(TownHallLevel {
            level,
            building_counts: carried.clone(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn movement_class_prefers_flying_over_other_flags() {
        let c = Character {
            name: "T".into(),
            is_flying: true,
            is_jumper: true,
            is_underground: false,
            triggers_traps: true,
            speed: None,
            attack_range: None,
            attack_speed_ms: None,
            air_targets: false,
            ground_targets: true,
            housing_space: None,
            preferred_target_class: None,
            preferred_target_damage_mod: None,
            wall_movement_cost: None,
            levels: vec![],
        };
        assert_eq!(c.movement_class(), MovementClass::Air);
    }

    #[test]
    fn wall_cost_falls_back_to_global_when_unset() {
        let g = PathingGlobals {
            target_list_size: 3,
            max_target_list_size: 6,
            wall_cost_base: 1000,
            retarget_after_destroying_wall: false,
            use_wall_weights_for_jump_spell: true,
            underground_speed_percent: 70,
            heat_map_attack_position: true,
            wall_breaker_smart_radius: 2500,
            wall_breaker_smart_cnt_limit: 30,
            wall_breaker_smart_retarget_limit: 2000,
            wall_breaker_use_rooms: false,
        };
        let mut c = Character {
            name: "T".into(),
            is_flying: false,
            is_jumper: false,
            is_underground: false,
            triggers_traps: true,
            speed: None,
            attack_range: None,
            attack_speed_ms: None,
            air_targets: false,
            ground_targets: true,
            housing_space: None,
            preferred_target_class: None,
            preferred_target_damage_mod: None,
            wall_movement_cost: None,
            levels: vec![],
        };
        assert_eq!(c.wall_cost(&g), 1000);

        c.wall_movement_cost = Some(128);
        assert_eq!(c.wall_cost(&g), 128);

        c.is_jumper = true;
        assert_eq!(c.wall_cost(&g), 0, "jumpers cross walls for free");

        assert_eq!(g.wall_cost_tiles(), 10.0);
    }
}
