#!/usr/bin/env python3
"""Check the battle-intel tally invariant in a stress-battle log.

Invariant (2026-09-26, after `8e913aa` + the company-report fix):

    for every `command:` line:   <own deaths> + sum(company strengths) == roster

Second check (same run, added with the Regroup fix): a camp's `score` field must equal the
OTHER camp's death toll -- the commander's Regroup branch reads the score, so crossed wires
there would silently feed it its own losses (which is exactly the old defect).

Why it matters: "own deaths" (`round_kills_*`, logged as the `zhen wang` field) and the
company strengths (`CompanyReport.strength`) are two numbers the AI commander -- and the
LLM command channel -- reads.  When they disagree with the roster the battle picture is
simply wrong, and that is exactly how two 2026-09-26 defects stayed hidden:

  * the tail 20 men were missing from the company rosters (128 reported as 108);
  * the death tally only counted one of the two death paths (35 reported, 68 real).

Usage:
    python tools/battle_tally_check.py logs/llmbattle.log.err

Exit codes (a scanner needs a third outcome -- "did not run"):
    0 = scanned, no mismatch
    1 = mismatch found
    2 = did not run (no such file / no roster line / no command line)
"""
import io
import re
import sys

# Chinese markers are built from code points so this file stays ASCII.
DEAD = "\u9635\u4ea1"    # zhen wang -- own deaths of that camp (log field)
SCORE = "\u6218\u679c"   # zhan guo  -- that camp's score == the OTHER camp's deaths
STR = "\u5f3a\u5ea6"     # qiang du  -- company strength
ROSTER = re.compile(
    "\u7ea2\u8425\\s*(\\d+)\\s*\u4eba\\s*/\\s*\u84dd\u8425\\s*(\\d+)\\s*\u4eba"
)
CMD = "command: "
SQUAD_MARK = "\u8fde0["  # lian 0 [ -- first company of a camp block


def main(argv):
    if len(argv) < 2:
        print("usage: python tools/battle_tally_check.py <game .log.err>")
        return 2
    path = argv[1]
    try:
        text = io.open(path, encoding="utf-8", errors="replace").read()
    except OSError as exc:
        print("did not run: cannot read %s (%s)" % (path, exc))
        return 2

    rosters = None
    checked = 0
    bad = []
    cross = []
    scores = {}
    deaths = {}
    for line in text.split("\n"):
        if "command:" not in line:
            continue
        m = ROSTER.search(line)
        if m:
            # A round reset re-announces the rosters (and resets the death tallies).
            rosters = (int(m.group(1)), int(m.group(2)))
            continue
        if SQUAD_MARK not in line or rosters is None:
            continue
        parts = line.split("|")
        if len(parts) < 2:
            continue
        for seg, roster, tag in ((parts[0], rosters[0], "RED"), (parts[1], rosters[1], "BLUE")):
            k = re.search(DEAD + "(\\d+)", seg)
            sc = re.search(SCORE + "(\\d+)", seg)
            strengths = [int(x) for x in re.findall(STR + "(\\d+)", seg)]
            if not k or not strengths:
                continue
            checked += 1
            got = int(k.group(1)) + sum(strengths)
            if got != roster:
                stamp = line.split("INFO")[0].strip().strip("[]")
                bad.append((stamp, tag, roster, got, got - roster))
            if sc:
                scores[tag] = int(sc.group(1))
                deaths[tag] = int(k.group(1))
        # 交叉核对：本营战果必须等于**敌方阵亡**（重组判据读的就是战果）
        if "RED" in scores and "BLUE" in scores:
            if scores["RED"] != deaths["BLUE"] or scores["BLUE"] != deaths["RED"]:
                stamp = line.split("INFO")[0].strip().strip("[]")
                cross.append(
                    (stamp, scores["RED"], deaths["BLUE"], scores["BLUE"], deaths["RED"])
                )

    if rosters is None:
        print("did not run: no roster line (buildup) found in %s" % path)
        return 2
    if checked == 0:
        print("did not run: no command line with company strengths in %s" % path)
        return 2
    print("rosters: red=%d blue=%d; command lines checked: %d" % (rosters[0], rosters[1], checked))
    if bad:
        print("MISMATCH: %d camp-lines where own_deaths + strengths != roster" % len(bad))
        for stamp, tag, roster, got, delta in bad[:12]:
            print("  %s %-4s roster=%d got=%d (%+d)" % (stamp, tag, roster, got, delta))
        if len(bad) > 12:
            print("  ... and %d more" % (len(bad) - 12))
    if cross:
        print("CROSS-MISMATCH: %d lines where a camp's score != the other camp's deaths" % len(cross))
        for stamp, rs, bd, bs, rd in cross[:8]:
            print("  %s red_score=%d blue_deaths=%d | blue_score=%d red_deaths=%d" % (stamp, rs, bd, bs, rd))
    if bad or cross:
        return 1
    print("OK: every line satisfies own_deaths + sum(strengths) == roster")
    print("OK: every line satisfies camp_score == the other camp's deaths")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
