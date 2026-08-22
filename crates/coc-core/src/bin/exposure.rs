//! `cargo run -p coc-core --bin exposure`
//!
//! Measured exposure per town hall, for pairing with `data/meta/attacks.json`.
use coc_core::{builder, metrics};
use coc_data::GameData;
fn main() -> anyhow::Result<()> {
    let d = GameData::load_default()?;
    println!("{:>4} {:>10} {:>13} {:>12}  leans", "TH", "air cover", "ground cover", "compactness");
    for th in 1..=d.max_townhall() {
        let l = builder::seed(&d, th, 0xC0FFEE);
        let e = metrics::exposure(&l);
        let leans = if e.compactness >= 0.55 {
            "compact core"
        } else if e.compactness <= 0.40 {
            "spread"
        } else {
            "balanced"
        };
        println!(
            "{th:>4} {:>9.0}% {:>12.0}% {:>12.2}  {leans}",
            e.air_cover * 100.0,
            e.ground_cover * 100.0,
            e.compactness
        );
    }
    Ok(())
}
