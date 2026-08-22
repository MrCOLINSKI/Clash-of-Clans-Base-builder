//! Loading `config/pathing.toml`.
//!
//! RULE 2 of this project: no pathfinding or targeting constant may be written
//! in Rust source. Every number the pathfinder uses arrives here, either as a
//! literal in the TOML or — the usual case — as the string `"from_data"`,
//! meaning "whatever the shipped `globals.csv` says". This module is the only
//! place the two are reconciled.
//!
//! The reason for the indirection is that the extracted value must always win
//! by default. A hand-written default that happens to match today's game data
//! silently stops matching when the game changes, and nothing fails. A
//! `"from_data"` marker cannot go stale, because it is not a value.
//!
//! Presets are the third form: a bare string that is not `"from_data"` names an
//! entry under `[walls.presets]`. That exists for the wall-cost conflict
//! recorded in ASSUMPTIONS.md 2.2, where the shipped constant and a 2017
//! community measurement disagree and the calibration harness must be able to
//! re-fit against either.

use coc_data::model::PathingGlobals;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// A setting that may defer to the shipped game data, name a preset, or state
/// a literal value.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Setting {
    /// A literal number written in the TOML.
    Value(i64),
    /// `"from_data"`, or the name of a preset.
    Name(String),
}

/// A boolean setting, which may also defer to the shipped data.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum BoolSetting {
    Value(bool),
    Name(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("reading {0}: {1}")]
    Io(String, std::io::Error),
    #[error("parsing {0}: {1}")]
    Toml(String, toml::de::Error),
    #[error("`{key}` is set to \"{name}\", which is neither \"from_data\" nor a known preset")]
    UnknownName { key: String, name: String },
    #[error("`{key}` is set to \"from_data\" but the shipped data has no value for it")]
    NoDataValue { key: String },
    #[error("`{key}` = {value} is not usable: {why}")]
    Unusable {
        key: String,
        value: i64,
        why: &'static str,
    },
}

/// The raw TOML shape. Resolved into [`PathingConfig`] before use.
#[derive(Debug, Deserialize)]
struct Raw {
    targeting: RawTargeting,
    walls: RawWalls,
    grid: RawGrid,
    movement: RawMovement,
    attack_position: RawAttackPosition,
    wall_breaker: RawWallBreaker,
    lifecycle: Lifecycle,
}

#[derive(Debug, Deserialize)]
struct RawTargeting {
    rule_of_n: Setting,
    max_rule_of_n: Setting,
    defense_retarget_delay_ms: i64,
}

#[derive(Debug, Deserialize)]
struct RawWalls {
    break_cost: Setting,
    cost_scales_with_hp: bool,
    jumper_cost: i64,
    retarget_after_destroying_wall: BoolSetting,
    use_wall_weights_for_jump_spell: BoolSetting,
    #[serde(default)]
    presets: HashMap<String, i64>,
}

#[derive(Debug, Deserialize)]
struct RawGrid {
    subtiles_per_tile: i32,
    units_per_tile: i32,
}

#[derive(Debug, Deserialize)]
struct RawMovement {
    underground_speed_percent: Setting,
}

#[derive(Debug, Deserialize)]
struct RawAttackPosition {
    strategy: String,
    discard_unreachable_targets: bool,
}

#[derive(Debug, Deserialize)]
struct RawWallBreaker {
    smart_radius: Setting,
    smart_count_limit: Setting,
    smart_retarget_limit: Setting,
    use_rooms: BoolSetting,
}

/// When a unit recomputes its path. A path is otherwise computed once, on
/// target acquisition, and then followed.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Lifecycle {
    pub recompute_on_target_destroyed: bool,
    pub recompute_on_displacement: bool,
    pub recompute_on_jump_spell: bool,
    pub recompute_on_spawn: bool,
    pub mass_retarget_benchmark_units: usize,
}

/// Wall Breaker special targeting, all of it extracted.
#[derive(Debug, Clone, Copy)]
pub struct WallBreaker {
    pub smart_radius: i64,
    pub smart_count_limit: i64,
    pub smart_retarget_limit: i64,
    pub use_rooms: bool,
}

/// How a ranged unit picks the square it fires from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttackPosition {
    /// The nearest reachable subtile from which the target is in range. An
    /// approximation of the client's heat map — see ASSUMPTIONS.md 4.2.
    NearestReachableInRange,
}

/// Every constant the pathfinder and the target selector use.
#[derive(Debug, Clone)]
pub struct PathingConfig {
    /// Stage-1 candidate count, the "rule of N".
    pub rule_of_n: usize,
    /// Ceiling the client will grow the candidate list to.
    pub max_rule_of_n: usize,
    /// Delay between a defence losing its target and acquiring the next.
    pub defense_retarget_delay_ms: i64,
    /// Additive path cost of crossing one wall, in game distance units.
    pub wall_break_cost: i64,
    /// Whether that cost scales with the wall's hitpoints. The data says no.
    pub wall_cost_scales_with_hp: bool,
    /// Cost for units flagged `IsJumper`.
    pub jumper_cost: i64,
    pub retarget_after_destroying_wall: bool,
    pub use_wall_weights_for_jump_spell: bool,
    /// Navigation grid resolution.
    pub subtiles_per_tile: i32,
    /// Game distance units per tile.
    pub units_per_tile: i32,
    pub underground_speed_percent: i64,
    pub attack_position: AttackPosition,
    pub discard_unreachable_targets: bool,
    pub wall_breaker: WallBreaker,
    pub lifecycle: Lifecycle,
}

impl PathingConfig {
    /// Loads and resolves `config/pathing.toml` from the repository root.
    pub fn load_default(globals: &PathingGlobals) -> Result<PathingConfig, ConfigError> {
        Self::load(default_path(), globals)
    }

    pub fn load(path: impl AsRef<Path>, globals: &PathingGlobals) -> Result<PathingConfig, ConfigError> {
        let path = path.as_ref();
        let shown = path.display().to_string();
        let text = std::fs::read_to_string(path).map_err(|e| ConfigError::Io(shown.clone(), e))?;
        let raw: Raw = toml::from_str(&text).map_err(|e| ConfigError::Toml(shown, e))?;
        raw.resolve(globals)
    }

    /// Cost of one orthogonal step between adjacent subtiles.
    pub fn step_cost(&self) -> i32 {
        self.units_per_tile / self.subtiles_per_tile
    }

    /// Cost of one diagonal step: `step_cost * sqrt(2)`, integral.
    ///
    /// Fixed-point rather than floating: the simulator has to produce the same
    /// bytes on every machine, and a float here would put rounding into every
    /// path cost comparison.
    pub fn diagonal_cost(&self) -> i32 {
        // 1.414213... to four places, which is exact enough that no realistic
        // path length accumulates a one-unit error against `f64`.
        (self.step_cost() as i64 * 14_142 / 10_000) as i32
    }

    /// Nav grid extent in subtiles, for a playfield of `tiles` tiles.
    pub fn nav_extent(&self, tiles: i32) -> i32 {
        tiles * self.subtiles_per_tile
    }
}

impl Raw {
    fn resolve(self, g: &PathingGlobals) -> Result<PathingConfig, ConfigError> {
        let presets = &self.walls.presets;

        let rule_of_n = resolve_num(
            "targeting.rule_of_n",
            &self.targeting.rule_of_n,
            presets,
            Some(g.target_list_size),
        )?;
        let max_rule_of_n = resolve_num(
            "targeting.max_rule_of_n",
            &self.targeting.max_rule_of_n,
            presets,
            Some(g.max_target_list_size),
        )?;
        if rule_of_n < 1 {
            return Err(ConfigError::Unusable {
                key: "targeting.rule_of_n".into(),
                value: rule_of_n,
                why: "a unit must consider at least one candidate",
            });
        }
        if max_rule_of_n < rule_of_n {
            return Err(ConfigError::Unusable {
                key: "targeting.max_rule_of_n".into(),
                value: max_rule_of_n,
                why: "the ceiling cannot be below the default candidate count",
            });
        }

        let wall_break_cost = resolve_num(
            "walls.break_cost",
            &self.walls.break_cost,
            presets,
            Some(g.wall_cost_base),
        )?;

        let subtiles = self.grid.subtiles_per_tile;
        let units = self.grid.units_per_tile;
        if subtiles < 1 {
            return Err(ConfigError::Unusable {
                key: "grid.subtiles_per_tile".into(),
                value: subtiles as i64,
                why: "the nav grid needs at least one cell per tile",
            });
        }
        if units < 1 || units % subtiles != 0 {
            // An indivisible ratio would make one step cost a fraction of a
            // unit, and the whole cost model is integral.
            return Err(ConfigError::Unusable {
                key: "grid.units_per_tile".into(),
                value: units as i64,
                why: "must divide evenly by subtiles_per_tile",
            });
        }

        let attack_position = match self.attack_position.strategy.as_str() {
            "nearest_reachable_in_range" => AttackPosition::NearestReachableInRange,
            other => {
                return Err(ConfigError::UnknownName {
                    key: "attack_position.strategy".into(),
                    name: other.to_string(),
                })
            }
        };

        Ok(PathingConfig {
            rule_of_n: rule_of_n as usize,
            max_rule_of_n: max_rule_of_n as usize,
            defense_retarget_delay_ms: self.targeting.defense_retarget_delay_ms,
            wall_break_cost,
            wall_cost_scales_with_hp: self.walls.cost_scales_with_hp,
            jumper_cost: self.walls.jumper_cost,
            retarget_after_destroying_wall: resolve_bool(
                "walls.retarget_after_destroying_wall",
                &self.walls.retarget_after_destroying_wall,
                g.retarget_after_destroying_wall,
            )?,
            use_wall_weights_for_jump_spell: resolve_bool(
                "walls.use_wall_weights_for_jump_spell",
                &self.walls.use_wall_weights_for_jump_spell,
                g.use_wall_weights_for_jump_spell,
            )?,
            subtiles_per_tile: subtiles,
            units_per_tile: units,
            underground_speed_percent: resolve_num(
                "movement.underground_speed_percent",
                &self.movement.underground_speed_percent,
                presets,
                Some(g.underground_speed_percent),
            )?,
            attack_position,
            discard_unreachable_targets: self.attack_position.discard_unreachable_targets,
            wall_breaker: WallBreaker {
                smart_radius: resolve_num(
                    "wall_breaker.smart_radius",
                    &self.wall_breaker.smart_radius,
                    presets,
                    Some(g.wall_breaker_smart_radius),
                )?,
                smart_count_limit: resolve_num(
                    "wall_breaker.smart_count_limit",
                    &self.wall_breaker.smart_count_limit,
                    presets,
                    Some(g.wall_breaker_smart_cnt_limit),
                )?,
                smart_retarget_limit: resolve_num(
                    "wall_breaker.smart_retarget_limit",
                    &self.wall_breaker.smart_retarget_limit,
                    presets,
                    Some(g.wall_breaker_smart_retarget_limit),
                )?,
                use_rooms: resolve_bool(
                    "wall_breaker.use_rooms",
                    &self.wall_breaker.use_rooms,
                    g.wall_breaker_use_rooms,
                )?,
            },
            lifecycle: self.lifecycle,
        })
    }
}

/// Marker meaning "take the shipped value".
const FROM_DATA: &str = "from_data";

fn resolve_num(
    key: &str,
    setting: &Setting,
    presets: &HashMap<String, i64>,
    from_data: Option<i64>,
) -> Result<i64, ConfigError> {
    match setting {
        Setting::Value(v) => Ok(*v),
        Setting::Name(n) if n == FROM_DATA => from_data.ok_or_else(|| ConfigError::NoDataValue {
            key: key.to_string(),
        }),
        Setting::Name(n) => presets
            .get(n)
            .copied()
            .ok_or_else(|| ConfigError::UnknownName {
                key: key.to_string(),
                name: n.clone(),
            }),
    }
}

fn resolve_bool(key: &str, setting: &BoolSetting, from_data: bool) -> Result<bool, ConfigError> {
    match setting {
        BoolSetting::Value(v) => Ok(*v),
        BoolSetting::Name(n) if n == FROM_DATA => Ok(from_data),
        BoolSetting::Name(n) => Err(ConfigError::UnknownName {
            key: key.to_string(),
            name: n.clone(),
        }),
    }
}

/// `config/pathing.toml`, relative to the repository root.
fn default_path() -> std::path::PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "..", "..", "config", "pathing.toml"]
        .iter()
        .collect()
}

#[cfg(test)]
pub(crate) fn test_globals() -> PathingGlobals {
    PathingGlobals {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_config_resolves_against_shipped_data() {
        let g = test_globals();
        let c = PathingConfig::load_default(&g).expect("config/pathing.toml loads");

        // Every `from_data` marker must have taken the extracted value.
        assert_eq!(c.rule_of_n, 3, "TARGET_LIST_SIZE");
        assert_eq!(c.max_rule_of_n, 6, "MAX_TARGET_LIST_SIZE");
        assert_eq!(c.wall_break_cost, 1000, "WALL_COST_BASE");
        assert!(!c.retarget_after_destroying_wall);
        assert!(c.use_wall_weights_for_jump_spell);
        assert_eq!(c.underground_speed_percent, 70);
        assert_eq!(c.wall_breaker.smart_radius, 2500);
        assert!(!c.wall_breaker.use_rooms);
    }

    #[test]
    fn from_data_tracks_the_data_rather_than_a_written_default() {
        // The point of the marker: change the shipped value and the config
        // follows, with nothing in the TOML or the source edited.
        let mut g = test_globals();
        g.target_list_size = 5;
        g.wall_cost_base = 1234;
        let c = PathingConfig::load_default(&g).expect("loads");
        assert_eq!(c.rule_of_n, 5);
        assert_eq!(c.wall_break_cost, 1234);
    }

    #[test]
    fn a_named_preset_overrides_the_shipped_constant() {
        let toml = sample_toml("break_cost = \"community_2017\"");
        let c = parse(&toml).expect("preset resolves");
        assert_eq!(c.wall_break_cost, 1550, "the 2017 community measurement");
    }

    #[test]
    fn a_literal_overrides_everything() {
        let c = parse(&sample_toml("break_cost = 777")).expect("literal resolves");
        assert_eq!(c.wall_break_cost, 777);
    }

    #[test]
    fn an_unknown_preset_name_is_an_error_not_a_silent_default() {
        let e = parse(&sample_toml("break_cost = \"gut_feeling\"")).unwrap_err();
        assert!(matches!(e, ConfigError::UnknownName { .. }), "{e}");
    }

    #[test]
    fn step_costs_are_integral_and_diagonal_matches_sqrt_two() {
        let c = PathingConfig::load_default(&test_globals()).expect("loads");
        assert_eq!(c.step_cost(), 50, "100 units per tile, 2 subtiles");
        let exact = 50.0 * std::f64::consts::SQRT_2;
        assert_eq!(c.diagonal_cost(), 70);
        assert!(
            (c.diagonal_cost() as f64 - exact).abs() < 1.0,
            "fixed-point diagonal should be within a unit of sqrt(2)"
        );
    }

    #[test]
    fn an_indivisible_grid_ratio_is_rejected() {
        let toml = sample_toml("break_cost = \"from_data\"")
            .replace("subtiles_per_tile = 2", "subtiles_per_tile = 3");
        let e = parse(&toml).unwrap_err();
        assert!(matches!(e, ConfigError::Unusable { .. }), "{e}");
    }

    /// A minimal but complete config, with the walls line substituted.
    fn sample_toml(break_cost_line: &str) -> String {
        format!(
            r#"
[targeting]
rule_of_n = "from_data"
max_rule_of_n = "from_data"
defense_retarget_delay_ms = 0

[walls]
{break_cost_line}
cost_scales_with_hp = false
jumper_cost = 0
retarget_after_destroying_wall = "from_data"
use_wall_weights_for_jump_spell = "from_data"

[walls.presets]
community_2017 = 1550
shipped = 1000

[grid]
subtiles_per_tile = 2
units_per_tile = 100

[movement]
underground_speed_percent = "from_data"

[attack_position]
strategy = "nearest_reachable_in_range"
discard_unreachable_targets = true

[wall_breaker]
smart_radius = "from_data"
smart_count_limit = "from_data"
smart_retarget_limit = "from_data"
use_rooms = "from_data"

[lifecycle]
recompute_on_target_destroyed = true
recompute_on_displacement = true
recompute_on_jump_spell = true
recompute_on_spawn = true
mass_retarget_benchmark_units = 250
"#
        )
    }

    fn parse(text: &str) -> Result<PathingConfig, ConfigError> {
        let raw: Raw = toml::from_str(text).map_err(|e| ConfigError::Toml("<test>".into(), e))?;
        raw.resolve(&test_globals())
    }
}
