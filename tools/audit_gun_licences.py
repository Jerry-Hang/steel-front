#!/usr/bin/env python3
"""Audit the licences of the third-party weapon models in assets/guns_ext/.

WHY THIS EXISTS
---------------
`docs/WEAPON-LICENCE-AUDIT.md` could not be completed by hand, for a reason that
looked fatal: the files kept their Sketchfab slugs, so they are *findable*, but a
slug does **not** identify the asset. Searching `pp-19-01_vityaz` returns models
by two different authors under different licences, and picking one on the
strength of the name produces a licence record that looks verified and is not.

WHAT SOLVES IT
--------------
Sketchfab's public API exposes `faceCount` per model, and a GLB states its own
triangle count. Matching the two turns an ambiguous name into a positive
identification:

    search  "Low-Poly OSV-96"  -> TastyTony, faces=6721, CC Attribution
    our file low-poly_osv-96.glb -> tris=6717        <- 4 tris apart: same asset

The residual delta comes from preprocessing (`tools/blender/prep_guns.py`
normalises orientation and rescales); a few triangles of tolerance is expected,
hundreds is not.

WHAT THIS TOOL DOES NOT DO
--------------------------
It does not decide anything. It reports what it found and flags what it could
not confirm, so a human can act on it. A `MATCH` here is evidence, not a
conclusion -- no licence text has been read, only the API's `license.label`.

USAGE
-----
    python tools/audit_gun_licences.py            # audit every file
    python tools/audit_gun_licences.py --json     # machine-readable output
"""

from __future__ import annotations

import argparse
import json
import struct
import sys
import urllib.parse
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
EXT_DIR = REPO / "assets" / "guns_ext"
API = "https://api.sketchfab.com/v3"

# Licences that are safe for this project. The project is AGPL-3.0 **and sells
# commercial licences**, so "safe" here means: permits commercial use AND
# permits redistribution.
SAFE = {
    "CC Attribution": "CC BY - commercial OK, redistribution OK, attribution required",
    "CC Attribution-ShareAlike": "CC BY-SA - commercial OK, share-alike applies to the artwork",
    "CC Attribution-NoDerivs": "CC BY-ND - commercial OK, but MODIFICATION is prohibited",
    "CC Attribution-NonCommercial": "CC BY-NC - COMMERCIAL USE PROHIBITED",
    "CC Attribution-NonCommercial-ShareAlike": "CC BY-NC-SA - COMMERCIAL USE PROHIBITED",
    "CC Attribution-NonCommercial-NoDerivs": "CC BY-NC-ND - COMMERCIAL USE PROHIBITED",
    "CC0 Public Domain": "CC0 - no restrictions",
}
BLOCKING = {k for k, v in SAFE.items() if "PROHIBITED" in v}
CAUTION = {"CC Attribution-NoDerivs", "CC Attribution-ShareAlike"}


def glb_tris(path: Path) -> tuple[int, int]:
    """(triangles, vertices) as stated by the GLB's own accessors."""
    d = path.read_bytes()
    ln = struct.unpack("<I", d[12:16])[0]
    j = json.loads(d[20 : 20 + ln])
    tris = 0
    verts = 0
    for mesh in j.get("meshes", []):
        for prim in mesh.get("primitives", []):
            if "indices" in prim:
                tris += j["accessors"][prim["indices"]]["count"] // 3
            verts += j["accessors"][prim["attributes"]["POSITION"]]["count"]
    return tris, verts


def slug_to_query(stem: str) -> str:
    """`low-poly_osv-96` -> `low poly osv-96`."""
    return stem.replace("_", " ").replace("-", " ").strip()


def api_get(url: str) -> dict:
    req = urllib.request.Request(url, headers={"User-Agent": "steel-front-licence-audit/1.0"})
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.load(r)


def search(query: str, limit: int = 12) -> list[dict]:
    q = urllib.parse.quote(query)
    url = f"{API}/search?type=models&downloadable=true&q={q}&count={limit}"
    try:
        return api_get(url).get("results", [])
    except Exception as e:  # network hiccup on one query should not kill the run
        print(f"    ! search failed: {e}", file=sys.stderr)
        return []


def audit_one(path: Path, tolerance: int = 0.02) -> dict:
    tris, verts = glb_tris(path)
    stem = path.stem
    query = slug_to_query(stem)
    rows = search(query)

    best = None
    for m in rows:
        fc = m.get("faceCount") or 0
        if fc == 0:
            continue
        delta = abs(fc - tris)
        # Tolerance is percentage-based: preprocessing rescales, which can drop a
        # handful of degenerate triangles, but never a large fraction.
        if delta <= max(8, tris * tolerance):
            if best is None or delta < best[0]:
                best = (delta, m)

    if best is None:
        return {
            "file": path.name, "tris": tris, "query": query, "status": "NO MATCH",
            "note": f"{len(rows)} downloadable result(s) searched, none within "
                    f"{tolerance:.0%} of {tris} tris",
        }

    delta, m = best
    lic = (m.get("license") or {}).get("label", "(none stated)")
    return {
        "file": path.name, "tris": tris, "query": query, "status": "MATCH",
        "delta": delta,
        "name": m.get("name"), "author": m.get("user", {}).get("displayName"),
        "username": m.get("user", {}).get("username"),
        "faces": m.get("faceCount"),
        "licence": lic,
        "verdict": SAFE.get(lic, f"UNRECOGNISED licence label: {lic}"),
        "url": m.get("viewerUrl") or m.get("uri"),
        "blocking": lic in BLOCKING,
        "caution": lic in CAUTION,
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args()

    files = sorted(EXT_DIR.glob("*.glb"))
    if not files:
        print(f"no .glb files under {EXT_DIR}", file=sys.stderr)
        return 2

    results = []
    for p in files:
        r = audit_one(p)
        results.append(r)
        if not args.json:
            print(f"\n{r['file']}   ({r['tris']} tris)")
            if r["status"] == "NO MATCH":
                print(f"   NO MATCH  - {r['note']}")
                continue
            print(f"   matched : {r['name']}  by {r['author']} (@{r['username']})")
            print(f"   faces   : {r['faces']}  (delta {r['delta']})")
            print(f"   licence : {r['licence']}")
            print(f"   verdict : {r['verdict']}")
            print(f"   url     : {r['url']}")

    if args.json:
        print(json.dumps(results, indent=2, ensure_ascii=False))
        return 0

    matched = [r for r in results if r["status"] == "MATCH"]
    blocking = [r for r in matched if r["blocking"]]
    caution = [r for r in matched if r["caution"]]
    print("\n" + "=" * 62)
    print(f"  {len(matched)}/{len(results)} identified")
    print(f"  {len(blocking)} with a licence that PROHIBITS commercial use")
    print(f"  {len(caution)} needing a judgement call (ND / share-alike)")
    print(f"  {len(results) - len(matched)} unresolved")
    print("=" * 62)
    return 1 if (blocking or len(matched) != len(results)) else 0


if __name__ == "__main__":
    sys.exit(main())
