//! Grid, layout representation, placement legality, and geometric metrics.
//!
//! This is the placement layer. It knows nothing about combat: it knows where
//! things may go, and how to measure the arrangement.

pub mod builder;
pub mod grid;
pub mod legality;
pub mod metrics;

pub use grid::{Layout, Placement, Rect};
pub use metrics::{Metrics, Weights};
