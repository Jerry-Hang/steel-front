"""Compare the first-person gun region across a set of screenshots.

WHY: the user reported that switching weapons (mouse wheel / number keys) does not
change the gun MODEL. The game log shows the GLB *is* re-imported and the world AABB
changes, so the question is purely visual: do the pixels in the gun region differ
between two switch methods?

Reading seven 2560x1600 PNGs by eye is slow and easy to fool (the HUD weapon NAME
always changes, which is not the question). This reduces it to a number.

The gun sits in the lower-right of the frame in this build; the region below is where
it is drawn. It deliberately EXCLUDES the HUD strips (bottom bar, top-left perf text,
top-right minimap) so a HUD change alone cannot make two shots look different.

Usage:
  python tools/diff_gun_region.py screenshots/wpn_0_start.png screenshots/wpn_3_key3.png ...
"""

import sys

import numpy as np
from PIL import Image

# Lower-right quadrant, inset from the HUD: the gun lives here in every build so far.
X0_FRAC, X1_FRAC = 0.55, 1.00
Y0_FRAC, Y1_FRAC = 0.55, 0.93


def gun_region(path):
    im = Image.open(path).convert("RGB")
    w, h = im.size
    box = (int(w * X0_FRAC), int(h * Y0_FRAC), int(w * X1_FRAC), int(h * Y1_FRAC))
    a = np.asarray(im.crop(box), dtype=np.int16)
    return a, im.size


def main():
    paths = sys.argv[1:]
    if len(paths) < 2:
        print("usage: diff_gun_region.py <png> <png> [png...]")
        return 1

    ref, size = gun_region(paths[0])
    print(f"region = x {X0_FRAC:.2f}..{X1_FRAC:.2f}, y {Y0_FRAC:.2f}..{Y1_FRAC:.2f}  of {size[0]}x{size[1]}")
    print(f"{'file':38s} {'mean|d|':>9s} {'%pixels>12':>11s}  verdict")
    print(f"{paths[0].split(chr(92))[-1].split('/')[-1]:38s} {'-':>9s} {'-':>11s}  (reference)")

    for p in paths[1:]:
        a, _ = gun_region(p)
        if a.shape != ref.shape:
            print(f"{p:38s} size mismatch")
            continue
        d = np.abs(a - ref).mean(axis=2)
        mean = float(d.mean())
        frac = float((d > 12).mean() * 100.0)
        name = p.split("\\")[-1].split("/")[-1]
        # The gun occupies a few percent of this region; a real model swap moves well
        # over 1% of its pixels by more than one JND.
        verdict = "DIFFERENT" if frac > 1.0 else ("same" if frac < 0.15 else "marginal")
        print(f"{name:38s} {mean:9.2f} {frac:11.2f}  {verdict}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
