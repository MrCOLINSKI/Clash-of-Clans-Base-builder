# Art container formats

Reverse-engineering notes for the three art containers, and what remains
unknown in each. Findings here were established by walking real shipped files
from game version 18.400.11, not from documentation — none is published.

## What ships

The asset manifest lists 9,075 files:

| Extension | Count | Contents |
|---|---|---|
| `.glb` | 3,079 | 3D models and animation clips (`sc3d/`) |
| `.ogg` | 3,044 | Sound effects |
| `.sctx` | 1,833 | Texture atlases |
| `.sc` | 728 | Legacy 2D sprite/animation containers |
| `.csv` | 139 | Logic tables (handled by `coc-data`) |
| `.png` | 60 | UI and effect bitmaps only |

Clash is a 3D game now. Building and troop art lives in `sc3d/` as glTF, not
as sprite sheets, so a renderer aiming at 1:1 fidelity is doing 3D model
import, not blitting.

## `.glb` — glTF binary with a Supercell descriptor

The container is **standard glTF 2.0**: `glTF` magic, version 2, correct
length field, spec chunk framing. The `BIN` chunk is ordinary glTF binary
payload holding vertices and indices.

The descriptor chunk is **not** standard. The specification requires a chunk
typed `JSON`; these carry one typed **`FLA2`** holding FlatBuffers.

```
alchemist_default_geo.ingame.glb   144,644 bytes
  chunk FLA2   21,688 bytes   <- FlatBuffers, not JSON
  chunk BIN   122,928 bytes   <- standard glTF payload
```

This single substitution is why importing this art is a reverse-engineering
job and not a library call: every off-the-shelf glTF loader rejects the file
at the descriptor chunk.

**Confirmed.** The `FLA2` chunk parses as a valid FlatBuffers root table.
Strings recovered from it include `SC_odin_format`, `bounds`, `parent`, and
skeleton joint names (`L_index_03_s`, `L_middle_03_s`, `L_pinky_03_s`,
`L_ring_03_s`) — so it plays the same role glTF's JSON would: scene graph,
node hierarchy, skinning.

**Open.** The FlatBuffers schema. Field *names* are compiled into the client
and are not in the buffer, so fields must be identified by index across many
files. Until the accessor and buffer-view fields are mapped, the `BIN` chunk
cannot be interpreted, even though its bytes are already in hand.

## `.sctx` — texture container

```
0   u32     header field A     (48 in every sample)
4   u32     header field B     (28 in every sample)
8   "SCTX"  magic              <- note: offset 8, not 0
12  ...     FlatBuffers metadata
..  ...     ZSTD frame: compressed texture payload
```

**Confirmed.**

- Magic is at offset 8. A decoder checking offset 0 finds nothing.
- The payload is a ZSTD frame. It began at offset 100 in every sample, but the
  decoder locates it by scanning for the ZSTD magic, since that offset is a
  consequence of the metadata size rather than a fixed layout.
- A `u32` in the header holds the decompressed length. Verified by matching it
  against the actual decompressed size:

  | File | declared | actual | match |
  |---|---|---|---|
  | `chr_cannon_cart_0.sctx` | 618,240 | 618,240 | yes |
  | `chr_cannon_mortar_cart_0.sctx` | 3,982,080 | 3,982,080 | yes |

- The payload is **ASTC** texture data: a flat array of 16-byte blocks whose
  first block is `fc fd ff ff ff ff ff ff 00 …`. The leading `0xFC` is the
  ASTC void-extent (constant colour) signature, which is what a fully
  transparent atlas margin encodes to.

**Open: image dimensions and ASTC block footprint.**

This is the trap worth recording. Reading a `u16` pair at offset 40 gives
plausible-looking dimensions, and for one file they even check out:

- `chr_cannon_mortar_cart_0.sctx`: reads 2928 × 1360, and 2928 × 1360 =
  3,982,080, exactly the payload size. Convincing.
- `chr_cannon_cart_0.sctx`: reads 1008 at the same offset, but 618,240 / 1008
  is not an integer. The payload instead factors cleanly as 672 × 920.

The offset is not stable, and it never could be: the metadata is FlatBuffers,
where a field the encoder omits is absent from the vtable and everything after
it shifts. Dimensions have to be read through the vtable, and the field index
is not yet pinned down across enough files to trust.

`Sctx::dimension_candidates` therefore returns every dimension pair consistent
with the payload size rather than picking one, and a test asserts the result
stays ambiguous — so that if it ever collapses to a single candidate, someone
has learned something and the test says so.

## `.sc` — legacy sprite container

Header is `"SC"` followed by a little-endian `u32` version. Shipped art in
18.400.11 is **version 6**.

Files pair as `name.sc` (shapes, movie clips, animation) with
`name_<n>.sctx` (the atlas they sample) — e.g. `chr_cannon_cart.sc` alongside
`chr_cannon_cart_0.sctx`.

**Open.** The record stream after the header. Only identification and version
reporting are implemented, so an extraction run can report precisely what it
could not handle rather than failing opaquely.

## Order of work

1. Map the SCTX metadata vtable → real dimensions and block footprint. This
   unblocks decoding textures to PNG and is the smallest remaining step.
2. Map the `FLA2` accessor/buffer-view fields → geometry out of the `BIN`
   chunk, which is already extracted and intact.
3. Decode the `.sc` record stream, needed only for 2D UI and effects.

Textures before models: a correct texture atlas plus building footprints
already produces a recognisable base, whereas geometry without materials does
not.
