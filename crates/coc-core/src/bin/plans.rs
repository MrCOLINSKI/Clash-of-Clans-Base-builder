//! Compare the wall plans across town halls.
use coc_core::builder::{self, Plan};
use coc_core::{metrics, Weights};
use coc_data::GameData;
fn main() -> anyhow::Result<()> {
    let d = GameData::load_default()?;
    println!("{:>4}  {:>26}  {:>26}", "TH", "lattice (walls/score)", "ring (walls/score)");
    for th in 1..=d.max_townhall() {
        let a = builder::seed_with(&d, th, 0xC0FFEE, Plan::Lattice);
        let b = builder::seed_with(&d, th, 0xC0FFEE, Plan::Ring);
        let ma = metrics::evaluate(&a, &Weights::war());
        let mb = metrics::evaluate(&b, &Weights::war());
        let ea = metrics::exposure(&a);
        let eb = metrics::exposure(&b);
        let same = a.walls == b.walls;
        println!(
            "{th:>4}  {:>10} {:.3} c{:.2}  {:>10} {:.3} c{:.2}  {}",
            a.walls.len(), ma.score, ea.compactness,
            b.walls.len(), mb.score, eb.compactness,
            if same { "RING FELL BACK" } else { "" }
        );
    }
    Ok(())
}
