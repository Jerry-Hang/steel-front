# Weapon Model Licence Audit — Results

## 🟢 处置结果（2026-10-30）—— 未识别的一律移出仓库

审计过程保留在下面（§1–§5 原样），**处置动作**如下：

| 动作 | 内容 |
|---|---|
| 移出仓库 | `ak104` `ash12` `pkm` `pkp` `rpk16` `saiga12` `sv98` `svd12`（8 件，追不到原始出处，许可证未知） |
| 一并移出 | `assets/guns_ext/**`（15 件，与 `assets/guns/**` **逐字节相同**的重复副本，7 MB；原始 slug 已抄进下表） |
| 保留 | `ak12` `ak12_baked` `asval` `mp443` `osv96` `pp19` `pp9` `vss`（8 件） |
| 新增确认 | **`ak12.glb` 自带 GLB 元数据**：`extras.author = momsboxtv`、`extras.license = CC-BY-4.0` ⇒ 第 5 件**硬证据**确认的 CC BY |

移出后**不需要改代码**：`load_gun_glb` 对缺模型回退 `assets/guns/ak12.glb`，
实测（2026-10-30，headless）只加载 1 件枪模、fps 249、无错误路径命中，只有外观退化。

**回归判据**：`gun_glb_indices_all_in_range` 的下限 10 → **8**（资产清单的事实值）；
`cargo test --release` = **880 passed**。

要恢复某件枪：**先补来源页与许可证**，再拷回 `assets/guns/`，并把下限改回去。

---

## 现有资产的来源与许可（逐件，2026-10-30 实测）

| 文件 | 原始 slug | sha256(前16) | 三角 | 许可状态 |
|---|---|---|---|---|
| `ak104.glb` | `low_poly_ak104` | `2e75682dbc04c57a` | 6784 | **未识别 — 2026-10-30 移出仓库** |
| `ak12.glb` | `—(元数据内含作者/许可)` | `80d39627c1c7ad15` | 23493 | CC BY 4.0 — momsboxtv（GLB 内 extras 自带） |
| `ak12_baked.glb` | `derived from ak12` | `462bb205d059cc19` | 23493 | 本仓派生物（源自 ak12） |
| `ash12.glb` | `ash_12.7__assault_rifle_shak_12` | `70517db33c4ceb1c` | 9629 | **未识别 — 2026-10-30 移出仓库** |
| `asval.glb` | `as_val` | `b5abe6086bd89120` | 6434 | CC BY — notcplkerry（Δ71） |
| `mp443.glb` | `low-poly_mp-443_grach` | `3a6a2917370bd2c2` | 3868 | CC BY — TastyTony（Δ0） |
| `osv96.glb` | `low-poly_osv-96` | `3ed1b8d13c371bf0` | 6717 | CC BY — TastyTony（Δ4） |
| `pkm.glb` | `pkm` | `b62c1572ccde9092` | 6750 | **未识别 — 2026-10-30 移出仓库** |
| `pkp.glb` | `pkp` | `5aafdbf6e5bf59b0` | 5002 | **未识别 — 2026-10-30 移出仓库** |
| `pp19.glb` | `pp-19-01_vityaz` | `4c0466700a502e59` | 4771 | CC BY（较可能）— veightyfive（Δ137） |
| `pp9.glb` | `pp-19_bizon` | `ccd33c0d2dc557fc` | 11195 | CC BY — 42manako（Δ8） |
| `rpk16.glb` | `low-poly_rpk-16` | `85170ae2c3c3c050` | 3864 | **未识别 — 2026-10-30 移出仓库** |
| `saiga12.glb` | `komrad_12_saiga_12` | `9f6da917b2a323f1` | 9156 | **未识别 — 2026-10-30 移出仓库** |
| `sv98.glb` | `sv98` | `356015d5e22a2478` | 4906 | **未识别 — 2026-10-30 移出仓库** |
| `svd12.glb` | `svd_63_cleaned` | `4567bf520d7dfa97` | 3374 | **未识别 — 2026-10-30 移出仓库** |
| `vss.glb` | `vss_vintorez` | `67bd3a6f1dc0e648` | 9692 | CC BY（较可能）— patrakeevasveta（Δ222） |


> `ak12_baked.glb` 是本仓对 `ak12.glb` 的**烘焙派生物**（作者与许可随源文件走）。

---

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
| `ak12.glb` | AK-12M | **momsboxtv** | — | **CC BY 4.0（GLB `extras` 自带，非推测）** |

## 2. Plausible, not confirmed

Matched by name **and** a face-count delta of 137–222 triangles (2–3%). Too large
to call a preprocessing artefact with confidence, too small to dismiss. Both are
CC BY, so the licence outcome would be the same either way — but they are
recorded as **unconfirmed**, not as verified.

| File | Candidate | Author | Δ tris | Licence |
|---|---|---|---|---|
| `pp-19-01_vityaz.glb` | Low-poly PP-19 Vityaz | veightyfive | 137 | CC BY |
| `vss_vintorez.glb` | VSS Vintorez | patrakeevasveta | 222 | CC BY |

## 3. Not identified (8) — **removed from the repository on 2026-10-30**

`ash_12.7__assault_rifle_shak_12` · `komrad_12_saiga_12` · `low-poly_rpk-16` ·
`low_poly_ak104` · `pkm` · `pkp` · `sv98` · `svd_63_-_dragunov`

Searched with the slug, de-hyphenated, and with hand-written variants (PKM /
Pecheneg, SV-98, Dragunov, AK-104 …). No downloadable Sketchfab model matched
within tolerance. A **2026-10-30 re-run** — same method plus a 3% tolerance and
more query variants (`shak-12`, `saiga 12`, `rpk-16`, `ak-104`, `pecheneg`,
`svd-63` …) — produced exactly **one** weak candidate (`pkp` → a model named
`objects` by "PECENEG", 2.4% off). That is a shortlist item, not an
identification, and was rejected as such.

**Disposition: removed.** With the source and licence unknown, the files are no
longer redistributed. `assets/guns_ext/**` (byte-identical duplicates kept for
provenance) went in the same commit; the original slugs are preserved in the
table above, which is exactly what a future re-identification needs.

**Cost:** none in code — the eight weapons fall back to the AK-12 model.

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
  "AK-12M"                 by momsboxtv    — https://sketchfab.com/momsboxtv
                                         (author/licence read from the GLB's own
                                          `asset.extras`: license = CC-BY-4.0)

Modifications: models were preprocessed with tools/blender/prep_guns.py
(orientation normalised, uniformly rescaled, materials baked to vertex colours)
and, for shipped weapons, light-baked. No geometry was edited.
```

That closing paragraph is not decoration: **CC BY requires an indication of
whether changes were made**, and every one of these files was modified.

---

## 5. Recommendation — superseded by the 2026-10-30 disposition above

The original advice was "keep the models, record the state honestly, keep it
reversible". That was written before this repository had a licensing structure
of its own. After the 2026-10-30 rework (verbatim AGPL text + a §7 additional
permission, see [`NOTICE`](../NOTICE)), the project publishes under **stated
terms** — and a repository cannot both promise a licence and redistribute art
that nobody can license.

So the disposition flipped from "keep and disclose" to **"remove and disclose"**:
8 models with confirmed or near-confirmed CC BY stay (attribution below), the
other 8 are gone, and the originals live outside the repository in case a source
page surfaces later.

