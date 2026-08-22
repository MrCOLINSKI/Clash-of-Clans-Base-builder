//! Optimises a layout for every town hall level and writes them as JSON.
//!
//! Usage: cargo run --release -p coc-opt --example optimize -- <out.json> [profile]

use anyhow::Result;
use coc_core::{builder, legality, metrics::Weights};
use coc_data::GameData;
use coc_opt::{anneal, Config, Geometric};
use rayon::prelude::*;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let out_path = args.get(1).cloned().unwrap_or_else(|| "layouts.json".into());
    let profile = args.get(2).cloned().unwrap_or_else(|| "war".into());

    let data = GameData::load_default()?;
    println!("{}", data.provenance.banner());
    println!("\nSIMULATOR STATUS: UNCALIBRATED — geometric proxy only, not simulated destruction.\n");

    let weights = Weights::named(&profile);
    let levels: Vec<u32> = (1..=data.max_townhall()).collect();

    let t0 = std::time::Instant::now();
    let results: Vec<_> = levels
        .par_iter()
        .map(|&th| {
            let seed = builder::seed(&data, th, 42);
            let obj = Geometric { weights };
            let out = anneal(&seed, &obj, Config { seed: 1000 + th as u64, ..Default::default() });
            (th, seed, out)
        })
        .collect();
    let elapsed = t0.elapsed();

    println!("{:<6}{:>10}{:>10}{:>9}  {:<28}", "TH", "seed", "tuned", "gain", "biggest movers");
    let mut json = serde_json::Map::new();
    for (th, seed_layout, out) in &results {
        // Every emitted layout is re-checked: the optimizer must never be able
        // to produce an illegal base, so this is an assertion, not a filter.
        let v = legality::validate(&out.layout, &data);
        assert!(v.is_empty(), "TH{th} optimized layout illegal: {}", v[0]);

        let movers = biggest_movers(&out.start, &out.end);
        println!(
            "TH{:<4}{:>10.4}{:>10.4}{:>8.1}%  {}",
            th,
            out.start_score,
            out.final_score,
            out.gain_percent(),
            movers
        );

        json.insert(
            th.to_string(),
            serde_json::json!({
                "townhall": th,
                "score": out.final_score,
                "seed_score": out.start_score,
                "gain_percent": out.gain_percent(),
                "metrics": out.end,
                "seed_metrics": out.start,
                "layout": out.layout,
                "seed_layout": seed_layout,
            }),
        );
    }

    let doc = serde_json::json!({
        "version": data.provenance.version,
        "fingerprint": data.provenance.fingerprint,
        "profile": profile,
        "calibrated": false,
        "note": "geometric proxy score, not simulated destruction",
        "townhalls": json,
    });
    std::fs::write(&out_path, serde_json::to_string(&doc)?)?;
    println!(
        "\n{} town halls optimised in {:.2}s, written to {out_path}",
        results.len(),
        elapsed.as_secs_f64()
    );
    Ok(())
}

fn biggest_movers(a: &coc_core::Metrics, b: &coc_core::Metrics) -> String {
    let mut d: Vec<(&str, f64)> = vec![
        ("coverage", b.coverage - a.coverage),
        ("covered", b.covered_fraction - a.covered_fraction),
        ("th_depth", b.th_depth - a.th_depth),
        ("enclosed", b.enclosed - a.enclosed),
        ("loot", b.loot_protected - a.loot_protected),
        ("balance", b.balance - a.balance),
        ("perimeter", b.perimeter_safety - a.perimeter_safety),
    ];
    d.sort_by(|x, y| y.1.abs().partial_cmp(&x.1.abs()).unwrap());
    d.iter()
        .take(2)
        .map(|(n, v)| format!("{n} {v:+.2}"))
        .collect::<Vec<_>>()
        .join(", ")
}
