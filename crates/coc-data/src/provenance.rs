//! Where the loaded data came from, and how much it should be trusted.
//!
//! Every simulator and optimizer run prints this banner. A result produced
//! from stale or unverified data is worth less than no result at all, so the
//! provenance travels with the data rather than being looked up separately.

use serde::{Deserialize, Serialize};

/// Asset manifest shipped inside the APK and mirrored on the asset server.
#[derive(Debug, Clone, Deserialize)]
pub struct Fingerprint {
    /// Content hash naming the asset directory on the CDN.
    pub sha: String,
    /// Asset bundle version, e.g. `18.400.11`.
    pub version: String,
    #[serde(default)]
    pub files: Vec<FingerprintFile>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FingerprintFile {
    pub file: String,
    pub sha: String,
}

/// Record written when assets were extracted, used to judge staleness.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Extraction {
    /// Fingerprint sha the assets were pulled from.
    pub fingerprint: String,
    /// Asset bundle version of those assets.
    pub version: String,
    /// RFC 3339 date the extraction ran.
    pub extracted_at: String,
    /// How the fingerprint was obtained, for auditability.
    pub source: String,
    /// Live version seen when freshness was last confirmed, if ever.
    #[serde(default)]
    pub live_version_checked: Option<String>,
    /// RFC 3339 date of that confirmation.
    #[serde(default)]
    pub live_checked_at: Option<String>,
}

/// Whether the loaded assets are known to match the live game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// A live check confirmed the cached version is current.
    Current,
    /// A live check found a newer version than the cache holds.
    Stale,
    /// No live check has been performed; currency is unknown.
    Unverified,
}

impl Freshness {
    pub fn label(self) -> &'static str {
        match self {
            Freshness::Current => "CURRENT",
            Freshness::Stale => "STALE",
            Freshness::Unverified => "UNVERIFIED",
        }
    }
}

/// Full provenance of a loaded [`crate::GameData`].
#[derive(Debug, Clone)]
pub struct Provenance {
    pub fingerprint: String,
    pub version: String,
    pub extracted_at: String,
    pub source: String,
    pub freshness: Freshness,
    pub live_version: Option<String>,
    /// Tables loaded, with their decoded byte counts.
    pub tables: Vec<(String, usize)>,
}

impl Provenance {
    pub fn from_extraction(e: &Extraction) -> Provenance {
        let freshness = match &e.live_version_checked {
            None => Freshness::Unverified,
            Some(live) if *live == e.version => Freshness::Current,
            Some(_) => Freshness::Stale,
        };
        Provenance {
            fingerprint: e.fingerprint.clone(),
            version: e.version.clone(),
            extracted_at: e.extracted_at.clone(),
            source: e.source.clone(),
            freshness,
            live_version: e.live_version_checked.clone(),
            tables: Vec::new(),
        }
    }

    /// Renders the banner printed at the top of every run.
    ///
    /// Required by the project rules on every optimizer and simulator run, so
    /// that no output can be read without its data lineage attached.
    pub fn banner(&self) -> String {
        let mut s = String::new();
        s.push_str("+---------------------------------------------------------------+\n");
        s.push_str("| GAME DATA PROVENANCE                                          |\n");
        s.push_str("+---------------------------------------------------------------+\n");
        s.push_str(&format!("| version      : {:<46} |\n", self.version));
        s.push_str(&format!("| fingerprint  : {:<46} |\n", self.fingerprint));
        s.push_str(&format!("| extracted    : {:<46} |\n", self.extracted_at));
        s.push_str(&format!("| source       : {:<46} |\n", truncate(&self.source, 46)));
        let fresh = match &self.live_version {
            Some(v) => format!("{} (live {})", self.freshness.label(), v),
            None => format!("{} (no live check recorded)", self.freshness.label()),
        };
        s.push_str(&format!("| freshness    : {fresh:<46} |\n"));
        s.push_str(&format!(
            "| tables       : {:<46} |\n",
            format!("{} loaded", self.tables.len())
        ));
        s.push_str("+---------------------------------------------------------------+");
        s
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extraction(live: Option<&str>) -> Extraction {
        Extraction {
            fingerprint: "abc".into(),
            version: "18.400.11".into(),
            extracted_at: "2026-08-21T00:00:00Z".into(),
            source: "apk".into(),
            live_version_checked: live.map(str::to_string),
            live_checked_at: None,
        }
    }

    #[test]
    fn freshness_reflects_live_check() {
        assert_eq!(
            Provenance::from_extraction(&extraction(None)).freshness,
            Freshness::Unverified
        );
        assert_eq!(
            Provenance::from_extraction(&extraction(Some("18.400.11"))).freshness,
            Freshness::Current
        );
        assert_eq!(
            Provenance::from_extraction(&extraction(Some("18.500.1"))).freshness,
            Freshness::Stale
        );
    }

    #[test]
    fn banner_names_version_and_freshness() {
        let b = Provenance::from_extraction(&extraction(None)).banner();
        assert!(b.contains("18.400.11"));
        assert!(b.contains("UNVERIFIED"));
    }
}
