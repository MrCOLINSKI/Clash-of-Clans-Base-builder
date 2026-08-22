//! Pathfinding, targeting, and (from phase 4) combat.
//!
//! The layering is deliberate. `coc-core` knows where things may go;
//! this crate knows how a unit gets to them and what it picks. Nothing here
//! reads game data directly — the caller loads it once and passes what it needs
//! — and nothing here holds a balance constant, because every one of them
//! belongs either to the extracted data or to `config/pathing.toml`.
//!
//! Determinism is a hard requirement, not an aspiration. Two runs on the same
//! inputs must produce the same path, the same target, and the same cost, on
//! any machine. That rules out floating point in the cost model, iteration over
//! hash maps, and any tie broken by insertion order.

pub mod config;
pub mod movement;
pub mod navgrid;
pub mod path;
pub mod target;

pub use config::PathingConfig;
pub use movement::Movement;
pub use navgrid::{Cell, NavGrid};
pub use path::CostField;
pub use target::{Acquired, Preference, Target};
