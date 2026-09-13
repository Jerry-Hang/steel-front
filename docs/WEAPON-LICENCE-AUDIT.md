# Weapon Model Licence Audit — Worksheet

**Status: ⚠️ INCOMPLETE — 14 models pending verification.**

Companion to [`THIRD-PARTY-NOTICES.md`](THIRD-PARTY-NOTICES.md) §4. That section
states the problem; this file is the working document for resolving it.

> **Why this cannot be filled in automatically.**
> Every model in `assets/guns_ext/` kept its **original download filename**, which
> is the Sketchfab asset slug — so each one *is* findable. But the slug alone does
> **not** identify the exact asset. Searching `pp-19-01_vityaz`, for example,
> returns models by **at least two different authors** (SpatialNeglect and Sota
> 3007) with different licences. Guessing which one was downloaded years ago
> would produce a licence record that *looks* verified and is not — worse than
> leaving the cell blank. **Fill each row from the actual source page, and record
> the URL so the claim can be re-checked.**

---

## How to complete a row

1. Search the slug. Start from the slug with underscores/dashes normalised to
   spaces, e.g. `low-poly_osv-96` → `low poly OSV-96`.
2. **Disambiguate before recording anything.** Confirm the match using at least
   one of: triangle/vertex count, file size, thumbnail silhouette, or the
   downloader's own record. A matching name is not a matching asset.
3. Read the licence **on the source page** — not from the search snippet.
4. Fill in author, URL, licence identifier, and the two permission columns.
5. If any column cannot be established, write **UNKNOWN** rather than leaving it
   blank, so the row reads as "still open" instead of "done".

### Licence decision table

| Licence | Commercial use | Redistribution | Attribution | Notes |
|---|---|---|---|---|
| CC0 / Public Domain | ✅ | ✅ | not required | ideal |
| CC BY 4.0 | ✅ | ✅ | **required** | record the attribution string |
| CC BY-SA 4.0 | ✅ | ✅, **under the same licence** | **required** | adds a share-alike obligation on the *artwork*, separate from the AGPL covering the code |
| CC BY-NC * | ❌ | ✅ | **required** | **cannot ship in a commercial build at all** |
| CC BY-ND | ✅ | **unmodified only** | **required** | the .glb is preprocessed, so this likely fails |
| Sketchfab "Standard" | per listing | usually restricted | per listing | read the individual page |

**Two traps specific to this project:**

- **CC BY-SA** is not "fine because we're already AGPL". The two licences cover
  different subject matter and both apply.
- **CC BY-ND** permits redistribution of the *unmodified* work. These files were
  run through `tools/blender/prep_guns.py`, which normalises orientation and
  rescales. **If a model turns out to be ND, that preprocessing is itself
  arguably a prohibited derivative** — flag it rather than assuming it is fine.

---

## A. `assets/guns_ext/` — original downloads (14 files, 13 in use)

Slug → in-game key mapping is from `KEY_MAP` in `tools/install_guns.py`.

| # | Source slug (as downloaded) | In-game key | Source page URL | Author | Licence | Commercial? | Redistributable? |
|---|---|---|---|---|---|---|---|
| 1 | `as_val` | `asval` | | | | | |
| 2 | `ash_12.7__assault_rifle_shak_12` | `ash12` | *see lead below* | | | | |
| 3 | `komrad_12_saiga_12` | `saiga12` | *see lead below* | | | | |
| 4 | `low-poly_mp-443_grach` | `mp443` | | | | | |
| 5 | `low-poly_osv-96` | `osv96` | | | | | |
| 6 | `low-poly_rpk-16` | `rpk16` | | | | | |
| 7 | `low_poly_ak104` | `ak104` | | | | | |
| 8 | `pkm` | `pkm` | | | | | |
| 9 | `pkp` | `pkp` | | | | | |
| 10 | `pp-19-01_vityaz` | `pp19` | *see lead below* | | | | |
| 11 | `pp-19_bizon` | `pp9` | | | | | |
| 12 | `sv98` | `sv98` | | | | | |
| 13 | `vss_vintorez` | `vss` | | | | | |
| 14 | `svd_63_-_dragunov` | *(SKIP)* | | | | | not installed — see note A1 |

### Leads already found (verify before recording)

These came from a web search on 2026-09-14 and are **starting points, not
results** — none has been confirmed as the actual asset, and no licence has been
read from a source page yet.

- **#2 `ash_12.7__assault_rifle_shak_12`** — a Sketchfab listing titled *"Ash 12.7
  Assault Rifle SHAK 12"*, marked "Download Free 3D model", by **EastSeaSaltfishnet**:
  <https://sketchfab.com/3d-models/ash-127-assault-rifle-shak-12-92069a3fe95644e9961cc80b10ec0605>
- **#10 `pp-19-01_vityaz`** — ⚠️ **ambiguous, at least two candidates.**
  *"PP-19-01 Vityaz"* by **SpatialNeglect** ("Download Free 3D model"):
  <https://sketchfab.com/3d-models/pp-19-01-vityaz-6d4a89d483374668b05343e71ed7da78>
  — and a *different* model, *"PP-19-01 Vityaz-SN Modified SMG"*, by **Sota 3007**:
  <https://sketchfab.com/3d-models/pp-19-01-vityaz-sn-modified-smg-0074eb2b075d4b8c83e8f867f950a1a4>
  **Determine which one this file is before recording either.**
- **#3 `komrad_12_saiga_12`** — searches surfaced only **non-Sketchfab** mirrors
  (gta5-mods, 3dwhere), not an original listing. **The upstream source is
  unknown**, so its licence is unknown. This is the hardest row in the table.

### Note A1 — `svd_63_-_dragunov`

Deliberately **not installed**. `tools/install_guns.py` records the reason: the
source file is a product-render scene containing **two complete rifle bodies at
90° to each other plus a detached optic**, so it cannot be used as-is. Fixing it
means deleting one body in Blender first. Its licence should still be traced —
the file sits in the repository and is redistributed with it regardless of
whether the game loads it.

---

## B. `assets/guns/` — processed / working copies (15 files)

These are the files the engine actually loads. Most are the preprocessed output
of the corresponding `guns_ext` entry, and inherit that entry's licence — so
**verifying §A covers most of §B**. Two need separate attention:

| File | Origin | Licence follows |
|---|---|---|
| `ak104.glb` `ash12.glb` `asval.glb` `mp443.glb` `osv96.glb` `pkm.glb` `pkp.glb` `pp19.glb` `pp9.glb` `rpk16.glb` `saiga12.glb` `sv98.glb` `vss.glb` | preprocessed from §A | **row #N above** |
| `ak12.glb` | ⚠️ **not in `KEY_MAP`** | **trace separately** — it is the fallback model used when a weapon key has no dedicated file |
| `ak12_baked.glb` | derived from `ak12.glb` by the light-baking step | same as `ak12.glb` |

---

## C. What "done" looks like

1. Every row in §A has a URL, an author, a licence identifier, and both
   permission columns filled — **or** the row is explicitly marked UNKNOWN.
2. `ak12.glb` and `ak12_baked.glb` are traced (§B).
3. Models whose licence forbids commercial use are **deleted** from the
   repository, and `KEY_MAP` is updated so `install_guns.py` still validates.
4. Required attribution strings are pasted into `THIRD-PARTY-NOTICES.md` §4.
5. That section's status line changes from **UNVERIFIED, per asset** to a
   statement of what was actually verified, with the date.

**Until step 5, the honest description of this repository remains: the code is
clean to redistribute, and the weapon art is not yet cleared.**

---

## D. If a model has to go

Removing an entry is not just `rm`. In order:

```powershell
# 1. Remove the source and the working copy
Remove-Item assets/guns_ext/<slug>.glb
Remove-Item assets/guns/<key>.glb

# 2. Drop the mapping so install_guns.py stops expecting the file
#    (edit KEY_MAP in tools/install_guns.py)

# 3. The engine falls back to ak12.glb for any weapon key with no dedicated
#    model, so the game keeps working; check the weapon still looks sane.

# 4. Re-run the licence check that install_guns.py itself performs
python tools/install_guns.py --check
```

Step 3 is the reason a missing model is not a hard failure: `main.rs`'s
`load_gun_glb` falls back to `assets/guns/ak12.glb` when `assets/guns/<key>.glb`
does not exist. **That fallback is load-bearing for this audit** — it means
removing an unlicensed model degrades the weapon's appearance, not the build.
