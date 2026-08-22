"""Resolve each in-game building name to its wiki file prefix and level set."""
import json, re, sys, time
from wiki import allimages

ALIAS = {
    "Multi Gear Tower":   ["Multi-Gear_Tower"],
    "Multi Archer Tower": ["Multi-Archer_Tower"],
    "ShrinkTrap":         ["Shrink_Trap"],
    "FreezeBomb":         ["Freeze_Trap", "FreezeBomb", "Freeze_Bomb"],
    "Siege Workshop":     ["Workshop", "Siege_Machine_Workshop"],
    "BOBs Hut":         ["B.O.B_Hut", "BOB_Hut", "Bob_Hut"],
    "Builders Hut":     ["Builder%27s_Hut", "Builder's_Hut", "Builders_Hut"],
    "Helper Hut":       ["Helper_Hut", "Pet_House"],
    "X-Bow":            ["X-Bow"],
    "Town Hall":        ["Town_Hall"],
}

def candidates(name):
    base = name.replace(" ", "_")
    return ALIAS.get(name, []) + [base, base.replace("-", "_"), base.replace("_", "-")]

def resolve(name):
    for pre in dict.fromkeys(candidates(name)):
        try:
            files = [n for n, _ in allimages(pre, 200) if n.lower().endswith(".png")]
        except Exception:
            time.sleep(1.0); continue
        lv = {}
        for n in files:
            m = re.fullmatch(re.escape(pre) + r"(\d+)\.png", n)
            if m: lv[int(m.group(1))] = n
        if lv:
            return pre, lv, None
        # No plain per-level files: some buildings only ship state variants
        # (Revenge Tower is Dormant/Stage1..3), so take the calmest one.
        # Some defences only ship art per firing mode or state. Pick the
        # resting/default one so the base reads the way it sits idle.
        for suf in ("_Single", "_Ground", "_Rage", "_Dormant", "_FastAttack",
                    "_unarmed", "_info", ""):
            lv = {}
            for n in files:
                m = re.fullmatch(re.escape(pre) + r"(\d+)" + re.escape(suf) + r"\.png", n)
                if m: lv[int(m.group(1))] = n
            if lv: return pre, lv, suf
        if files:
            return pre, {}, files[:6]
    return None, {}, None

if __name__ == "__main__":
    need = json.load(open("needed_pairs.json"))
    out, problems = {}, {}
    for name in need:
        pre, lv, note = resolve(name)
        if lv:
            out[name] = {"prefix": pre, "levels": {str(k): v for k, v in sorted(lv.items())},
                         "suffix": note if isinstance(note, str) else ""}
        else:
            problems[name] = {"prefix": pre, "saw": note}
        print(f"{name:26} -> {pre} ({len(lv)} levels)" + (f"  NOTE {note}" if note and not isinstance(note,str) else ""))
    json.dump(out, open("wiki_map.json", "w"), indent=1)
    json.dump(problems, open("wiki_problems.json", "w"), indent=1)
    print(f"\nresolved {len(out)} / {len(need)}; unresolved {len(problems)}")
