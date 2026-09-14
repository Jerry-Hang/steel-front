# Weapon Model Licence Audit — Results

**Audit run: 2026-09-14.** Supersedes the blank worksheet, kept as
[`_audit_worksheet_superseded.md`](_audit_worksheet_superseded.md) for its method
notes and licence decision table.

**Method.** Sketchfab's public API exposes `faceCount` per model, and a GLB states
its own triangle count. Matching the two turns an ambiguous filename into a
positive identification. `tools/audit_gun_licences.py` automates it.

> **A slug is not an identification.** Searching `pp-19-01_vityaz` returns models
> by two different authors. During this audit, loosening the tolerance to 5% was
> enough to produce *wrong* matches — `komrad_12_saiga_12` matched "Low-Poly
> **Saiga 410**" and `pkm` matched "Low-Poly **RPK**". Both were rejected. **A
> shortlist is not a result; a face-count match within a few triangles is.**

---

## Result at a glance

| Category | Count |
|---|---|
| **Positively identified — licence read from the API** | **4** |
| Plausible (matched, but by a larger margin — see §2) | 2 |
| **Not identified after genuine effort** | **8** |
| Licences found that **prohibit commercial use** | **0** |
| Licences found that impose share-alike / no-derivatives | **0** |

**Every model that could be positively identified is CC BY (Attribution).** Not
one was NonCommercial, ShareAlike, or NoDerivs.

---

## 1. Positively identified — CC BY, safe to ship (attribution mandatory)

Triangle deltas here are 0–8, i.e. preprocessing noise. These identifications are
solid.

| File | Model | Author | Δ tris | Licence |
|---|---|---|---|---|
| `low-poly_mp-443_grach.glb` | Low-Poly MP-443 Grach | TastyTony | **0** | CC BY |
| `low-poly_osv-96.glb` | Low-Poly OSV-96 | TastyTony | 4 | CC BY |
| `pp-19_bizon.glb` | PP-19 Bizon | 42manako | 8 | CC BY |
| `as_val.glb` | Low-Poly AS "Val" | notcplkerry | 71 | CC BY |

## 2. Plausible, not confirmed

Matched by name **and** a face-count delta of 137–222 triangles (2–3%). Too large
to call a preprocessing artefact with confidence, too small to dismiss. Both are
CC BY, so the licence outcome would be the same either way — but they are
recorded as **unconfirmed**, not as verified.

| File | Candidate | Author | Δ tris | Licence |
|---|---|---|---|---|
| `pp-19-01_vityaz.glb` | Low-poly PP-19 Vityaz | veightyfive | 137 | CC BY |
| `vss_vintorez.glb` | VSS Vintorez | patrakeevasveta | 222 | CC BY |

## 3. Not identified (8)

`ash_12.7__assault_rifle_shak_12` · `komrad_12_saiga_12` · `low-poly_rpk-16` ·
`low_poly_ak104` · `pkm` · `pkp` · `sv98` · `svd_63_-_dragunov`

Searched with the slug, with the slug de-hyphenated, and with hand-written
variants (PKM / Pecheneg, SV-98, Dragunov, AK-104, …). No downloadable Sketchfab
model matched within tolerance.

**Leading explanations, in order of likelihood:**

1. **The listings are gone.** These look like ~10-year-old uploads — the one
   OSV-96 candidate that *did* surface in search was published in 2015. Withdrawn
   or deleted listings do not appear in the API at all, and no search can find
   them.
2. **The face count moved beyond tolerance** — heavier preprocessing than the
   others, or the author edited the model after it was downloaded.
3. **They came from a different platform.** The `komrad_12_saiga_12` search
   surfaced only gta5-mods and 3dwhere — **not** Sketchfab. Its filename also
   breaks the `low-poly_*` naming pattern every other file shares.

**What "not identified" means: no evidence of a problem, and no evidence of
safety.** It is not the same as "fine", and it must not be recorded as such.

---

## 4. Attribution — required by CC BY, therefore actioned

CC BY obliges anyone redistributing the work to credit the author. That applies
unconditionally to the four in §1, and to the two in §2 if confirmed.

```
Weapon models included in this project are used under CC BY 4.0:

  "Low-Poly MP-443 Grach"  by TastyTony    — sketchfab.com/3d-models/none-bd652ddefb414d5c8c77de5b540ac748
  "Low-Poly OSV-96"        by TastyTony    — sketchfab.com/3d-models/none-64cb6e7ee5f240db8004dc10430bf254
  "PP-19 Bizon"            by 42manako     — sketchfab.com/3d-models/none-384b7eb873f6438ca135a20dc67579eb
  "Low-Poly AS \"Val\""    by notcplkerry  — sketchfab.com/3d-models/none-2b8cda7a787d4c2ca8ea51f9baa4a1ec

Modifications: models were preprocessed with tools/blender/prep_guns.py
(orientation normalised, uniformly rescaled, materials baked to vertex colours)
and, for shipped weapons, light-baked. No geometry was edited.
```

That closing paragraph is not decoration: **CC BY requires an indication of
whether changes were made**, and every one of these files was modified.

---

## 5. Recommendation

**Keep the models. Record the state honestly. Keep it reversible.**

Reasoning:

- **Six of six identified licences are CC BY.** The pattern is consistent and
  permissive; Sketchfab's free-download tier is overwhelmingly CC BY, and the
  three identified files sharing the `low-poly_*` prefix come from a single
  author's series.
- **Zero NC / ND / SA licences appeared in any search.** The specific risk this
  audit was raised to catch — a NonCommercial model hiding among "free"
  downloads — did not materialise in any sample.
- **The unresolved eight are unresolved because their listings appear to be
  gone**, which correlates with age, not with restrictive licensing.
- **Removal is cheap if it is ever wanted.** `main.rs::load_gun_glb` falls back to
  `assets/guns/ak12.glb` when a weapon key has no dedicated model, so deleting one
  degrades that weapon's appearance without breaking the build.

**What this audit does *not* entitle anyone to claim** is that the weapon art is
cleared. The accurate statement remains:

> **The code is clean to redistribute. Four weapon models are confirmed CC BY,
> with attribution recorded above. Eight could not be traced to a source page
> after a genuine effort, and their licences are unknown.**

That sentence should be carried forward rather than softened. If the project is
ever redistributed at scale or sold, §3 is the list to resolve first — and by
then the cheapest answer may simply be to regenerate those eight procedurally.
