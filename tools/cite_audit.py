"""Audit: judgement names cited in the docs must still be defined somewhere in this repo.

Why this exists
---------------
AGENTS.md / docs/PROGRESS.md enforce rules by *citing a name*:

    判据 = `swapchain_waits_are_bounded`
    逐轴倍率只走 `geom::Shape::visual_half_gain`

If that symbol is renamed or deleted the rule silently loses its enforcement, and the next
session goes looking for a ghost.  Two real cases from 2026-09-26:

  * AGENTS.md cited `geom::Shape::visual_half_gain` as the single source of a marker's visible
    half-extent; it had been renamed on 2026-09-17 (now `template_half_extent`).  The same
    clause also still claimed "visible size = 2x the collision box", long superseded -- a stale
    citation teaches the wrong fix even when the name resolves.
  * PROGRESS.md cited `pt_and_rt_enable_are_read_from_file`, which had been split into two
    tests when config wiring landed.

Same failure shape as the aarch64 target documented for months but never installed (PROGRESS
21.55) and survive_pm reading an always-empty log (§21.51): **"I wrote a check" and "the check
runs" are two different things.**

Two tiers, because one rule cannot be both complete and quiet
------------------------------------------------------------
Tier 1 -- HARD.  Names cited on a *judgement line* (判据 / 测试 / 回归 / 红测).  These are the
    sentences that promise enforcement, so an unresolved name is a defect: exit 1.
Tier 2 -- REVIEW.  Names cited anywhere else.  The docs legitimately name std / ash / glam /
    Blender-tool / log-field symbols, so a miss here is a prompt to look, not a failure.  It is
    printed (and, with --strict, fatal) because the keyword filter alone is a coverage hole:
    the very `visual_half_gain` line fixed on 2026-09-26 carried no judgement keyword and so
    was invisible to Tier 1.  Do not "fix" that by failing on every backticked token -- the
    first draft did that and produced 93 mostly-irrelevant hits (commit hashes, clippy lints,
    tool functions): repo lesson F, the matching pattern was more complex than the data.

Exit status (repo convention, see AGENTS lesson 46)
---------------------------------------------------
  0 = scanned; no unresolved Tier-1 name (Tier-2 misses, if any, are listed as review)
  1 = scanned; an unresolved Tier-1 name (or any Tier-2 miss under --strict)
  2 = NOT SCANNED (docs missing, sources missing, nothing parsed) -- never a pass
"""
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

DOCS = (Path("AGENTS.md"), Path("docs/PROGRESS.md"))
SRC_ROOTS = (Path("src"), Path("build.rs"), Path("build_spv_rt.rs"))
TOOL_ROOT = Path("tools")

JUDGE = re.compile(r"判据|测试|回归|红测")
# Backticked identifier, optionally `a::b::name`.  The last segment is what gets resolved.
TOK = re.compile(r"`([A-Za-z_][A-Za-z0-9_:]*)`")
HEX7 = re.compile(r"^[0-9a-f]{7}$")
FILEISH = re.compile(r"\.(rs|py|ps1|md|toml|glb|spv|json|txt|bat|sh|glsl|wgsl|png|log)$")

# Identifiers quoted on judgement lines that are not repo symbols at all.
NOT_A_SYMBOL = {
    "cast_possible_truncation", "cast_precision_loss", "float_cmp", "doc_markdown",
    "uninlined_format_args", "unreadable_literal", "global_needs_wrapper", "suspicious",
    "correctness", "complexity", "get_unchecked", "transmute",
    "device_lost", "panics",
}

# Names that were real once and are cited only as history.  Each entry needs a reason **and a
# replacement**, otherwise it is just a way to silence the gate.
RETIRED = {
    "visual_half_gain": (
        "renamed 2026-09-17; the visible half-extent now has one source, "
        "`geom::Shape::template_half_extent` (test `marker_visible_size_matches_aabb`)"),
    "pt_and_rt_enable_are_read_from_file": (
        "split into two tests when config wiring landed: `pt_enable_is_read_from_file` + "
        "`pt_exposure_is_read_and_clamped`"),
}

# Definition index.  Over-collecting is safe: it can only turn a reported miss into a hit, and
# a false negative is cheaper than a false red (lesson 26).
DEF_RUST = re.compile(
    r"\b(?:fn|mod|struct|enum|trait|union|type)\s+([A-Za-z_][A-Za-z0-9_]*)"
    r"|\b(?:const|static)\s+(?:unsafe\s+)?(?:fn\s+)?([A-Za-z_][A-Za-z0-9_]*)")
DEF_FIELD = re.compile(
    r"(?:^\s*|[{,(]\s*)(?:pub(?:\([^)]*\))?\s+)?(?:mut\s+)?([a-z_][a-z0-9_]*)\s*:", re.M)
DEF_LET = re.compile(r"\blet\s+(?:mut\s+)?([a-z_][a-z0-9_]*)")
DEF_PY = re.compile(r"^\s*def\s+([A-Za-z_][A-Za-z0-9_]*)", re.M)


def build_definition_index(roots=SRC_ROOTS, tool_root=TOOL_ROOT) -> "tuple[set, int, list[str]]":
    """Every identifier this repo declares, plus tool-script names and their file stems.

    `roots` / `tool_root` are parameters so the self-test can point at a synthetic tree --
    otherwise it would "pass" by resolving synthetic names against the real repo, which is the
    exact failure this tool exists to catch (lesson 27).
    """
    defs, sources = set(), []
    for root in roots:
        if root.is_dir():
            sources.extend(sorted(root.rglob("*.rs")))
        elif root.is_file() and root.suffix == ".rs":
            sources.append(root)
    for path in sources:
        text = path.read_text(encoding="utf-8", errors="replace")
        for groups in DEF_RUST.findall(text):
            defs.update(g for g in groups if g)
        defs.update(DEF_FIELD.findall(text))
        defs.update(DEF_LET.findall(text))

    scripts = sorted(tool_root.rglob("*.py")) if tool_root.is_dir() else []
    for path in scripts:
        defs.add(path.stem)                     # `history_secret_audit` names its own file
        defs.update(DEF_PY.findall(path.read_text(encoding="utf-8", errors="replace")))
    return defs, len(sources), [str(p) for p in scripts]


def is_checkable(raw: str) -> bool:
    """Shape filter.  Unqualified names must look like this repo's own symbols (lowercase
    snake_case, >= 3 words); path-qualified names are checked at >= 2 words regardless of
    case, so `geom::Shape::visual_half_gain` cannot hide behind the case rule."""
    if FILEISH.search(raw) or "/" in raw or raw.startswith("RV3D"):
        return False
    name = raw.split("::")[-1]
    if not name or HEX7.match(name) or name in NOT_A_SYMBOL:
        return False
    if "::" in raw:
        return len(name.split("_")) >= 2
    return name[0].islower() and len(name.split("_")) >= 3


def collect_citations(docs) -> "tuple[dict, dict, int]":
    """Split citations into (tier1, tier2): judgement lines vs everything else."""
    tier1: "dict[str, set]" = {}
    tier2: "dict[str, set]" = {}
    scanned = 0
    for doc in docs:
        if not doc.is_file():
            continue
        scanned += 1
        for line in doc.read_text(encoding="utf-8", errors="replace").splitlines():
            bucket = tier1 if JUDGE.search(line) else tier2
            for raw in TOK.findall(line):
                if not is_checkable(raw):
                    continue
                bucket.setdefault(raw.split("::")[-1], set()).add(doc.name)
    return tier1, tier2, scanned


def audit(docs=DOCS, roots=SRC_ROOTS, tool_root=TOOL_ROOT, strict=False,
          retired=None) -> "tuple[int, list[str]]":
    retired = RETIRED if retired is None else retired
    defs, n_rust, scripts = build_definition_index(roots, tool_root)
    if n_rust == 0:
        return 2, ["NO-SCAN: no .rs sources under %s" % ", ".join(str(r) for r in roots)]
    if not defs:
        return 2, ["NO-SCAN: %d .rs file(s) parsed but no definitions extracted" % n_rust]

    tier1, tier2, n_docs = collect_citations(docs)
    if n_docs == 0:
        return 2, ["NO-SCAN: none of %s exist (run from the repo root)"
                   % ", ".join(str(d) for d in docs)]
    if not tier1 and not tier2:
        return 2, ["NO-SCAN: %d doc(s) scanned but no citations matched" % n_docs]

    def unresolved(bucket):
        return sorted(n for n in bucket if n not in defs and n not in retired)

    lines = ["scanned %d .rs + %d tool .py (%d definitions), %d doc(s): "
             "tier1=%d tier2=%d cited names"
             % (n_rust, len(scripts), len(defs), n_docs, len(tier1), len(tier2))]

    known_retired = sorted(n for n in set(tier1) | set(tier2) if n in retired)
    if known_retired:
        lines.append("RETIRED (cited as history, replacement named):")
        for name in known_retired:
            lines.append("   %-38s %s" % (name, retired[name]))

    bad1, bad2 = unresolved(tier1), unresolved(tier2)
    if bad1:
        lines.append("TIER 1 -- cited as a JUDGEMENT but defined nowhere (%d):" % len(bad1))
        for name in bad1:
            lines.append("   %-38s cited in %s" % (name, ", ".join(sorted(tier1[name]))))
    if bad2:
        lines.append("TIER 2 -- cited in prose, defined nowhere in src/ or tools/ (%d) "
                     "[may be std/ash/glam/Blender API or a log field -- review, not a verdict]:"
                     % len(bad2))
        for name in bad2:
            lines.append("   %-38s cited in %s" % (name, ", ".join(sorted(tier2[name]))))

    if bad1:
        lines.append("=> a judgement line names a ghost: fix the citation, or add it to RETIRED "
                     "**with a replacement** (a retirement without one just silences the gate)")
        return 1, lines
    if bad2 and strict:
        lines.append("=> --strict: Tier-2 misses are fatal in this mode")
        return 1, lines
    lines.append("OK: every tier-1 judgement name resolves%s"
                 % ("" if not bad2 else " (tier 2: %d to review)" % len(bad2)))
    return 0, lines


def self_test() -> int:
    """Prove the tool can fail, on a synthetic tree it cannot share with the real repo."""
    checks = []

    def check(label, got, want):
        checks.append((label, got, want))

    tmp = Path("./.cite_audit_selftest")
    tmp.mkdir(exist_ok=True)
    try:
        src, tools_dir = tmp / "src", tmp / "tools"
        src.mkdir(exist_ok=True)
        tools_dir.mkdir(exist_ok=True)
        (src / "a.rs").write_text(
            "pub struct R { pub last_npc_box_near: u32 }\n"
            "pub const NPC_SLOT_BASE: usize = 65601;\n"
            "pub const fn template_half_extent(self, axis: usize) -> f32 { 0.0 }\n"
            "fn swapchain_waits_are_bounded() {}\n"
            "fn f() { let mesh_shader_available = 1; }\n", encoding="utf-8")
        (tools_dir / "history_secret_audit.py").write_text(
            "def scan_repo():\n    pass\n", encoding="utf-8")
        doc = tmp / "DOC.md"
        roots = [src]

        def run(text, **kw):
            doc.write_text(text, encoding="utf-8")
            return audit([doc], roots, tools_dir, **kw)

        # 1) every definition shape this repo actually uses must resolve
        code, out = run("判据 = `swapchain_waits_are_bounded` 与 `geom::Shape::template_half_extent`\n"
                        "字段 `last_npc_box_near`、常量 `NPC_SLOT_BASE`、局部 `mesh_shader_available`\n"
                        "工具 `history_secret_audit` 与 `scan_repo`\n", retired={})
        check("tier1 real test name resolves (exit 0)", code, 0)
        check("`const fn` / const / field / local / tool all resolve",
              any("TIER" in l for l in out), False)

        # 2) the synthetic index must NOT silently borrow the real repo's symbols
        code, out = run("判据 = `marker_visible_size_matches_aabb`\n", retired={})
        check("no leakage from the real repo (exit 1)", code, 1)
        check("leaked name is reported", any("TIER 1" in l for l in out), True)

        # 3) a ghost on a judgement line is fatal
        code, out = run("判据 = `this_test_never_existed_anywhere`\n", retired={})
        check("tier1 ghost is fatal (exit 1)", code, 1)
        check("tier1 ghost is named",
              any("this_test_never_existed_anywhere" in l for l in out), True)

        # 4) a ghost on a NON-judgement line is still surfaced (the 2026-09-26 coverage hole)
        code, out = run("逐轴倍率只走 `geom::Shape::some_ghost_helper_name`，别各自再写 2.0。\n",
                        retired={})
        check("tier2 ghost is surfaced", any("TIER 2" in l for l in out), True)
        check("tier2 ghost alone is not fatal", code, 0)
        check("tier2 ghost is fatal under --strict",
              run("逐轴倍率只走 `geom::Shape::some_ghost_helper_name`。\n",
                  strict=True, retired={})[0], 1)

        # 5) a documented retirement passes but stays visible
        code, out = run("判据 = `this_test_never_existed_anywhere`\n",
                        retired={"this_test_never_existed_anywhere": "gone in 2020"})
        check("documented retirement passes (exit 0)", code, 0)
        check("retirement is still printed", any("RETIRED" in l for l in out), True)

        # 6) empty / missing input must never pass
        check("no docs -> exit 2", audit([tmp / "nope.md"], roots, tools_dir, retired={})[0], 2)
        check("no .rs sources -> exit 2", audit([doc], [tmp / "nosrc"], tools_dir, retired={})[0], 2)
        (src / "a.rs").write_text("// nothing declared here\n", encoding="utf-8")
        code, _ = audit([doc], roots, tmp / "no_tools", retired={})
        check("sources without definitions -> exit 2", code, 2)
    finally:
        for p in sorted(tmp.rglob("*"), reverse=True):
            p.unlink() if p.is_file() else p.rmdir()
        tmp.rmdir()

    bad = [c for c in checks if c[1] != c[2]]
    for label, got, want in checks:
        print("  %-46s %s (want %s)" % (label, got, want))
    print("self-test: %d/%d checks pass" % (len(checks) - len(bad), len(checks)))
    return 0 if not bad else 1


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--self-test", action="store_true",
                    help="verify this tool can fail, without touching repo state")
    ap.add_argument("--strict", action="store_true",
                    help="treat Tier-2 (prose) misses as fatal too")
    args = ap.parse_args()
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
        sys.stderr.reconfigure(encoding="utf-8", errors="replace")
    if args.self_test:
        return self_test()

    code, lines = audit(strict=args.strict)
    for line in lines:
        print(line, file=sys.stderr if code == 2 else sys.stdout)
    return code


if __name__ == "__main__":
    sys.exit(main())
