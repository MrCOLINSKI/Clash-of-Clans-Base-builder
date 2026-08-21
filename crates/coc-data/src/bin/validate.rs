//! `cargo run -p coc-data --bin validate`
//!
//! Fails the build when the cached game data cannot be trusted.

use anyhow::Result;
use coc_data::{validate, GameData};

fn main() -> Result<()> {
    let allow_stale = std::env::args().any(|a| a == "--allow-stale");

    let data = GameData::load_default()?;
    println!("{}", data.provenance.banner());
    println!();

    let report = validate::run(&data, allow_stale);
    println!("{}", report.render());

    if report.failed() {
        eprintln!(
            "\nDATA VALIDATION FAILED — refusing to proceed. \
             Re-run `clashsim extract`, or pass --allow-stale to work against \
             a known-old extraction on purpose."
        );
        std::process::exit(1);
    }
    println!("\nData validation passed.");
    Ok(())
}
