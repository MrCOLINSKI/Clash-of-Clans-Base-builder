# config/

Constants live here, never in Rust source.

The rule exists because a number baked into code cannot be re-fitted by the
calibration harness, cannot be varied in an experiment, and cannot be audited
against the game data it is supposed to mirror.

| File | Covers |
|---|---|
| `pathing.toml` | Target selection, wall costs, nav grid resolution, path lifecycle |
| `grid.toml` | Map extent and placement legality |

## `from_data`

A value of `"from_data"` means the default is read from the extracted game
data (`globals.csv`) rather than set here. The extracted value appears in the
trailing comment for reference, and is reported in the provenance banner on
every run.

Overriding one of these replaces a shipped game constant with a guess. Do it
only for a deliberate experiment, and record why in `ASSUMPTIONS.md`.

## Confidence

Not every constant here is equally trustworthy. `ASSUMPTIONS.md` tags each one
`DATA`, `DERIVED`, `ASSUMED`, `UNVERIFIED`, or `CONFLICT`. The weakest are the
map extent (§3.1) and the wall break cost (§2.2); both are flagged inline.
