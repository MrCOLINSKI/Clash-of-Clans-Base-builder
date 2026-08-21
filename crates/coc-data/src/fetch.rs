//! Fetching assets from Supercell's CDN into the local cache.
//!
//! # How the fingerprint is obtained
//!
//! Assets live at `https://game-assets.clashofclans.com/<sha>/<path>`, where
//! `<sha>` names an immutable snapshot of the whole asset set. The sha is not
//! discoverable from the CDN — it ships inside the client, at
//! `assets/fingerprint.json` in the APK.
//!
//! Downloading the whole APK to read one file wastes most of a gigabyte, so
//! [`fingerprint_from_apk`] reads the ZIP central directory over HTTP range
//! requests and pulls only that entry. See `docs/EXTRACTION.md`.
//!
//! # Transport
//!
//! Downloads shell out to `curl` rather than linking an HTTP client. This is
//! deliberate: the extraction path runs rarely and often behind a corporate or
//! sandbox proxy with its own CA bundle, and `curl` already honours the
//! standard proxy and certificate environment variables that such setups
//! configure. [`Downloader`] exists so an in-process client can replace it
//! without touching the extraction logic.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Base URL of the asset CDN.
pub const ASSET_HOST: &str = "https://game-assets.clashofclans.com";

/// Where the APK is fetched from when resolving a live fingerprint.
pub const APK_URL: &str =
    "https://d.apkpure.net/b/APK/com.supercell.clashofclans?version=latest";

/// Path of a logic table on the CDN, given its bare table name.
pub fn cdn_path(table: &str) -> String {
    format!("logic/{table}.csv")
}

/// Full CDN URL for one logic table at one fingerprint.
pub fn table_url(fingerprint: &str, table: &str) -> String {
    format!("{ASSET_HOST}/{fingerprint}/{}", cdn_path(table))
}

/// Cache directory for one fingerprint.
pub fn cache_dir(data_root: &Path, fingerprint: &str) -> PathBuf {
    data_root.join("raw").join(fingerprint)
}

/// Byte-range aware fetcher.
pub trait Downloader {
    /// Fetches a whole resource.
    fn get(&self, url: &str) -> Result<Vec<u8>>;
    /// Fetches an inclusive byte range.
    fn get_range(&self, url: &str, start: u64, end: u64) -> Result<Vec<u8>>;
    /// Resolves redirects and reports the final content length.
    fn content_length(&self, url: &str) -> Result<u64>;
}

/// [`Downloader`] backed by the `curl` binary.
pub struct CurlDownloader {
    pub timeout_secs: u32,
}

impl Default for CurlDownloader {
    fn default() -> Self {
        CurlDownloader { timeout_secs: 180 }
    }
}

impl CurlDownloader {
    fn run(&self, args: &[String]) -> Result<Vec<u8>> {
        let out = std::process::Command::new("curl")
            .args(args)
            .output()
            .context("running curl; is it installed and on PATH?")?;
        anyhow::ensure!(
            out.status.success(),
            "curl failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
        Ok(out.stdout)
    }

    fn base_args(&self) -> Vec<String> {
        vec![
            "-sS".into(),
            "-L".into(),
            "--fail".into(),
            "--max-time".into(),
            self.timeout_secs.to_string(),
        ]
    }
}

impl Downloader for CurlDownloader {
    fn get(&self, url: &str) -> Result<Vec<u8>> {
        let mut args = self.base_args();
        args.push(url.to_string());
        self.run(&args)
    }

    fn get_range(&self, url: &str, start: u64, end: u64) -> Result<Vec<u8>> {
        let mut args = self.base_args();
        args.push("-r".into());
        args.push(format!("{start}-{end}"));
        args.push(url.to_string());
        let bytes = self.run(&args)?;
        let want = (end - start + 1) as usize;
        anyhow::ensure!(
            bytes.len() == want,
            "range {start}-{end} returned {} bytes, expected {want}; \
             the server may not honour range requests",
            bytes.len()
        );
        Ok(bytes)
    }

    fn content_length(&self, url: &str) -> Result<u64> {
        let mut args = self.base_args();
        args.extend([
            "-o".into(),
            "/dev/null".into(),
            "-w".into(),
            "%{size_download}".into(),
            "-r".into(),
            "0-0".into(),
            "-D".into(),
            "-".into(),
        ]);
        args.push(url.to_string());
        let out = self.run(&args)?;
        let text = String::from_utf8_lossy(&out);
        // Content-Range on a 206 gives the full size: "bytes 0-0/833740335".
        for line in text.lines() {
            if let Some(rest) = line.to_ascii_lowercase().strip_prefix("content-range:") {
                if let Some((_, total)) = rest.trim().rsplit_once('/') {
                    if let Ok(n) = total.trim().parse::<u64>() {
                        return Ok(n);
                    }
                }
            }
        }
        anyhow::bail!("could not determine content length for {url}")
    }
}

/// The `sha` and `version` read out of a client `fingerprint.json`.
#[derive(Debug, Clone)]
pub struct LiveFingerprint {
    pub sha: String,
    pub version: String,
}

/// Reads `assets/fingerprint.json` out of the published APK.
///
/// Uses range requests to read the ZIP central directory and then only the one
/// entry, rather than downloading the full archive.
pub fn fingerprint_from_apk(dl: &dyn Downloader, apk_url: &str) -> Result<LiveFingerprint> {
    let size = dl
        .content_length(apk_url)
        .context("resolving APK size")?;

    // The end-of-central-directory record lives in the archive tail.
    let tail_len = 131_072u64.min(size);
    let tail = dl
        .get_range(apk_url, size - tail_len, size - 1)
        .context("reading APK tail")?;

    let eocd = rfind(&tail, b"PK\x05\x06")
        .context("no end-of-central-directory record in APK tail")?;
    let mut cd_size = u32::from_le_bytes(tail[eocd + 12..eocd + 16].try_into()?) as u64;
    let mut cd_off = u32::from_le_bytes(tail[eocd + 16..eocd + 20].try_into()?) as u64;

    // ZIP64 escape values mean the real offsets are in the ZIP64 EOCD record.
    if cd_size == u32::MAX as u64 || cd_off == u32::MAX as u64 {
        let z = rfind(&tail, b"PK\x06\x06")
            .context("ZIP64 offsets signalled but no ZIP64 EOCD found")?;
        cd_size = u64::from_le_bytes(tail[z + 40..z + 48].try_into()?);
        cd_off = u64::from_le_bytes(tail[z + 48..z + 56].try_into()?);
    }

    let cd = dl
        .get_range(apk_url, cd_off, cd_off + cd_size - 1)
        .context("reading APK central directory")?;

    let entry = find_central_entry(&cd, "assets/fingerprint.json")
        .context("assets/fingerprint.json not present in APK")?;

    // The local file header repeats the name and extra-field lengths, which
    // may differ from the central directory's, so it must be read.
    let lh = dl.get_range(apk_url, entry.local_header_offset, entry.local_header_offset + 29)?;
    let name_len = u16::from_le_bytes(lh[26..28].try_into()?) as u64;
    let extra_len = u16::from_le_bytes(lh[28..30].try_into()?) as u64;
    let data_start = entry.local_header_offset + 30 + name_len + extra_len;

    let raw = dl.get_range(apk_url, data_start, data_start + entry.compressed_size - 1)?;
    let json = match entry.method {
        0 => raw,
        8 => inflate_raw(&raw).context("inflating fingerprint.json")?,
        m => anyhow::bail!("unsupported ZIP compression method {m}"),
    };

    let parsed: serde_json::Value =
        serde_json::from_slice(&json).context("parsing fingerprint.json from APK")?;
    let sha = parsed["sha"]
        .as_str()
        .context("fingerprint.json has no `sha`")?
        .to_string();
    let version = parsed["version"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();
    Ok(LiveFingerprint { sha, version })
}

#[derive(Debug)]
struct CentralEntry {
    method: u16,
    compressed_size: u64,
    local_header_offset: u64,
}

/// Scans a ZIP central directory for one entry by name.
fn find_central_entry(cd: &[u8], want: &str) -> Option<CentralEntry> {
    let mut p = 0usize;
    while p + 46 <= cd.len() && &cd[p..p + 4] == b"PK\x01\x02" {
        let method = u16::from_le_bytes(cd[p + 10..p + 12].try_into().ok()?);
        let compressed_size = u32::from_le_bytes(cd[p + 20..p + 24].try_into().ok()?) as u64;
        let name_len = u16::from_le_bytes(cd[p + 28..p + 30].try_into().ok()?) as usize;
        let extra_len = u16::from_le_bytes(cd[p + 30..p + 32].try_into().ok()?) as usize;
        let comment_len = u16::from_le_bytes(cd[p + 32..p + 34].try_into().ok()?) as usize;
        let local_header_offset =
            u32::from_le_bytes(cd[p + 42..p + 46].try_into().ok()?) as u64;
        let name = std::str::from_utf8(cd.get(p + 46..p + 46 + name_len)?).ok()?;
        if name == want {
            return Some(CentralEntry {
                method,
                compressed_size,
                local_header_offset,
            });
        }
        p += 46 + name_len + extra_len + comment_len;
    }
    None
}

/// Last occurrence of `needle` in `haystack`.
fn rfind(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=haystack.len() - needle.len())
        .rev()
        .find(|&i| &haystack[i..i + needle.len()] == needle)
}

/// Inflates a raw DEFLATE stream.
///
/// ZIP stores raw deflate with no zlib wrapper. `lzma-rs` does not cover this,
/// so the small amount of work is done through `zstd`'s bundled zlib-free
/// path is not applicable; instead we rely on `flate2` when available. To
/// avoid pulling another dependency for a file that is nearly always STORED,
/// an uncompressed entry is handled directly and DEFLATE is reported clearly.
fn inflate_raw(_data: &[u8]) -> Result<Vec<u8>> {
    anyhow::bail!(
        "assets/fingerprint.json is DEFLATE-compressed in this APK build; \
         this extractor only handles STORED entries. Extract it manually with \
         `unzip -p <apk> assets/fingerprint.json` and pass --fingerprint."
    )
}

/// Downloads every required table for a fingerprint into the cache.
///
/// Files are written exactly as served, still compressed, so the cache is a
/// faithful and reproducible record of what the CDN returned.
pub fn download_tables(
    dl: &dyn Downloader,
    fingerprint: &str,
    tables: &[&str],
    dest: &Path,
) -> Result<Vec<(String, usize)>> {
    let logic = dest.join("logic");
    std::fs::create_dir_all(&logic)
        .with_context(|| format!("creating {}", logic.display()))?;

    let mut written = Vec::new();
    for table in tables {
        let url = table_url(fingerprint, table);
        let bytes = dl
            .get(&url)
            .with_context(|| format!("downloading {url}"))?;
        // Reject an error page masquerading as data before it reaches the cache.
        anyhow::ensure!(
            bytes.len() > 64,
            "{url} returned only {} bytes; refusing to cache",
            bytes.len()
        );
        crate::compress::decompress(&bytes)
            .with_context(|| format!("{url} did not decode as a Supercell asset"))?;
        let path = logic.join(format!("{table}.csv"));
        std::fs::write(&path, &bytes)
            .with_context(|| format!("writing {}", path.display()))?;
        written.push(((*table).to_string(), bytes.len()));
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_cdn_urls() {
        assert_eq!(
            table_url("abc123", "buildings"),
            "https://game-assets.clashofclans.com/abc123/logic/buildings.csv"
        );
    }

    #[test]
    fn rfind_locates_last_match() {
        assert_eq!(rfind(b"xxPKyyPKzz", b"PK"), Some(6));
        assert_eq!(rfind(b"nothing", b"PK"), None);
    }

    /// Builds a minimal central directory and reads one entry back.
    #[test]
    fn parses_central_directory_entry() {
        let name = b"assets/fingerprint.json";
        let mut cd = Vec::new();
        cd.extend_from_slice(b"PK\x01\x02");
        cd.extend_from_slice(&[0u8; 6]); // version fields, flags
        cd.extend_from_slice(&0u16.to_le_bytes()); // method = STORED
        cd.extend_from_slice(&[0u8; 8]); // time, date, crc
        cd.extend_from_slice(&4321u32.to_le_bytes()); // compressed size
        cd.extend_from_slice(&4321u32.to_le_bytes()); // uncompressed size
        cd.extend_from_slice(&(name.len() as u16).to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes()); // extra len
        cd.extend_from_slice(&0u16.to_le_bytes()); // comment len
        cd.extend_from_slice(&[0u8; 8]); // disk, attrs
        cd.extend_from_slice(&9999u32.to_le_bytes()); // local header offset
        cd.extend_from_slice(name);

        let e = find_central_entry(&cd, "assets/fingerprint.json").expect("found");
        assert_eq!(e.method, 0);
        assert_eq!(e.compressed_size, 4321);
        assert_eq!(e.local_header_offset, 9999);
        assert!(find_central_entry(&cd, "assets/other.json").is_none());
    }
}
