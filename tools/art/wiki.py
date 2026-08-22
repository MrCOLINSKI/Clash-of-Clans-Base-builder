"""Look up building art on the Clash of Clans wiki by file-name prefix.

The art host used for everything else predates TH17/18, so the newest
structures have no portrait there at any level. The wiki does carry them, one
file per level, named `<Building><level>.png`.
"""
import json, urllib.request, urllib.parse, sys

API = "https://clashofclans.fandom.com/api.php"
UA = {"User-Agent": "Mozilla/5.0 (X11; Linux x86_64)"}

def allimages(prefix, limit=60):
    q = urllib.parse.urlencode({
        "action": "query", "list": "allimages", "aiprefix": prefix,
        "ailimit": limit, "format": "json"})
    req = urllib.request.Request(f"{API}?{q}", headers=UA)
    with urllib.request.urlopen(req, timeout=25) as r:
        d = json.load(r)
    return [(i["name"], i["url"]) for i in d.get("query", {}).get("allimages", [])]

if __name__ == "__main__":
    for pre in sys.argv[1:]:
        got = [(n, u) for n, u in allimages(pre) if n.lower().endswith(".png")]
        print(f"== {pre}: {len(got)} png")
        for n, u in got[:24]:
            print("   ", n)
