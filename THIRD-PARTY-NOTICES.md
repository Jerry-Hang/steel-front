# Third-Party Notices, Asset Provenance, and Redistribution Checklist

**Required companion to [`LICENSE`](LICENSE). Do not separate the two when
redistributing this repository.**

This file exists because `LICENSE` covers only what the copyright holder owns.
Everything else bundled with — or referenced by — this project is listed here,
with its actual upstream terms where they could be verified.

> **Verification policy.** Every license statement below was read out of the
> relevant package's own source tree or `Cargo.toml` at the exact version this
> repository pins — **not** from memory and **not** from a summary page. Where
> something could not be verified, it is explicitly marked **UNVERIFIED** rather
> than guessed. See "How to re-verify" at the end.

---

## 1. Rust dependencies

Pinned versions are taken from `Cargo.lock`; license identifiers are taken from
each crate's own `Cargo.toml`, in `~/.cargo/registry/src/`.

| Crate | Pinned version | License (as declared) | Bundled license files |
|---|---|---|---|
| `ash` | 0.38.0+1.3.281 | MIT OR Apache-2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `ash-window` | 0.13.0 | MIT OR Apache-2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `env_logger` | 0.11.11 | MIT OR Apache-2.0 | 2 files |
| `glam` | 0.29.3 | MIT OR Apache-2.0 | 2 files |
| `image` | 0.25.10 | MIT OR Apache-2.0 | 2 files |
| `log` | 0.4.33 | MIT OR Apache-2.0 | 2 files |
| `naga` | 30.0.0 | MIT OR Apache-2.0 | 2 files |
| `raw-window-handle` | 0.6.2 | **MIT OR Apache-2.0 OR Zlib** | 3 files |
| `rspirv` | 0.11.0+1.5.4 | **Apache-2.0** (single) | none in tree |
| `winit` | 0.30.13 | **Apache-2.0** (single) | `LICENSE` |

**Two of these are worth calling out**, because the intuitive guess is wrong:

- **`winit` is Apache-2.0 only** — not the usual Rust "MIT OR Apache-2.0" dual
  license. If you previously assumed dual, that assumption was incorrect.
- **`rspirv` is Apache-2.0 only**, for the same reason.

### What these licenses require of you

All ten are permissive: they permit use, modification, and redistribution in
both source and binary form, including commercially. Their common obligations
are:

1. **Retain the copyright notice and license text** of every crate you
   redistribute. This matters chiefly if you ship a **binary** — a statically
   linked Rust binary contains their code, so their notices must accompany it.
2. **State significant changes** you made to their files (Apache-2.0 §4(b)).
   This project does not modify vendored crate sources, so this is normally
   moot — but it stops being moot if you patch a crate via `[patch]`.
3. **Do not use their names or trademarks** to endorse your derivative
   (Apache-2.0 §6).

> **In practice:** when you distribute a built `steel-front` binary, ship a copy
> of the licenses of these ten crates alongside it. `cargo-bundle-licenses` or
> `cargo about` can generate that bundle automatically; neither is currently a
> dependency of this project.

---

## 2. Assets created by this project — no third-party claim

The following are original works of the copyright holder and are covered by
`LICENSE`. They are listed here so that a redistributor can tell them apart
from the genuinely third-party material in §3 and §4.

| Path | How it is produced | Notes |
|---|---|---|
| `assets/props/**` (24 files) | `tools/blender/build_city_kit.py`, `gen_props.py` — headless Blender | City kit: buildings, trees, street furniture |
| `assets/soldier/soldier.glb` | `tools/blender/build_soldier.py` — headless Blender | 1082 verts / 540 tris, real-world proportions |
| `assets/maps/**` (6 files) | Hand-written TOML, parsed by `src/engine/map.rs` | `index`, `street_fight`, `open_field`, `factory_ambush`, `bridgehead`, `defense_line` |
| `assets/rt/**` (2 files) | Project-authored GLSL for the path tracer | Compiled by `scripts/compile_pt.ps1` |
| `assets/*.spv` | Generated **at build time** by `build.rs` from in-repo WGSL | Not an input; a build product |
| Terrain, city layout, textures | Generated procedurally at runtime by `src/engine/procedural.rs`, `src/engine/city.rs` | No external data files involved |

**Why this matters:** these directories are safe to redistribute. The
directories in §4 are not, without checking.

---

## 3. ⚠️ Font-derived glyph data — the highest-risk item in this repository

### What it is

`src/engine/cjk_glyphs.rs` is a ~21,500-line generated Rust file containing
pre-baked 12×12 pixel bitmaps for Chinese characters and CJK punctuation. Its
own header states the provenance:

```rust
//! 预烘焙 12x12 中文像素点阵（SimSun 宋体 12px 硬边位图）
//! 提取自 Windows 系统字体 SimSun（构建时一次性提取，运行时纯查表）
```

Translated: *"Pre-baked 12×12 Chinese pixel bitmap (SimSun 12px hard-edged
bitmap). Extracted from the Windows system font SimSun (extracted once at build
time; at runtime it is a pure lookup table)."*

### Why it is a problem

**SimSun (宋体) is a proprietary font owned by Microsoft and licensed from
ZhongYi Electronic Ltd.** The font files that ship with Windows are covered by
the Windows EULA, which — like essentially all commercial font licenses —
**prohibits redistribution and prohibits embedding the font, or a derived
representation of it, in another product**.

A bitmap rendering of a font is generally treated as a **derived work** of that
font, not as an independent creation. So baking these glyphs into a source file
and distributing that file — which is exactly what this repository does — sits
in the same legal territory as shipping the font itself.

This is a genuine, unresolved issue, not a theoretical one. **It is recorded
here rather than silently fixed, because the fix is a decision for the copyright
holder, not for a maintainer.**

### Aggravating factors specific to this repository

- **The extraction tool no longer exists.** There is no script under `tools/`
  that regenerates these bitmaps. The process was performed once, ad hoc, and
  the result was committed. The exact source font file and the extraction code
  are therefore **not reproducible from this repository**.
- **The data is 21,490 lines — roughly a third of `src/`** — so it cannot be
  casually regenerated without a replacement glyph source being chosen first.

### Recommended remediation, in order of preference

1. **Re-extract from a font licensed for this use.** Candidates with suitable
   CJK coverage and redistributable licenses include Noto Sans CJK / Source Han
   Sans (SIL OFL 1.1), WenQuanYi Zen Hei (GPL v2 + font exception), and
   Unifont (GPL v2 + font exception). **OFL-1.1 is the cleanest fit for an
   AGPL project**, because it imposes no copyleft obligation on the *software*
   that embeds the glyphs.
2. **Author a minimal bitmap font in-repo** covering only the characters the
   HUD actually uses. The HUD vocabulary is small and fixed, which makes this
   far more tractable here than in a general-purpose project.
3. **Ship no glyph data and require a system font at runtime.** Rejected as a
   default: it contradicts the project's "no external runtime dependencies"
   design constraint.

Whichever route is taken, the extraction process should be **committed as a
script under `tools/`** so the provenance stays auditable.

### Status: ✅ **RESOLVED on 2026-09-14**

The table was re-extracted from **Noto Sans SC** (SIL Open Font License 1.1)
and trimmed to the code points the source actually uses. The full OFL text is
in [`assets/fonts/OFL-NotoSansCJK.txt`](assets/fonts/OFL-NotoSansCJK.txt).

| | Before | After |
|---|---|---|
| Source font | SimSun (proprietary) | **Noto Sans SC (SIL OFL 1.1)** |
| Entries in table | 21,486 | **1,580** |
| File size | 2,256,253 B | **167,181 B** (−92.6%) |
| Generator in repo | **none** | `tools/extract_cjk_glyphs.py` |
| Redistribution blocked | **yes** | **no** |

**Why the trim, and how the required set is derived.** A scan of `src/**/*.rs`
(excluding the generated table) finds **1,580** distinct CJK code points in use,
against 21,486 shipped entries — so 19,906 glyphs were **dead weight that no code
referenced**. `tools/extract_cjk_glyphs.py --scan` regenerates that list
(`tools/cjk_used_codepoints.txt`), and the extractor refuses to write a table
that would not cover it.

**The contract is now enforced by a test.** `font_cjk::tests::cjk_glyph_generates`
used to assert `CJK_GLYPHS.len() > 20000` — a condition satisfiable by padding
the table with glyphs nothing uses, which is exactly how the dead weight
survived. It now asserts that the table covers every code point in
`cjk_used_codepoints.txt` **and contains nothing else**.

**Regenerating.** If a future change introduces characters outside the current
set, `cargo test` fails with the exact missing characters. The fix is:

```powershell
python tools/extract_cjk_glyphs.py --scan                                # refresh the used set
python tools/extract_cjk_glyphs.py --font <path-to-OFL-font>             # re-extract
```

**A note on the extraction parameters.** The first attempt produced glyphs that
sat 3–4 rows low with their bottom clipped, because PIL's *default* text anchor
is `"la"` (left-**ascender**) and a CJK glyph's em box extends **above** the
ascender. The extractor now passes `anchor="lt"` explicitly. If you point it at
a different face and the glyphs look vertically wrong, that is the first thing to
check — `tools/_calib_cjk.py` sweeps size/anchor/offset and reports the density
each combination achieves.

**Still worth doing (not required for redistribution):** the OFL requires that
derivative works not use the Reserved Font Name. Nothing here is named "Noto",
so no action is needed, but the attribution above must stay in place.

---

## 4. Third-party 3D models

`assets/guns/` (15 files) and `assets/guns_ext/` (14 files) contain weapon
models obtained from third-party sources — originally Sketchfab and comparable
sites. `tools/install_guns.py` and `tools/blender/prep_guns.py` record that the
source filenames were preserved through preprocessing, which is how the files
in these directories can be traced back.

### Current status: **partially audited (2026-09-14)**

**4 of 14 positively identified — all CC BY.** 2 more plausible (also CC BY).
**8 could not be traced** to a source page after a genuine effort; their
listings appear to have been withdrawn, since these are ~10-year-old uploads.
**No NonCommercial, ShareAlike, or NoDerivs licence appeared in any search.**

Full results, per-model attribution strings, and the recommendation are in
**[`docs/WEAPON-LICENCE-AUDIT.md`](docs/WEAPON-LICENCE-AUDIT.md)**.

The audit was automated by `tools/audit_gun_licences.py`, which identifies a
model by matching its **triangle count** against Sketchfab's API — a slug alone
is not an identification, and treating one as such produces fake verification.

The individual license of each model **has not been recorded in this
repository**. This is an acknowledged gap, not an oversight in this document.

Bundled CC licenses vary in ways that matter here:

| Variant | Commercial use | Redistribution | Attribution required |
|---|---|---|---|
| CC0 / Public Domain | Yes | Yes | No |
| CC BY | Yes | Yes | **Yes** |
| CC BY-SA | Yes | Yes, **under the same license** | **Yes** |
| CC BY-NC | **No** | Yes | **Yes** |
| CC BY-ND | Yes | **Unmodified only** | **Yes** |
| Sketchfab "Standard" | Depends on the individual listing | Usually restricted | Per listing |

**Note the trap:** a model under **CC BY-NC** cannot be used in a commercial
build at all, while one under **CC BY-SA** would impose a share-alike
obligation on the artwork that is distinct from — and additional to — the
AGPL-3.0 covering the code.

### Working document

**[`docs/WEAPON-LICENCE-AUDIT.md`](docs/WEAPON-LICENCE-AUDIT.md)** is the
worksheet for resolving this: all 14 source slugs, the slug→key mapping, the
licence decision table, the three leads found so far, and the two traps that
make a naive pass produce *fake* verification.

**The short version of the trap:** these files kept their Sketchfab slugs, so
they are findable — but a slug does **not** identify the asset. Searching
`pp-19-01_vityaz` returns models by two different authors under different
licences. Recording either without disambiguating produces a licence record that
looks verified and is not.

### Required action before redistribution

For **each** file in `assets/guns/` and `assets/guns_ext/`:

1. Identify the original source page.
2. Record: author, source URL, exact license identifier, and whether commercial
   use and redistribution are permitted.
3. Add the attribution string the license requires to this file.
4. **Remove any model whose license does not permit the intended use.**

Any model that cannot be traced back to a source page should be treated as
**not redistributable** and removed.

---

## 5. Summary table

| Component | Third-party? | Redistributable as-is? |
|---|---|---|
| Original source, build scripts, tools, docs | No | **Yes** |
| Procedurally generated props, soldier, terrain, city | No | **Yes** |
| Hand-written map TOML, PT shaders | No | **Yes** |
| Build-time `.spv` products | No | **Yes** |
| 10 Rust dependencies | Yes — MIT/Apache-2.0 | **Yes**, with notices bundled |
| CJK glyph bitmaps (§3) | ⚠️ Yes — derived from a proprietary font | **No — must be re-sourced first** |
| Gun models (§4) | Yes — per-asset | **Unknown — verify per asset** |

---

## 6. How to re-verify the claims in this file

The dependency licenses in §1 were read directly from the crates' own source
trees. To reproduce that check on any machine with this project's dependencies
fetched:

```powershell
# Where cargo unpacked each crate:
$reg = "$env:USERPROFILE\.cargo\registry\src"

# Print the declared license of every dependency, plus any bundled license files:
Get-ChildItem $reg -Directory | ForEach-Object {
    Get-ChildItem $_.FullName -Directory -Filter 'ash-*' -ErrorAction SilentlyContinue
} | ForEach-Object {
    $toml = Join-Path $_.FullName 'Cargo.toml'
    if (Test-Path $toml) {
        $lic = ([regex]'(?m)^\s*license\s*=\s*"([^"]+)"').Match(
            [IO.File]::ReadAllText($toml)).Groups[1].Value
        $files = (Get-ChildItem $_.FullName -File |
                  Where-Object Name -match '^(LICENSE|COPYING)').Name -join ', '
        "{0,-32} {1,-32} [{2}]" -f $_.Name, $lic, $files
    }
}
```

Substitute the crate name prefix to check the others. **Do not trust a summary
table — including the one above — over the package's own metadata.**

---

## 7. Reporting an error in this file

If any statement here is wrong or out of date, please open an issue at
<https://github.com/Jerry-Hang/steel-front/issues>. Accuracy matters more than
completeness in a document of this kind: an incorrect "this is fine" is worse
than an honest "UNVERIFIED".

---

*End of third-party notices.*
