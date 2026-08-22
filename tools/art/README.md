# Art pipeline

Resolves every home-village structure to its per-level artwork and packs the
renderer payload.

```
python3 resolve.py       # building name -> wiki file prefix + published levels
python3 geturls.py       # file names -> CDN urls (50 per API call)
./dl.sh via xargs        # download
python3 repack_wiki.py   # trim, scale, base64, emit payload
```

## Why the wiki and not an icon host

The icon host used first (`coc.guide`) stops at the TH16 roster. Five
structures — Multi-Gear Tower, Revenge Tower, Super Wizard Tower, Crafting
Station, Skeleton Trap — had no portrait at any level. It also publishes a
picture only at levels where a building's appearance changes, which is correct
in itself but meant most structures rendered a tier below their own.

The wiki carries one file per level for everything: Cannon 1-21, Wall 1-19,
Town Hall 1-18.

It also settles the walls. Shop art draws a wall as an L-shaped corner about two
tiles wide, and at level 18 as a free-standing tower; stamping that per tile
piles corners on top of each other, which is why walls were procedural blocks
for several iterations. The wiki publishes each wall level as a single block one
tile wide, so the real artwork tiles along a run.

## Naming

Most structures resolve as `Name.replace(' ', '_')`. The exceptions are real and
worth keeping written down:

| In-game name | Wiki prefix |
|---|---|
| Multi Gear Tower | `Multi-Gear_Tower` |
| Multi Archer Tower | `Multi-Archer_Tower` |
| ShrinkTrap | `Shrink_Trap` |
| Siege Workshop | `Workshop` |
| BOBs Hut | `B.O.B` |

Some defences ship art per firing mode or state rather than plain per level —
Inferno Tower is `_Single`/`_Multi`, X-Bow `_Ground`/`_Air`, Spell Tower
`_Rage`/`_Poison`/`_Invisibility`, Revenge Tower `_Dormant`/`_Stage1..3`. The
resolver takes the resting default so a base reads the way it sits idle.
