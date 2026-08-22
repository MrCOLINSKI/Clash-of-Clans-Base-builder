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
| 2 | `coc-core` — grid, layout, placement legality, metrics | **complete, 14 tests green** |
| 3 | `coc-sim` — pathing per spec | not started |
| 4 | `coc-sim` — combat, traps, spells, heroes | not started |
| 5 | Calibration harness | not started |
| 6 | `coc-meta` — attack strategies | not started |
| 7 | `coc-opt` — simulated annealing over legal layouts | **first pass, 4 tests green** |
| 8 | `coc-render` + CLI | not started |
| — | `coc-assets` — art container decoders (parallel track) | containers parsed, 26 tests green |

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

## Layout optimization

`coc-opt` anneals layouts against geometric metrics from `coc-core` — coverage,
town hall depth, wall enclosure, quadrant balance, perimeter exposure —
profile-weighted for war, farming or trophy.

```
cargo run --release -p coc-opt --example optimize -- layouts.json war
```

All 18 town halls tune in about 3.5 seconds. Every candidate passes the
legality validator before scoring, so the search cannot emit an illegal base;
the example asserts it on every emitted layout.

**This is a geometric proxy, not simulated destruction.** The project's rule
that real fitness needs a calibrated simulator still holds — what this replaces
is hand-placement, giving a repeatable ruler rather than a guess. Battle-based
fitness slots in behind the `Objective` trait without touching the search.

## Art extraction

Running in parallel with the simulator. `coc-assets` decodes the three shipped
art containers; see `docs/ART_FORMATS.md`.

Clash is a 3D game: of 9,075 shipped files, 3,079 are `.glb` models and 1,833
are `.sctx` texture atlases, against only 60 plain `.png`. Matching the game
1:1 means importing models, not blitting sprites.

Container parsing is done and verified against real files. The remaining work
is schema mapping: the `.glb` descriptor chunk is FlatBuffers typed `FLA2`
rather than the spec's `JSON`, and `.sctx` image dimensions must be read
through the FlatBuffers vtable rather than a fixed offset. Neither is guessed
— see `ASSUMPTIONS.md` §6.

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
