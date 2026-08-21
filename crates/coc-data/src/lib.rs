//! Extraction, decoding, and typing of Clash of Clans game data.
//!
//! This crate is the single source of balance truth for the workspace. No
//! other crate may hardcode a hitpoint total, damage figure, footprint, range,
//! cooldown, or per-town-hall building count; all of it is read from the
//! shipped logic tables through the types here.
//!
//! Assets are cached under `data/raw/<fingerprint>/` in their original
//! compressed form and committed, so a given fingerprint always decodes to
//! byte-identical inputs and any run is reproducible offline.

pub mod compress;
pub mod csv;
pub mod fetch;
pub mod model;
pub mod provenance;
pub mod validate;

use anyhow::{Context, Result};
use model::{Building, Character, Globals, PathingGlobals, TownHallLevel, Trap};
use provenance::{Extraction, Fingerprint, Provenance};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Logic tables this project requires.
///
/// Names are the paths used on the asset CDN, which differ from the paths
/// inside the APK (`logic/` on the CDN, `assets/csv_logic/` in the APK).
pub const REQUIRED_TABLES: &[&str] = &[
    "buildings",
    "characters",
    "heroes",
    "spells",
    "traps",
    "townhall_levels",
    "globals",
    "projectiles",
    "pets",
    "character_items",
    "super_licences",
    "building_classes",
];

/// Everything loaded from one asset fingerprint.
pub struct GameData {
    pub provenance: Provenance,
    /// Raw parsed tables, keyed by table name, for access to columns the
    /// typed views do not yet expose.
    pub tables: BTreeMap<String, csv::Table>,
    pub globals: Globals,
    pub pathing: PathingGlobals,
    /// Buildings, walls, and the town hall.
    pub buildings: Vec<Building>,
    /// Traps, which use their own schema (no hitpoints).
    pub traps: Vec<Trap>,
    pub characters: Vec<Character>,
    pub heroes: Vec<Character>,
    pub townhall_levels: Vec<TownHallLevel>,
}

impl GameData {
    /// Loads the newest cached extraction under `data/raw/`.
    pub fn load_default() -> Result<GameData> {
        let root = locate_data_root()?;
        Self::load_latest(&root)
    }

    /// Loads the most recently extracted fingerprint under `data_root`.
    pub fn load_latest(data_root: &Path) -> Result<GameData> {
        let raw = data_root.join("raw");
        let mut candidates: Vec<(String, PathBuf)> = std::fs::read_dir(&raw)
            .with_context(|| format!("reading {}", raw.display()))?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .filter(|e| e.path().join("extraction.json").exists())
            .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
            .collect();
        anyhow::ensure!(
            !candidates.is_empty(),
            "no extraction found under {}; run `clashsim extract` first",
            raw.display()
        );

        // Newest by recorded extraction date, falling back to directory name.
        candidates.sort_by_key(|(name, path)| {
            let date = std::fs::read_to_string(path.join("extraction.json"))
                .ok()
                .and_then(|s| serde_json::from_str::<Extraction>(&s).ok())
                .map(|e| e.extracted_at)
                .unwrap_or_default();
            (date, name.clone())
        });
        let (_, newest) = candidates.pop().expect("non-empty");
        Self::load_from(&newest)
    }

    /// Loads one extraction directory.
    pub fn load_from(dir: &Path) -> Result<GameData> {
        let extraction: Extraction = serde_json::from_str(
            &std::fs::read_to_string(dir.join("extraction.json"))
                .with_context(|| format!("reading {}/extraction.json", dir.display()))?,
        )
        .context("parsing extraction.json")?;

        let mut provenance = Provenance::from_extraction(&extraction);
        let mut tables = BTreeMap::new();

        for name in REQUIRED_TABLES {
            let path = dir.join("logic").join(format!("{name}.csv"));
            let blob = std::fs::read(&path)
                .with_context(|| format!("reading table {}", path.display()))?;
            let decoded = compress::decompress(&blob)
                .with_context(|| format!("decompressing {}", path.display()))?;
            let text = String::from_utf8_lossy(&decoded.bytes);
            let table = csv::Table::parse(&text)
                .with_context(|| format!("parsing {}", path.display()))?;
            provenance
                .tables
                .push(((*name).to_string(), decoded.bytes.len()));
            tables.insert((*name).to_string(), table);
        }

        let get = |n: &str| -> Result<&csv::Table> {
            tables
                .get(n)
                .ok_or_else(|| anyhow::anyhow!("table `{n}` not loaded"))
        };

        let globals = Globals::parse(get("globals")?).context("parsing globals")?;
        let pathing = PathingGlobals::from_globals(&globals)
            .context("extracting pathing globals from globals.csv")?;
        let buildings = model::parse_buildings(get("buildings")?, "buildings")
            .context("parsing buildings")?;
        let traps = model::parse_traps(get("traps")?).context("parsing traps")?;
        let characters =
            model::parse_characters(get("characters")?).context("parsing characters")?;
        let heroes = model::parse_characters(get("heroes")?).context("parsing heroes")?;

        let known: Vec<String> = buildings
            .iter()
            .map(|b| b.name.clone())
            .chain(traps.iter().map(|t| t.name.clone()))
            .collect();
        let townhall_levels =
            model::parse_townhall_levels(get("townhall_levels")?, &known)
                .context("parsing townhall_levels")?;

        Ok(GameData {
            provenance,
            tables,
            globals,
            pathing,
            buildings,
            traps,
            characters,
            heroes,
            townhall_levels,
        })
    }

    pub fn building(&self, name: &str) -> Option<&Building> {
        self.buildings.iter().find(|b| b.name == name)
    }

    pub fn trap(&self, name: &str) -> Option<&Trap> {
        self.traps.iter().find(|b| b.name == name)
    }

    pub fn character(&self, name: &str) -> Option<&Character> {
        self.characters.iter().find(|c| c.name == name)
    }

    /// Buildings placeable at a given town hall level, with their counts.
    pub fn townhall(&self, level: u32) -> Option<&TownHallLevel> {
        self.townhall_levels.iter().find(|t| t.level == level)
    }

    /// Highest town hall level present in the data.
    pub fn max_townhall(&self) -> u32 {
        self.townhall_levels
            .iter()
            .map(|t| t.level)
            .max()
            .unwrap_or(0)
    }

    /// Reads the shipped asset manifest, if it was cached alongside the tables.
    pub fn fingerprint_manifest(dir: &Path) -> Result<Fingerprint> {
        let text = std::fs::read_to_string(dir.join("fingerprint.json"))
            .with_context(|| format!("reading {}/fingerprint.json", dir.display()))?;
        serde_json::from_str(&text).context("parsing fingerprint.json")
    }
}

/// Finds the workspace `data/` directory by walking up from the current
/// executable's manifest location, then the working directory.
pub fn locate_data_root() -> Result<PathBuf> {
    if let Ok(explicit) = std::env::var("CLASHSIM_DATA_DIR") {
        return Ok(PathBuf::from(explicit));
    }
    // From a crate directory, `data/` sits two levels up at the workspace root.
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    for base in [manifest.join("../.."), manifest.to_path_buf()] {
        let candidate = base.join("data");
        if candidate.join("raw").is_dir() {
            return Ok(candidate.canonicalize().unwrap_or(candidate));
        }
    }
    let mut cwd = std::env::current_dir()?;
    loop {
        let candidate = cwd.join("data");
        if candidate.join("raw").is_dir() {
            return Ok(candidate);
        }
        if !cwd.pop() {
            break;
        }
    }
    anyhow::bail!("could not locate a data/raw directory; set CLASHSIM_DATA_DIR")
}
