//! `cargo run --release -p coc-opt --bin dump_layouts -- <profile> <out.json>`
//!
//! Emits seeded and optimized layouts for every town hall level as JSON, which
//! is what the renderer preview is built from. Kept as a binary rather than a
//! throwaway script so the preview can be rebuilt from a known command after
//! the builder changes, instead of being reconstructed from memory.

use anyhow::{bail, Result};
use coc_core::{builder, metrics, Layout, Weights};
use coc_data::GameData;
use coc_opt::{anneal, Config, Geometric};
use serde_json::{json, Value};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let profile = args.get(1).map(String::as_str).unwrap_or("war");
    let out = args.get(2).map(String::as_str).unwrap_or("layouts.json");
    let weights = match profile {
        "war" => Weights::war(),
        "farming" | "farm" => Weights::farming(),
        other => bail!("unknown profile {other}; expected war or farming"),
    };

    let data = GameData::load_default()?;
    let obj = Geometric { weights };

    let mut halls = serde_json::Map::new();
    for th in 1..=data.max_townhall() {
        let seed = builder::seed(&data, th, 0xC0FFEE);
        let seed_metrics = metrics::evaluate(&seed, &weights);
        let outcome = anneal(&seed, &obj, Config { seed: 0xC0FFEE, ..Config::default() });

        halls.insert(
            th.to_string(),
            json!({
                "townhall": th,
                "seed_layout": layout_json(&seed),
                "seed_metrics": metrics_json(&seed_metrics),
                "seed_score": seed_metrics.score,
                "layout": layout_json(&outcome.layout),
                "metrics": metrics_json(&outcome.end),
                "score": outcome.final_score,
                "gain_percent": outcome.gain_percent(),
            }),
        );
        eprintln!(
            "TH{th:<2} seed {:.3} -> {:.3} ({:+.1}%)  walls {}  structures {}",
            seed_metrics.score,
            outcome.final_score,
            outcome.gain_percent(),
            outcome.layout.walls.len(),
            outcome.layout.placements.len()
        );
    }

    let doc = json!({
        "profile": profile,
        "fingerprint": &data.provenance.fingerprint[..12],
        "calibrated": false,
        "note": "Geometric proxy metrics only. The simulator is uncalibrated; \
                 these layouts have never been attacked. See ASSUMPTIONS.md 5.",
        "townhalls": Value::Object(halls),
    });
    std::fs::write(out, serde_json::to_string(&doc)?)?;
    eprintln!("wrote {out}");
    Ok(())
}

fn layout_json(l: &Layout) -> Value {
    json!({
        "townhall": l.townhall,
        "wall_level": l.wall_level,
        "walls": l.walls.iter().map(|&(x, y)| json!([x, y])).collect::<Vec<_>>(),
        "placements": l.placements.iter().map(|p| json!({
            "name": p.name,
            "level": p.level,
            "class": p.class,
            "range": p.range,
            "min_range": p.min_range,
            "is_trap": p.is_trap,
            "rect": { "x": p.rect.x, "y": p.rect.y, "w": p.rect.w, "h": p.rect.h },
        })).collect::<Vec<_>>(),
    })
}

fn metrics_json(m: &metrics::Metrics) -> Value {
    json!({
        "coverage": m.coverage,
        "covered_fraction": m.covered_fraction,
        "th_depth": m.th_depth,
        "enclosed": m.enclosed,
        "loot_protected": m.loot_protected,
        "balance": m.balance,
        "perimeter_safety": m.perimeter_safety,
        "score": m.score,
    })
}
