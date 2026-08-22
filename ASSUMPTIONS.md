# ASSUMPTIONS

Every approximation, unverified constant, and judgement call in this project.
Anything not read directly from the extracted game data belongs here.

Status legend:

| Tag | Meaning |
|---|---|
| `DATA` | Read from the shipped logic tables. Not an assumption; recorded because the mapping needed a decision. |
| `DERIVED` | Computed from shipped data, with the derivation stated. |
| `ASSUMED` | Not in the data. A choice was made. Confidence noted. |
| `UNVERIFIED` | A value we cannot check against anything. Flagged loudly. |
| `CONFLICT` | Project spec and game data disagree. Resolution stated. |

Data used throughout: game version **18.400.11**, fingerprint
`7f04bdfdc4124b1f49308423bb8f4aa8b137aae3`, extracted 2026-08-21.

---

## 1. Data extraction

### 1.1 `DATA` — File names differ from the project brief

The brief named several files that do not exist under those names on the asset
CDN. The real names were taken from the shipped `fingerprint.json` manifest
(9075 entries):

| Brief | Actual | Note |
|---|---|---|
| `client_globals.csv` | `logic/globals.csv` | No file named `client_globals.csv` exists. |
| `supers.csv` | `logic/super_licences.csv` | |
| `equipment.csv` | `logic/character_items.csv` | |
| `characters_alt.csv` | *(absent)* | No such file in the manifest. |
| `assets/csv_logic/` | `logic/` | The `assets/csv_logic/` prefix is the path *inside the APK*; the CDN serves them under `logic/`. |

### 1.2 `DATA` — Signature header ahead of the compression header

Every `logic/*.csv` in this version is prefixed with a 68-byte `Sig:` header
(4-byte magic + 64-byte signature) *before* the LZMA header. The brief
described the LZMA quirk but not this wrapper. Decoding without stripping it
fails on every file. Detection is by magic bytes, so files served without it
still decode.

The brief's LZMA fix is confirmed exactly as described: insert four zero bytes
at offset 9 to repair the truncated 8-byte uncompressed-size field.

### 1.3 `ASSUMED` — LZHAM is detected but not decoded

Confidence: high that it does not matter today; the risk is deferred, not
eliminated.

All 12 required tables in 18.400.11 are LZMA. ZSTD is implemented. LZHAM
(`SCLZ`) is **detected and reported with its framing fields but not decoded** —
no maintained pure-Rust LZHAM decoder exists, and adding a C dependency for a
format the current asset set does not use was not worth the build fragility.

If Supercell switches to LZHAM this fails loudly with the dictionary size and
uncompressed length in the error, rather than silently producing garbage. That
is the point at which a decoder must be added.

### 1.4 `ASSUMED` — Fingerprint via APK range reads

Confidence: high. Verified working end to end.

The asset sha is not discoverable from the CDN; it ships in the client at
`assets/fingerprint.json`. Rather than download the ~795 MB APK, the ZIP
central directory is read over HTTP range requests and only that one entry is
fetched. This depends on the APK host honouring range requests (it does) and on
the entry being `STORED` rather than `DEFLATE` (it is, at 976 KB). A DEFLATE
entry currently fails with instructions rather than being handled.

The APK is sourced from a third-party mirror (APKPure). The version it serves
(`18.400.22`) is the *app* version; the asset bundle version inside is
`18.400.11`. These are expected to differ and the asset version is the one that
matters.

### 1.5 `DATA` — `townhall_levels.csv` has a second inheritance rule

This table carries a table-specific rule on top of the usual block carry-over,
and getting it wrong is silent.

Each town hall level is its own single-row entity, so ordinary block carry-over
resets at every level and does nothing. A blank count means **unchanged from
the previous town hall level**, not zero:

```
TH1  Cannon = 1
TH2  Cannon = 2
TH3  Cannon =        <- still 2
TH5  Cannon = 3
```

Read as zero, buildings appear to vanish and reappear as the town hall rises.
This was caught by the monotonicity check (148 spurious "count drops"), not by
inspection. Counts are now carried forward across levels in TH order, and
`tests/carryover.rs` pins the Cannon series `[1,2,2,2,3]`.

### 1.6 `DATA` — Traps do not share the buildings schema

`traps.csv` is a separate 90-column schema with **no `Hitpoints` column** —
traps are consumed on trigger, not destroyed by damage. They are modelled as a
distinct `Trap` type rather than forced into `Building`. The brief implied a
shared schema.

### 1.7 `DATA` — `VillageType` separates home village from Builder Base

`VillageType` is blank/0 for the home village and 1 for the Builder Base (73
and 32 buildings respectively). The Builder Base is a separate mode with its
own hall progression, so its entries are excluded from home-village town hall
checks. This project simulates the home village only.

**This filter is easy to forget and fails loudly only if you count.**
`townhall_levels.csv` carries counts for *both* villages in one table, so a
layout builder reading it unfiltered places both. At TH17 that meant 255 extra
Builder Base structures — including 180 BB Walls, a Battle Machine Altar and a
Clock Tower — giving 359 structures where the home village allows 156. The base
still validated, because every one of those structures genuinely has a non-zero
count in the table.

`GameData::is_home_village` and `GameData::home_counts` exist so callers filter
in one place, and `coc-core` has a regression test asserting no Builder Base
structure ever appears in a home village layout and that TH17 lands in the
100-200 range.

### 1.8 `DATA` — `heroes.csv` is not a superset of `characters.csv`

The two tables share a parser but not a schema: `heroes.csv` lacks
`IsUnderground` and several other flags. Columns that legitimately differ are
read through explicit `*_if_present` accessors; everywhere else a missing
column stays a hard error, because there it means schema drift. The distinction
is deliberate — a blanket lenient default would silently null real fields.

Note also the shipped spelling `PreferedTarget*` (one `r`) in `characters.csv`
versus `PreferredTarget*` in `buildings.csv`. Both spellings are accepted so an
upstream correction does not silently null the field.

### 1.9 `DERIVED` — 100 game distance units = 1 tile

**The tile scale is stated nowhere in the data.** It is recovered from defences
whose tile ranges are directly observable in game:

| Building | `AttackRange` | Known in-game range |
|---|---|---|
| Cannon | 900 | 9 tiles |
| Archer Tower | 1000 | 10 tiles |
| Mortar | 1100 (min 400) | 11 tiles (min 4) |
| X-Bow | 1400 | 14 tiles |

Four independent anchors agree, and Mortar's minimum range agrees on the same
scale, so confidence is high. `validate::check_unit_scale` re-checks these on
every load: if Supercell ever rescales, every distance in the simulator changes
meaning at once, so this must fail loudly rather than drift.

`AttackSpeed` is milliseconds (Cannon 800 = 0.8 s, matching the game).
`DPS` is damage per second; damage per hit is `DPS * AttackSpeed / 1000`.
Damage is **not** in the `Damage` column for defences — that column is blank
for every defence checked; `DPS` is authoritative.

### 1.10 `ASSUMED` — Freshness is judged by recorded live check

Confidence: medium. The mechanism is sound; the guarantee is only as good as
the last check.

The gate cannot know the live version offline. `extraction.json` records the
live version seen when the fingerprint was resolved. Freshness is `CURRENT`
when it matches, `STALE` when behind, and `UNVERIFIED` when no check was ever
recorded. `UNVERIFIED` **fails** the gate by default; `--allow-stale`
downgrades both to warnings for deliberate offline work. There is no
time-based expiry yet — a check recorded a year ago still reads `CURRENT`.
That is a known weakness; a max-age threshold should be added.

---

## 2. Pathfinding and targeting

### 2.1 `DATA` — Rule of N is shipped, not assumed

The brief specified `rule_of_n = 3` from community history. The game data
**confirms it directly**: `globals.csv` has `TARGET_LIST_SIZE = 3`, alongside
`MAX_TARGET_LIST_SIZE = 6`.

This is no longer an assumption. It is read from the data and reported in the
provenance banner on every run. The config knob remains, defaulting to the
extracted value.

Historical note from the brief, retained: Supercell shipped N=5 in November
2017 and reverted after player backlash. `MAX_TARGET_LIST_SIZE = 6` suggests
the client can still grow the list under some condition not yet identified —
**open question**, see §4.1.

### 2.2 `CONFLICT` — Wall cost: 15.5 tiles (brief) vs 10.0 tiles (data)

**Resolution: the extracted value is the default; the brief's value ships as a
named preset; the calibration harness re-fits both.** Directed by the project
owner ("do the most possible") when the conflict was raised.

The brief mandated `wall_break_cost_tiles = 15.5`, sourced from single-source
2017-era community forum testing (a Barbarian preferred a ~15 tile detour over
breaking one wall, but broke through at ~16+).

The game ships `WALL_COST_BASE = 1000` in `globals.csv`. At the verified scale
of 100 units/tile that is **10.0 tiles** — a real conflict, not a restatement.

Rule 1 of the brief says data wins over remembered or community numbers, so the
extracted value is the default. The 15.5 figure is preserved as the
`community_2017` preset because it is an *empirical behavioural observation*,
which is a different kind of evidence from a config constant: it is possible
that `WALL_COST_BASE` is scaled or combined with other terms before reaching
the pathfinder, in which case 10.0 would be wrong as an effective tile cost.
Calibration against real replays is what will settle it.

Neither value is confirmed against observed behaviour yet. **Until calibration
fixtures exist, no wall-cost-sensitive result should be trusted.**

### 2.3 `DATA` — Wall cost is per-unit, and the brief's open question is answered

The brief asked whether wall cost scales with wall level/HP, and said to
default to flat.

The data answers it: `characters.csv` has a **`WallMovementCost`** column that
overrides the global per unit. It is populated for exactly 7 of 193 units:

| Value | Units |
|---|---|
| 128 | Wall Breaker, Super Wall Breaker |
| 16 | Wall Wrecker, Log Launcher, Elephant, Elephant Rider, *UnusedSiegePrototype* |
| *(blank)* | the other 186 — these use `WALL_COST_BASE` |

The cost is keyed on **who is crossing**, not on the wall's level or hitpoints.
Nothing in the data ties the path cost to wall HP, so **flat with respect to
wall level is confirmed**, and `wall_cost_scales_with_hp` defaults to `false`
as the brief specified — now on evidence rather than as a fallback.

Lower values mean cheaper crossing, consistent with siege machines (16)
smashing through and Wall Breakers (128) being built for it.

### 2.4 `DATA` — Wall jumping is a flag, not a unit list

The brief asked for wall-jumpers to be resolved from a data flag rather than a
hardcoded list. The flag is **`IsJumper`**, true for **Hog Rider** and **Root
Rider** in this version. `MovementClass` is resolved from `IsFlying`,
`IsJumper`, and `IsUnderground` in that precedence order, with no name
matching anywhere.

Jumpers are given wall cost 0. Note this is an interpretation: the data marks
them as jumpers but does not state a cost, and they have no `WallMovementCost`
override. Zero matches observed behaviour.

### 2.5 `DATA` — Further shipped pathing globals

Extracted and surfaced rather than assumed:

| Global | Value | Bearing |
|---|---|---|
| `RETARGET_AFTER_DESTROYING_WALL` | `FALSE` | A unit that breaks a wall does **not** retarget. Constrains §2.4 of the brief's path lifecycle. |
| `USE_WALL_WEIGHTS_FOR_JUMP_SPELL` | `TRUE` | Jump Spell interacts with wall weights rather than plain zeroing. |
| `UNDERGROUND_UNIT_GROUND_SPEED_PERCENTAGE` | `70` | Miner-class speed while burrowed. |
| `USE_HEAT_MAP_IN_ATTACK_POSITION_SELECTION` | `TRUE` | The client uses a heat map for ranged attack-position selection — the brief's §2.5 model is an approximation of something more elaborate. **Open question**, see §4.2. |
| `WALL_BREAKER_SMART_RADIUS` | `2500` | Wall Breaker special targeting, 25 tiles. |
| `WALL_BREAKER_SMART_CNT_LIMIT` | `30` | |
| `WALL_BREAKER_SMART_RETARGET_LIMIT` | `2000` | |
| `WALL_BREAKER_USE_ROOMS` | `FALSE` | Room/compartment analysis is off in this version. |

### 2.6 `UNVERIFIED` — Defence retarget delay

`defense_retarget_delay_ms` defaults to **0**, as the brief instructed. Nothing
in `globals.csv` or `buildings.csv` was found that encodes it.

`characters.csv` has related but distinct columns — `NewTargetAttackDelay`,
`TargetKilledCooldownTimer`, `RetargetAfterHit` — which are *troop* retarget
timings, not defence ones. Whether an equivalent exists for defences under
another name has not been established. Flagged as unverified per the brief.

### 2.7 `ASSUMED` — Half-tile navigation grid

Confidence: medium, taken from the brief.

`subtiles_per_tile = 2` per the brief's §2.3, on the basis of an approximate
0.5-tile gap around building hitboxes. Nothing in the extracted data states a
hitbox inset — `buildings.csv` gives `Width`/`Height` in whole tiles only, and
`AreEdgesUnpassableByVillagers` exists but is a villager-pathing flag, not a
hitbox measurement. The 0.5-tile gap remains an unverified community
observation. Configurable; whether 4 subtiles is needed for Valkyrie-style
channel exploitation is untested.

### 2.8 `DATA` — Trap displacement is three distinct mechanics

The brief treated Spring Trap and Tornado Trap as displacement effects forcing
a retarget. The data disagrees and separates three behaviours:

| Trap | Columns | Actual effect |
|---|---|---|
| Spring Trap | `EjectVictims=FALSE`, `EjectWhenKilling=TRUE`, `EjectRadius=200`, `EjectHousingLimit=10` | **Removes** units from the battle. They do not retarget, because they are gone. |
| Bomb, Giant Bomb | `Pushback=100`, `PushbackHousingLimit=3` / `30` | **Displaces** survivors — this is the case that forces a path recompute. |
| Tornado Trap | `DurationMS=5000`, `SpeedMod=100`, no eject/pushback | A timed area effect, not a throw. |

So the brief's "Spring Trap causes displacement → forced retarget" is wrong on
this version's data: it causes *removal*. Modelled as `removes_victims()` and
`causes_displacement()` separately, with `immune_by_housing()` for the housing
thresholds that decide who is too heavy to be affected.

### 2.9 `ASSUMED` — Wall cost is charged once per wall *tile*

Confidence: high that this is the only defensible choice; no data source.

Nothing shipped states how the wall penalty interacts with a navigation grid
finer than one tile, because the grid resolution is a property of this
implementation, not of the game. Charging on every wall *cell* entered would
make one wall cost `subtiles_per_tile` times too much, and — worse — would make
the game's balance depend on a configuration knob: doubling the nav resolution
would double the effective cost of every wall in the game.

So the cost is charged when a step enters a wall cell belonging to a different
tile than the cell it came from. One wall tile is charged once; a double layer
is charged twice, which is what a double layer should cost. `path.rs` has a
regression test (`wall_cost_does_not_depend_on_grid_resolution`) that runs the
same crossing at two resolutions and requires the paid difference to be equal.

### 2.10 `ASSUMED` — Diagonal steps may not cut corners

Confidence: medium. No shipped constant describes it and it is not observable
from the data.

A diagonal step between two nav cells is permitted only when both of its
orthogonal neighbours are passable. Without the rule, a unit slips through the
zero-width gap where two buildings meet corner to corner, which would make
"corner-to-corner" a free channel through any base and would reward a layout
style that does not work in the real game.

The alternative — allowing corner cuts — is a one-line change in
`path::cost_field` and is a candidate for the phase 5 calibration harness to
test against real replays.

### 2.11 `ASSUMED` — `MAX_TARGET_LIST_SIZE` is used for unreachable candidates

Confidence: low on the trigger, high that *some* widening is needed.

§4.1 records that the condition under which the client grows the candidate list
from 3 to 6 is unidentified. This project grows it on one condition only: when
every candidate in the current list turns out to be unreachable at any cost.
Without some such rule a unit sealed away from its three nearest targets simply
stops, which is certainly not what the client does.

This is a *floor*, not a claim to have found the trigger. There may be others,
and if there are, this simulator does not reproduce them.

### 2.12 `ASSUMED` — Burrowing units path in a straight line

Confidence: medium.

`IsUnderground` units (the Miner and its variants) tunnel, and tunnelling
ignores walls. Whether it also ignores *buildings* is not stated anywhere in the
data. Modelled as ignoring both, i.e. identically to a flier except for the
extracted `UNDERGROUND_UNIT_GROUND_SPEED_PERCENTAGE = 70` speed penalty, on the
grounds that a Miner visibly surfaces inside compartments no ground unit could
walk into.

### 2.13 `ASSUMED` — A stated target preference is a preference, not a filter

Confidence: high; behavioural, not from data.

A Giant whose preferred class has been wiped out keeps attacking rather than
standing still. Implemented as: filter to the preferred class; if that leaves
nothing, fall back to every structure. Nothing in `characters.csv` states this —
the column simply names a class — but the alternative reading (a hard filter)
would leave Giants idle at the end of every successful attack, which is not what
happens.

---

## 3. Grid and layout

### 3.1 `ASSUMED` — Map dimensions are not in the shipped data

Confidence: low on the exact numbers. **This is the weakest assumption in the
project so far.**

The brief's §1.4 said to read map dimensions from `client_globals.csv`. That
file does not exist, and `logic/globals.csv` contains **no** width, height,
tile, grid, map-size, border, or playfield key — all 500 keys were searched.
The dimension is compiled into the client, not shipped as data.

Directed by the project owner to "do all" of the offered options, so:

1. **Default config**: 44×44 total with a 40×40 buildable interior, the common
   community value, set in `config/grid.toml` rather than in code.
2. **A validator** rejects any layout exceeding the configured extent, so a
   wrong constant surfaces as a failure rather than as silently clipped bases.
3. **A base-link decoder** is planned so the extent can be derived empirically
   from a real in-game layout link. Until a sample link is supplied this cannot
   be confirmed, and the 44×40 figure remains unverified.

Every layout-extent-dependent result is provisional until (3) runs.

---

## 2b. Combat timing — three numbers the game does not ship

Found while starting phase 4, and worth its own section because these bound
every combat result the project will ever produce. All 500 keys in
`globals.csv` were searched for each.

### 2b.1 `UNVERIFIED` — What a Speed value is *per*

Confidence: low. **The most consequential unknown in the project.**

`characters.csv` gives `Speed` as a bare integer — Giant 150, Barbarian 220,
Archer 300, Goblin 400 — with no unit and no conversion constant anywhere in
the shipped data. There is no tick rate, no reference duration, and nothing
relating `Speed` to the distance units every range in the game is expressed in.

The reading adopted is the one that invents no number: `Speed` is game distance
units per second, the same units as `AttackRange` (100 = 1 tile, §1.9). A Giant
then crosses 1.5 tiles per second and a Goblin 4.0.

This may be wrong by a constant factor. If it is, *every unit is wrong by the
same factor* — which is precisely the error shape a calibration fit against one
observed replay corrects. It is therefore expressed in `config/combat.toml` as
a single global multiplier (`units_per_second_per_speed_point` over
`units_per_second_divisor`) rather than as a per-unit table, so the fit has one
parameter to find rather than 193.

Nothing here is guessed from memory of the game. The alternative would have
been to write down a remembered tiles-per-minute figure, which RULE 1 forbids
and which would have been unfalsifiable once written.

### 2b.2 `UNVERIFIED` — Battle length

The community figure is 30 seconds of scouting plus 3 minutes of battle.
Neither is in the data. Scouting is irrelevant to a simulator that deploys on
tick zero, so only the 180 seconds is modelled, and it lives in
`config/combat.toml`.

### 2b.3 `UNVERIFIED` — Star thresholds

50% destruction, the Town Hall, and 100% destruction. None of the three is in
the data.

One near-miss worth recording so nobody else mistakes it for a source:
`HIDDEN_BUILDING_APPEAR_DESTRUCTION_PERCENTAGE = 50` **is** shipped, but it
governs when a Hidden Tesla surfaces, not when a star is awarded. It shares the
number 50 with the star threshold coincidentally, and reading it as
confirmation would be exactly the kind of plausible-looking error this document
exists to prevent.

Also `ASSUMED`: destruction percentage does not count walls. A base with 300
walls would otherwise be near-impossible to three-star, which is not how the
game plays — but this is reasoning from behaviour, not from data.

---

## 4. Open questions

Unresolved. Listed so they are not quietly forgotten.

1. **`MAX_TARGET_LIST_SIZE = 6`** — under what condition does the client grow
   the target list beyond `TARGET_LIST_SIZE = 3`? If there is a trigger, the
   two-stage selection has a mode this project does not yet reproduce.
   *Partially addressed in §2.11*: the list is grown when every candidate is
   unreachable. That is a necessary condition, not the identified one.
2. **`USE_HEAT_MAP_IN_ATTACK_POSITION_SELECTION = TRUE`** — the real client
   picks ranged attack positions with a heat map. The brief's "nearest
   reachable subtile in range" is an approximation of unknown fidelity.
3. **Defence retarget delay** — see §2.6. No source found.
4. **Wall cost units** — whether `WALL_COST_BASE` is consumed by the
   pathfinder in raw distance units, or scaled first. Decides whether §2.2
   resolves to 10.0 tiles or something else.
5. **Hitbox inset** — the 0.5-tile gap in §2.7 has no data source.
6. **Map extent** — §3.1, pending a real base link.

---

## 6. Art containers

Added when 1:1 art extraction was brought into scope. Full notes in
`docs/ART_FORMATS.md`.

### 6.1 `DATA` — Buildings are 2D sprites; only characters are 3D

**This corrects an earlier entry** which claimed building art was 3D geometry.
It is not.

Of 9,075 shipped files, 3,079 are `.glb` and 1,833 are `.sctx`. But the `.glb`
models resolve to only **38 distinct stems**, and every one is a character:
`alchemist`, `archerqueen`, `barbking`, `grandwarden`, `royalchampion`,
`lassi`, `unicorn`, and so on — heroes, pets and a few troops.

Building art is 2D, in `sc/buildings.sc` plus its **71** `.sctx` atlases
(`buildings_0` … `buildings_70`), with further sheets in `buildings2`,
`building_bases` and `buildings_cc`. That is exactly what a top-down or
isometric renderer needs, so the art path is much shorter than first assessed.

### 6.2 `DATA` — `.glb` is glTF 2.0 with a non-standard descriptor chunk

The container is spec-compliant and the `BIN` chunk is ordinary glTF payload.
The descriptor chunk is typed **`FLA2`** and holds FlatBuffers where the
specification requires `JSON`. Verified: the chunk parses as a valid
FlatBuffers root table, and its strings include `SC_odin_format`, `bounds`,
`parent`, and skeleton joint names.

Consequence: every off-the-shelf glTF loader rejects these files. This is the
whole reason art import is a reverse-engineering project.

### 6.3 `DATA` — `.sctx` is ZSTD-compressed ASTC

Magic at offset 8 (not 0), FlatBuffers metadata, then a ZSTD frame. The
decompressed payload is a flat array of 16-byte blocks beginning
`fc fd ff ff …`; `0xFC` is the ASTC void-extent signature. The declared
decompressed length matched the actual size exactly on both samples tested
(618,240 and 3,982,080 bytes).

### 6.4 `DATA` — SCTX dimensions and block footprint, solved

**Supersedes the previous entry**, which recorded these as unresolved and
warned against reading them at a fixed offset. The warning was right; the
reason is now understood.

The header is not three opaque words. It is the standard **size-prefixed
FlatBuffers preamble**:

```
[0..4]   u32   size of the FlatBuffers region
[4..8]   u32   offset to the root table, relative to offset 4
[8..12]  char  file identifier, "SCTX"
```

The buffer base is **offset 4** — not 0, and not 12 as first assumed. That is
precisely why a fixed offset validated on one file and silently failed on the
next: the root table starts at a different place depending on how many fields
the encoder wrote.

Read through the vtable, the fields are stable across every file tested:

| Field | Meaning |
|---|---|
| 2 | width in pixels |
| 3 | height in pixels |
| 6 | format enum (5 and 12 both observed) |
| 7 | decompressed payload length |

### 6.5 `DERIVED` — The payload is ASTC 6x6

With real dimensions the footprint follows arithmetically. For every file
tested, `ceil(w/6) * ceil(h/6) * 16` equals the payload length **exactly**:

| File | Dimensions | Blocks | ceil(w/6) x ceil(h/6) |
|---|---|---|---|
| `chr_cannon_cart_0` | 1008 x 1376 | 38,640 | 168 x 230 |
| `chr_cannon_mortar_cart_0` | 2928 x 3058 | 248,880 | 488 x 510 |
| `buildings_0` | 608 x 1004 | 17,136 | 102 x 168 |
| `buildings_2` | 1632 x 2042 | 92,752 | 272 x 341 |

No other footprint accounts for the payload, so this is derived rather than
assumed. Confirmed by decoding `sc/buildings_0.sctx` to PNG and looking at it:
correct colours, clean alpha, recognisable buildings.

### 6.6 `DATA` — Two payload modes, detected rather than flagged

Both ship in the same version:

- **ZSTD-compressed** — character atlases (`chr_*`).
- **Stored raw** — the building atlases; ASTC blocks sit at the end of the file.

No field has been identified that declares which, so the decoder detects it: a
ZSTD frame after the metadata means compressed, its absence means raw. If a
future format adds a third mode this fails loudly on the length check rather
than producing garbage.

### 6.7 `DERIVED` — Portrait identity is matched by TID; images come from a third party

**The renderer now shows the right building at the right level.** How it gets
there needs stating plainly, because the identity and the pixels have
different provenance.

**Identity: data-driven, from the game's own tables.** Each structure is matched
to its portrait by **TID** — `TID_BUILDING_CANNON` and so on — not by name or
by eye. That matters because the display names diverge from the internal keys:

| Our name | Internal key | Slug |
|---|---|---|
| X-Bow | Bow | `bow` |
| Inferno Tower | Dark Tower | `dark-tower` |
| Clan Castle | Alliance Castle | `alliance-castle` |
| Army Camp | Troop Housing | `troop-housing` |
| Eagle Artillery | Ancient Artillery | `ancient-artillery` |
| Hidden Tesla | Tesla Tower | `tesla-tower` |
| Bomb | Mine | `mine` |
| Spring Trap | Ejector | `ejector` |
| Seeking Air Mine | MegaAirTrap | `megaairtrap` |

Matching by name would have failed on every one of these. 65 of 73
home-village buildings and 16 of the traps matched by TID; the misses are
TH17/18 additions the third party has not published yet.

The slug rule is: lowercase, split on non-alphanumerics, **do not split
camelCase**. So `Alliance Castle` becomes `alliance-castle` but `AirTrap`
becomes `airtrap`, not `air-trap`. Getting this wrong silently 404s a whole
category — it is what initially lost every trap portrait.

**Levels: data-driven.** The level shown is the max that town hall allows,
read per level row from `TownHallLevel`, so a TH4 Cannon is a level-5 Cannon
and is drawn as one. Coverage at the time of writing: 1255 structures at their
exact level, 395 at the nearest published level, 50 with no portrait.

**Pixels: `ASSUMED`, and not from the game files.** The portraits are fetched
from coc.guide rather than decoded from Supercell's own atlases, because the
`.sc` shape-to-rectangle hop (below) is still unsolved. Confidence that they
are the correct artwork is high — the site states it extracts from the game
files, and spot checks match — but this is **second-hand**, unlike every
balance figure in this project, and should be replaced once the SC decoder can
produce the rectangles directly.

Where a level's portrait is unpublished, the nearest published level is drawn.
The structure is still correct in identity, footprint, range and hitpoints,
since those come from the extracted data; only the picture is approximate, and
the renderer reports the counts. A structure with no portrait falls back to a
plain footprint block rather than borrowing another building's, so nothing on
screen is ever actively wrong about what it is.

### 6.7b `UNVERIFIED` — SC shape record to atlas rectangle

Still unsolved, and the reason 6.7 relies on a third party for pixels. See
`crates/coc-assets/src/sc.rs` for how far the container is mapped: the export
table, the object graph, the point pool, the transform banks and the texture
list all decode. What does not is the hop from a shape record to its rectangle
in the atlas.

Three approaches were tried and rejected: assigning by footprint (drew a
max-level X-Bow where a level-2 Mortar belonged), object ordering (not
alphabetical, not atlas order), and identifying sprites by eye across all 4,920
extracted sprites (the atlases carry every seasonal skin and every level, so
most cannot be told apart with confidence).

### 6.8 `DATA` — `.sc` is version 6, little-endian

An initial big-endian reading gave 100663296 instead of 6 and was caught by a
test against a real file. Only the header is parsed; the record stream is
unmapped.

### 6.9 Redistribution of extracted art

Committing decoded art to a public repository redistributes Supercell's
copyrighted assets, which is a materially different proposition from the
balance CSVs. This was raised and the project owner directed that assets be
committed. Recorded here as a decision made with the tradeoff stated, not as
an oversight.

---

## 7. Layout links

### 7.1 `DATA` — A base link cannot carry a layout, so it cannot be generated

The brief's Section 8 asked for the in-game base link encoding to be supported
"if you can decode it". It cannot be, and the reason is structural rather than
a matter of effort.

A layout link takes the form:

```
https://link.clashofclans.com/?action=OpenLayout&id=TH17%3AHV%3A<32 chars>
```

The base64url payload decodes to exactly **24 bytes**, of which bytes 4-8 carry
the base slot (1, 2 or 3). Twenty-four bytes cannot describe 150 buildings at
44x44 resolution — it is not a compression question, there is not enough
entropy in the string.

So the link is an **identifier for a layout already stored on Supercell's
servers**, tied to a player account and one of their slots, and community
tooling confirms the in-app deep-link handler is the only thing that can
resolve one.

Consequences, which are worth stating plainly because they bound the product:

- A layout designed offline **cannot be turned into a working link**. There is
  no client-side encoding to reverse.
- Sites that share base links are sharing bases somebody actually built in
  game, not generated ones.
- Export is therefore a **placement plan** — exact tile coordinates followed by
  hand in the in-game editor — plus the project's own JSON schema.

### 7.1b `DATA` — Published base catalogues carry no coordinates either

Checked directly, since "fetch real base designs" is an obvious thing to want.

- **`clash-bases`** — 5,162 curated layouts, TH4 to TH18. Every entry is
  `{name, town_hall, type, link, builder, image}`. The link is the same 24-byte
  identifier; the image is a screenshot on an image host. **No coordinates.**
- **`cocbases`, `clashofclanslayouts`, `cocbase.net`** — same shape: links and
  screenshots.
- **`coc-base-analyser`** — genuinely reads real layouts, but its
  `VillageJsonParser` consumes the game's **private village JSON**, obtained
  from an authenticated session. Not reachable, and not a public endpoint.
- **The official API** (`api.clashofclans.com`) exposes clan and player stats
  and **no layout endpoint at all**.

So a real base can be *referenced* and opened in-game, but its placements
cannot be imported. Reconstructing one from a screenshot is the only remaining
route, and that is image analysis with no ground truth to check against.

The catalogue is surfaced in the renderer as links, not as importable designs,
and says so.

### 7.2 Consequence for the map extent

This also closes the door on deriving the 44x44 grid extent from a link, which
§3.1 held open. A 24-byte identifier contains no coordinates. The extent stays
an unverified community figure unless another source is found.

---

## 5. Simulator status

**The simulator is UNCALIBRATED.** No fixture of a real observed attack has
been compared against simulated output. Every result is directional only, and
the warning is printed on every run and is not suppressible.

Calibration is Phase 5 and must complete before any optimizer output is
treated as meaningful.

Phase 3 (pathfinding and targeting) is complete and self-consistent, which is
not the same thing as being right. What it does guarantee:

- Every constant it uses comes from `globals.csv` or `config/pathing.toml`;
  `tests/pathing.rs::no_pathing_constant_is_written_in_rust_source` fails if the
  config drifts from the shipped data.
- The two-stage selection reproduces the rule-of-3 behaviour it is supposed to,
  including the case where a troop walks past a defence it is standing beside.
- Costs are integral end to end, and repeated runs on identical inputs produce
  identical paths and targets.

What it does not guarantee is that a real Giant would make the same choice. The
open items in §4, and §2.9 through §2.13, are exactly the knobs the calibration
harness will need to fit.

Measured, for regression reference: a 250-unit mass retarget on a full TH17
base takes ~190 ms in release (~0.75 ms per unit) on the development machine.
That is acceptable for a once-per-Jump-Spell event and would not be acceptable
per tick; if retargeting ever becomes continuous, the cost field will need to be
shared between units rather than recomputed per unit.
