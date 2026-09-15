"""Split a per-file git diff into two patches by hunk keyword.

One-off helper used while landing the 2026-09-15 round: two independent changes
(a gun-buffer device-lost fix and the bullet-decal feature) touched the same two
files, and the repo rule is one feature per commit.  Splitting by hunk keeps the
history honest without hand-editing patches.

Usage:  python scripts/split_patch.py <out_a.patch> <out_b.patch> -- <file...>
Hunks containing any keyword from GROUP A go to the first patch, everything else
to the second.  ASCII only on purpose (this shell mangles non-ASCII literals).
"""
import subprocess
import sys

A_KEYS = (
    "gun_buffer_capacity",
    "gun_glb_indices_all_in_range",
    "load_gun_glb",
    "device_wait_idle",
    "need_verts",
    "need_idx",
)


def main() -> int:
    argv = sys.argv[1:]
    if "--" not in argv:
        print("usage: split_patch.py out_a out_b -- file...")
        return 2
    cut = argv.index("--")
    out_a, out_b = argv[0], argv[1]
    files = argv[cut + 1:]

    # git 输出是 UTF-8，而本机 Python 的默认 locales 是 GBK —— 必须显式解码，
    # 否则中文注释直接抛 UnicodeDecodeError（2026-09-15 实测）。
    diff = subprocess.run(
        ["git", "diff", "--"] + files, capture_output=True, check=True
    ).stdout.decode("utf-8")

    a_parts, b_parts = [], []
    file_header = []      # diff --git 行
    full_header = None    # diff --git + index + --- + +++（每个 hunk 都要带）
    pending_header = []
    cur_hunk = []
    is_a = False
    stats = {"a": 0, "b": 0}

    def flush():
        nonlocal cur_hunk
        if cur_hunk:
            (a_parts if is_a else b_parts).append("".join(cur_hunk))
            stats["a" if is_a else "b"] += 1
            cur_hunk = []

    for line in diff.splitlines(keepends=True):
        if line.startswith("diff --git"):
            flush()
            file_header = [line]
            full_header = None
            pending_header = []
        elif line.startswith("@@"):
            flush()
            if full_header is None:
                # 第一次遇到本文件的 hunk：把 index/---/+++ 一起记下来，
                # **后续 hunk 必须复用同一份头**（git apply 要求每个 hunk 都带文件头）
                full_header = file_header + pending_header
            cur_hunk = full_header + [line]
            is_a = False
        elif cur_hunk:
            cur_hunk.append(line)
            if line.startswith("+") and any(k in line for k in A_KEYS):
                is_a = True
        else:
            pending_header.append(line)
    flush()

    with open(out_a, "w", encoding="utf-8", newline="") as fh:
        fh.write("".join(a_parts))
    with open(out_b, "w", encoding="utf-8", newline="") as fh:
        fh.write("".join(b_parts))
    print(f"patch A hunks: {stats['a']}  patch B hunks: {stats['b']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
