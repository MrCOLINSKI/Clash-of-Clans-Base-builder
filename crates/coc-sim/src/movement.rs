//! How a unit crosses the map, resolved from data flags.
//!
//! Nothing here decides anything: `characters.csv` already states whether a
//! unit flies, jumps walls, or burrows, and `coc-data` turns those flags into a
//! [`MovementClass`]. This module only attaches the cost the class implies.

use crate::config::PathingConfig;
use coc_data::model::{Character, MovementClass};

/// A unit's traversal rules, ready for the pathfinder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Movement {
    pub class: MovementClass,
    /// Additive cost of crossing one wall tile, in game distance units.
    pub wall_cost: i64,
}

impl Movement {
    /// Resolves a unit's movement from its data flags and the config.
    ///
    /// Precedence for the wall cost, strictest first:
    ///
    /// 1. `IsJumper` — the flag, not a column, is how the shipped data marks a
    ///    unit that crosses walls freely, so it wins outright.
    /// 2. `WallMovementCost` — a per-unit override, blank for most units.
    /// 3. `walls.break_cost` from the config, which itself defaults to the
    ///    shipped `WALL_COST_BASE` but may be a calibration preset.
    ///
    /// The config sits below the per-unit column deliberately: an experiment
    /// re-fitting the *global* wall cost should not silently retune the units
    /// the game gave an explicit figure to.
    pub fn of(unit: &Character, cfg: &PathingConfig) -> Movement {
        let class = unit.movement_class();
        let wall_cost = match class {
            MovementClass::Air | MovementClass::Underground => 0,
            MovementClass::WallJumper => cfg.jumper_cost,
            MovementClass::Ground => unit
                .wall_movement_cost
                .unwrap_or(cfg.wall_break_cost),
        };
        Movement { class, wall_cost }
    }

    /// A plain ground unit paying the configured wall cost.
    pub fn ground(cfg: &PathingConfig) -> Movement {
        Movement {
            class: MovementClass::Ground,
            wall_cost: cfg.wall_break_cost,
        }
    }

    /// Whether the unit travels in a straight line, ignoring the layout.
    ///
    /// True for fliers, and for burrowing units, which tunnel under walls and
    /// buildings alike. Their path cost is geometry, not a search.
    pub fn ignores_obstacles(&self) -> bool {
        matches!(self.class, MovementClass::Air | MovementClass::Underground)
    }

    /// Speed scaling as a percentage of the unit's stated speed.
    ///
    /// Only burrowed units differ, and the factor is extracted rather than
    /// chosen: `UNDERGROUND_UNIT_GROUND_SPEED_PERCENTAGE`.
    pub fn speed_percent(&self, cfg: &PathingConfig) -> i64 {
        match self.class {
            MovementClass::Underground => cfg.underground_speed_percent,
            _ => 100,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::test_globals;
    use coc_data::model::CharacterLevel;

    fn unit(name: &str) -> Character {
        Character {
            name: name.into(),
            is_flying: false,
            is_jumper: false,
            is_underground: false,
            triggers_traps: true,
            speed: Some(16),
            attack_range: Some(60),
            attack_speed_ms: Some(1000),
            air_targets: false,
            ground_targets: true,
            housing_space: Some(1),
            preferred_target_class: None,
            preferred_target_damage_mod: None,
            wall_movement_cost: None,
            levels: vec![CharacterLevel {
                level: 1,
                hitpoints: 100,
                dps: Some(10),
            }],
        }
    }

    fn cfg() -> PathingConfig {
        PathingConfig::load_default(&test_globals()).expect("config loads")
    }

    #[test]
    fn a_plain_ground_unit_pays_the_configured_wall_cost() {
        let c = cfg();
        let m = Movement::of(&unit("Barbarian"), &c);
        assert_eq!(m.class, MovementClass::Ground);
        assert_eq!(m.wall_cost, c.wall_break_cost);
        assert!(!m.ignores_obstacles());
    }

    #[test]
    fn a_jumper_crosses_walls_for_free() {
        let mut u = unit("Hog Rider");
        u.is_jumper = true;
        // Even with an explicit column value, the flag wins.
        u.wall_movement_cost = Some(999);
        let m = Movement::of(&u, &cfg());
        assert_eq!(m.class, MovementClass::WallJumper);
        assert_eq!(m.wall_cost, 0);
    }

    #[test]
    fn a_per_unit_column_beats_the_config_default() {
        let mut u = unit("Odd");
        u.wall_movement_cost = Some(42);
        assert_eq!(Movement::of(&u, &cfg()).wall_cost, 42);
    }

    #[test]
    fn fliers_and_burrowers_ignore_the_layout() {
        let mut air = unit("Dragon");
        air.is_flying = true;
        assert!(Movement::of(&air, &cfg()).ignores_obstacles());

        let mut under = unit("Miner");
        under.is_underground = true;
        let m = Movement::of(&under, &cfg());
        assert!(m.ignores_obstacles());
        assert_eq!(m.speed_percent(&cfg()), 70, "extracted, not chosen");
    }
}
