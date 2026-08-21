# Game data extraction

How `coc-data` turns Supercell's asset server into typed Rust structs, and the
non-obvious details that break naive implementations.

## Pipeline

```
APK (~795 MB)  --range reads-->  assets/fingerprint.json  -->  sha
                                                               |
                        game-assets.clashofclans.com/<sha>/logic/*.csv
                                                               |
                          Sig: header (68 bytes) stripped      |
                                                               v
                              LZMA / LZHAM / ZSTD  -->  CSV text
                                                               |
                                    carry-over parser  -->  typed structs
                                                               |
                                          validation gate  -->  GameData
```

## 1. Resolving the fingerprint

Assets are served from `https://game-assets.clashofclans.com/<sha>/`, where
`<sha>` names an immutable snapshot. The sha is not discoverable from the CDN;
requesting the host root returns `AccessDenied`. It ships inside the client at
`assets/fingerprint.json`.

Downloading a 795 MB APK to read a 976 KB file is wasteful, so
`fetch::fingerprint_from_apk` reads it with HTTP range requests:

1. `GET` the last 128 KB, scan backwards for the `PK\x05\x06` end-of-central-directory record.
2. Read the central directory offset and size from it (falling back to the ZIP64 record if the fields are escaped to `0xFFFFFFFF`).
3. `GET` just the central directory, scan it for `assets/fingerprint.json`.
4. `GET` the local file header for that entry — its name and extra-field lengths can differ from the central directory's, so the data offset must be computed from it.
5. `GET` only the entry's bytes.

Roughly 1.2 MB transferred instead of 795 MB.

The entry is `STORED` in current builds. A `DEFLATE` entry is reported with
instructions rather than handled.

## 2. Paths differ between the APK and the CDN

Inside the APK the tables are under `assets/csv_logic/`. On the CDN they are
under `logic/`. Requesting the APK-style path returns 403.

Several files are also named differently from what community documentation
suggests:

| Commonly cited | Actual |
|---|---|
| `client_globals.csv` | `logic/globals.csv` |
| `supers.csv` | `logic/super_licences.csv` |
| `equipment.csv` | `logic/character_items.csv` |

## 3. The `Sig:` header comes first

Every `logic/*.csv` in 18.400.11 begins with a 68-byte signature header — a
4-byte `Sig:` magic followed by a 64-byte signature — **before** the
compression header. Decoders that check the LZMA properties byte at offset 0
see `S` (0x53) and fail or produce garbage.

Strip it before detecting compression. Detection is by magic bytes, so files
served without the header still work.

## 4. Compression

Detected by magic, never by extension:

| Magic | Format | Status |
|---|---|---|
| `SCLZ` | LZHAM | Detected, framing reported, **not decoded** — no maintained pure-Rust decoder |
| `28 B5 2F FD` | ZSTD | Supported |
| *(otherwise)* | LZMA | Supported |
| `"Name"` | Already decoded | Passed through |

All 12 required tables are currently LZMA.

### The LZMA header quirk

Supercell writes a truncated LZMA1 `alone` header: 5 property bytes followed by
a **4-byte** uncompressed size, where the format expects 8. Insert four zero
bytes at offset 9 to repair it, then decode normally.

## 5. Carry-over parsing

The single most consequential detail. Layout:

```
row 0     column names
row 1     column types ("String" | "int" | "boolean")
row 2..   data rows, grouped into per-entity blocks
```

A non-empty first cell starts a new entity block. Within a block each row is
one level, and **a blank cell inherits the last non-blank value seen for that
column in the same block**. Carry state resets at every block boundary.

```
"Cannon","Defense","3","300","900"     <- entity columns written once
"","","","360",""                      <- Width is still 3, range still 900
"","","","420",""
"Mortar","Defense","3","400","1100"    <- reset: range is 1100, not 900
```

Two failure modes, both silent:

- **Not carrying forward** nulls every stat above level 1.
- **Carrying across blocks** gives Mortar the Cannon's range.

Both are covered in `src/csv.rs` unit tests and again in `tests/carryover.rs`
against the real shipped files.

### An empty line is not a blank row

A row of full width whose cells are all empty is a *legitimate level* that
inherits everything from the level above. An empty line is a separator.
Conflating them drops a level and shifts every level number after it. Current
tables contain no such rows, but the distinction is enforced anyway.

### `townhall_levels.csv` carries forward across entities

This table needs a second rule. Each town hall level is its own single-row
entity, so block carry-over resets at every level and does nothing. Here a
blank count means *unchanged from the previous town hall level*:

```
TH1  Cannon = 1
TH2  Cannon = 2
TH3  Cannon =        <- still 2, not 0
TH5  Cannon = 3
```

Read as zero, buildings appear to vanish and reappear as the town hall rises.

## 6. Units

| Quantity | Unit | Anchor |
|---|---|---|
| Distance | 100 units = 1 tile | Cannon range 900 = 9 tiles |
| Time | milliseconds | Cannon `AttackSpeed` 800 = 0.8 s |
| Damage | `DPS` = damage/second | per-hit = `DPS * AttackSpeed / 1000` |

The tile scale is **not stated in the data**; it is derived from four defences
whose in-game ranges are observable, and re-verified on every load.

Defence damage is in the `DPS` column. The `Damage` column is blank for every
defence checked.

## 7. Reproducibility

Raw compressed assets are cached under `data/raw/<fingerprint>/logic/` and
committed. A given fingerprint therefore always decodes to byte-identical
input, and runs reproduce offline. `extraction.json` records the fingerprint,
version, extraction date, source, and the live version seen at extraction time.
