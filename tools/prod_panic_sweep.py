"""Report unwrap()/expect()/panic!/unreachable! that live in PRODUCTION code.

Test bodies are excluded by cutting the file at the `#[cfg(test)]` module (this repo puts tests
at the bottom of each file). Without that cut the counts are meaningless -- most matches are
assertions in tests, which is exactly why an "N unwraps" number by itself has never found
anything here.

Usage: python tools/prod_panic_sweep.py [file ...]      (default: a fixed list of src files)
"""
import io
import re
import sys

DEFAULT = [
    "src/main.rs",
    "src/engine/game.rs",
    "src/engine/ai.rs",
    "src/engine/city.rs",
    "src/engine/map.rs",
    "src/engine/objective.rs",
    "src/engine/physics.rs",
    "src/engine/props.rs",
    "src/engine/assets.rs",
    "src/ui.rs",
    "src/audio.rs",
    "src/audio_out.rs",
    "src/config.rs",
    "src/camera.rs",
    "src/perf_log.rs",
    "src/llm_cmd.rs",
]

PAT = re.compile(r"\.unwrap\(\)|\.expect\(|panic!\(|unreachable!\(")


def main(paths):
    total = 0
    for path in paths:
        try:
            text = io.open(path, encoding="utf-8").read()
        except OSError:
            continue
        cut = text.find("#[cfg(test)]")
        prod = text if cut < 0 else text[:cut]
        hits = []
        for i, line in enumerate(prod.split("\n"), 1):
            stripped = line.strip()
            if stripped.startswith("//") or stripped.startswith("///"):
                continue
            if PAT.search(line):
                hits.append((i, stripped))
        if hits:
            print("%s: %d production site(s)" % (path, len(hits)))
            for i, line in hits:
                print("  L%-5d %s" % (i, line[:150]))
            total += len(hits)
    print("total production sites: %d" % total)


if __name__ == "__main__":
    main(sys.argv[1:] or DEFAULT)
