"""Static audit helper: every Vulkan *handle* struct field should be released somewhere.

Why this exists (2026-09-26)
---------------------------
A whole class of defects in this repo is "a handle was created, but nothing ever destroys it"
-- or "it is destroyed on one path but not on the other". Both are silent: the driver does not
complain, the game keeps running, and it only shows up as growing VRAM in a long session (or
as a VUID when the validation layer sees a double destroy). Reading 14k lines of renderer.rs to
answer "is every handle released?" is a bad use of attention, so this script does the boring
part and prints a shortlist to review.

It found (and this file's sibling commit fixed) a real one on its first useful run: the NPC
sphere/cylinder geometry buffers created by `create_sphere_geometry` /
`create_cylinder_geometry` were in no release table at all.

How it works
------------
1. Walks each .rs file tracking brace depth, and only treats a line as a *struct field* when it
   is directly inside a `struct X { ... }` body. (The first version skipped that step and
   matched struct-literal arguments all over the file: 85 "suspects", nearly all noise --
   lesson 27: make sure the ruler measures what you think it measures.)
2. Keeps fields whose type is a Vulkan **handle** (Buffer/Image/ImageView/DeviceMemory/...).
   Plain value structs (`vk::Extent2D`, `vk::Format`, `vk::SampleCountFlags`,
   `vk::PhysicalDeviceProperties`) are not resources and are skipped.
3. Searches the WHOLE `src/` tree (not just the declaring file) for the field name within a
   few dozen lines of a `destroy_*` / `free_*` call -- this repo's cleanup is table driven and
   sometimes lives in another file (e.g. `PtAssets` fields are freed by
   `Renderer::pt_destroy_assets`).

This is a HEURISTIC, not a proof: descriptor sets and command buffers are freed with their pool,
Vec-owned handles are released through the Vec, and a handle can be handed to another owner.
Verify every hit in the source before acting on it.

Usage:
    python tools/audit_vk_resources.py
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "src"

LIFETIME_ALLOWLIST = {
    "device",
    "instance",
    "entry",
    "surface",
    "surface_loader",
    "physical_device",
    "graphics_queue",
    "present_queue",
    "debug_messenger",
    "debug_utils",
}

# Types that are handles (i.e. need an explicit destroy/free) -- everything else is a value.
HANDLE_TYPES = (
    "Buffer",
    "Image",
    "ImageView",
    "DeviceMemory",
    "Sampler",
    "Pipeline",
    "PipelineLayout",
    "RenderPass",
    "Framebuffer",
    "DescriptorSetLayout",
    "DescriptorPool",
    "CommandPool",
    "Semaphore",
    "Fence",
    "Event",
    "QueryPool",
    "SwapchainKHR",
    "AccelerationStructureKHR",
    "BufferView",
    "ShaderModule",
)

# Freed by their pool / by the driver, not by an explicit call.
POOL_OWNED = ("DescriptorSet", "CommandBuffer")

FIELD_RE = re.compile(r"^\s*(?:pub\s+)?([a-z_][a-z0-9_]*)\s*:\s*([A-Za-z0-9_:<>, \[\]]+?),?\s*$")
STRUCT_OPEN_RE = re.compile(r"^\s*(?:pub\s+)?struct\s+([A-Za-z0-9_]+)(?:<[^>]*>)?\s*\{")
WINDOW = 25


def is_handle(ty: str) -> bool:
    if any(p in ty for p in POOL_OWNED):
        return False
    return any(re.search(r"\b" + t + r"\b", ty) for t in HANDLE_TYPES)


def struct_fields(text):
    """Field name -> declared type, for fields *directly* inside a struct body."""
    fields = {}
    depth = 0
    struct_depth = None
    for line in text.splitlines():
        stripped = line.split("//", 1)[0]
        if struct_depth is None:
            m = STRUCT_OPEN_RE.match(stripped)
            if m:
                struct_depth = depth + 1
                depth += stripped.count("{") - stripped.count("}")
                continue
        else:
            if depth == struct_depth:
                m = FIELD_RE.match(stripped)
                if m and "vk::" in m.group(2) and is_handle(m.group(2)):
                    fields[m.group(1)] = m.group(2).strip()
        depth += stripped.count("{") - stripped.count("}")
        if struct_depth is not None and depth < struct_depth:
            struct_depth = None
    return fields


def main():
    sources = {}
    hot_lines = {}
    for path in sorted(SRC.rglob("*.rs")):
        text = path.read_text(encoding="utf-8")
        lines = text.splitlines()
        sources[path] = lines
        hot_lines[path] = [
            i
            for i, line in enumerate(lines)
            if ("destroy_" in line or "free_memory" in line or "free_descriptor" in line)
        ]

    total = 0
    suspects = []
    for path, lines in sources.items():
        fields = {k: v for k, v in struct_fields("\n".join(lines)).items() if k not in LIFETIME_ALLOWLIST}
        if not fields:
            continue
        total += len(fields)
        for name, ty in sorted(fields.items()):
            pat = re.compile(r"\b" + re.escape(name) + r"\b")
            released = False
            for other, other_lines in sources.items():
                hot = hot_lines[other]
                if not hot:
                    continue
                for i, line in enumerate(other_lines):
                    if not pat.search(line):
                        continue
                    if any(abs(i - h) <= WINDOW for h in hot):
                        released = True
                        break
                if released:
                    break
            if not released:
                suspects.append((str(path.relative_to(ROOT)), name, ty))

    print("vk:: handle fields scanned : %d" % total)
    print("no release call found     : %d" % len(suspects))
    for path, name, ty in suspects:
        print("  %-28s %-30s %s" % (path, name, ty))
    print()
    print("NOTE: heuristic (pools / Vecs / ownership transfer can legitimately account for a")
    print("      hit). Read the source before treating a line as a leak.")
    # 🔴 2026-09-26：「扫了 0 个字段」**不算通过** —— 在错误的目录下跑（SRC 为空）
    # 会打印 `scanned 0 / no release 0` 然后 exit 0，看起来像"一片干净"。
    # 判据：0 = 真扫过；**2 = 根本没扫成**（同 `history_secret_audit.py` 的约定）。
    if total == 0:
        print("结论：**一个 vk:: 句柄字段都没扫到** —— 这不是通过，是扫描没跑起来（检查 cwd / src 路径）。",
              file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
