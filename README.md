# clashsim

Clash of Clans base designer, attack simulator, and layout optimizer, in Rust.

> **SIMULATOR STATUS: UNCALIBRATED — results are directional only.**
> No simulated battle has yet been compared against a real observed attack.
> This warning is printed on every run and cannot be suppressed.

## Status

Built in strict phase order; a phase is not started until the previous one's
tests pass.

| Phase | Component | State |
|---|---|---|
| 1 | `coc-data` — fetch, decompress, carry-over parse, validation gate | **complete, 36 tests green** |
| 2 | `coc-core` — grid, layout, placement legality | not started |
| 3 | `coc-sim` — pathing per spec | not started |
| 4 | `coc-sim` — combat, traps, spells, heroes | not started |
| 5 | Calibration harness | not started |
| 6 | `coc-meta` — attack strategies | not started |
| 7 | `coc-opt` — GA / annealing | not started |
| 8 | `coc-render` + CLI | not started |

## Data

All balance figures come from the shipped game data. No hitpoint, damage,
footprint, range, cooldown, or per-town-hall count is hardcoded anywhere.

Currently pinned to game version **18.400.11**, fingerprint
`7f04bdfdc4124b1f49308423bb8f4aa8b137aae3` (TH18). Raw compressed assets are
committed under `data/raw/<fingerprint>/`, so builds are reproducible offline.

```
cargo run -p coc-data --bin validate
```

Prints the provenance banner and runs the gate. It fails the build on stale
data, unresolved nulls, a changed distance scale, or inconsistent per-town-hall
counts.

See `docs/EXTRACTION.md` for how the pipeline works and the details that break
naive implementations.

## Assumptions

`ASSUMPTIONS.md` records every approximation and unverified constant, tagged by
confidence. It is not optional reading: the two weakest constants — the map
extent and the wall break cost — materially affect optimizer output.

Notably, two things the brief treated as assumptions turned out to be **shipped
in the game data**: the rule-of-N candidate count (`TARGET_LIST_SIZE = 3`) and
the wall traversal cost (`WALL_COST_BASE = 1000`). The latter conflicts with
the specified value; see `ASSUMPTIONS.md` §2.2.

## Layout links

In-game base layout links are **not yet supported**. Layouts use a documented
JSON schema. A link decoder is planned, partly because it is the only way to
empirically derive the map extent.

## Build

```
cargo test          # all phases' tests
cargo build --release
```

Rust 1.85+. `unsafe` is denied workspace-wide.
