# -*- coding: utf-8 -*-
"""commit_guard.py — 提交白名单 + 明文密钥守卫（Steel Front）

为什么是白名单，而不是黑名单
--------------------------
黑名单永远漏：新的密钥格式、新的文件名、新的临时目录都会绕过它。
白名单把"没见过的东西"默认挡在门外，要放行必须显式改本文件 —— **改动本身就是留痕**。

两种模式
--------
    python tools/commit_guard.py --staged          # pre-commit 钩子：只查暂存区（推荐常开）
    python tools/commit_guard.py --scan [路径...]  # 手动巡检：默认全仓，跳过重目录

三条规则（任一不过即 exit 1，并打印可执行的下一步）
--------------------------------------------------
    R1 路径白名单：路径必须在 ALLOW_EXT / ALLOW_NAMES / ALLOW_PREFIX 内。
    R2 路径拒绝表：命中 DENY_GLOBS 的路径直接拒绝（即使扩展名在白名单里）。
       —— 优先级高于 R1：`.env` 不是靠"扩展名没见过"挡住的，是靠这条明确挡住的。
    R3 内容扫描：按已知密钥格式扫明文。占位符（<...> / YOUR_ / xxx / example / REDACTED）
       不算命中 —— 否则文档里写 `api_key="<Your Key>"` 会被误报，而误报会训练人忽略告警。

旁路（仅限确有理由的一次性场景）
--------------------------------
    COMMIT_GUARD_BYPASS="理由"   git commit ...
空理由 / 未设置 = 不放行。旁路会打印醒目警告，**理由会留在你的 shell 历史里**。

自我豁免
--------
本文件自己保存着密钥正则（否则没法扫），所以 R3 对自己豁免（见 SELF_EXEMPT）。
"""

import os
import re
import subprocess
import sys

sys.stdout.reconfigure(encoding="utf-8", errors="replace")

# --- 白名单：允许入库的扩展名 -------------------------------------------------
ALLOW_EXT = {
    "rs", "toml", "lock", "md", "txt", "ps1", "py", "sh", "bat", "cmd", "json",
    "yml", "yaml", "glsl", "wgsl", "spv", "glb", "png", "jpg", "jpeg", "svg",
    "ico", "cs", "c", "h", "cpp", "hpp", "gitignore", "gitattributes",
    "editorconfig", "csv",
    # `log` 在白名单里，是因为 docs/ 下有**刻意入库的性能证据日志**（perf-*/**.log）；
    # 运行时日志靠拒绝表的 `logs/*` + `.gitignore` 的 `*.log` 挡，不靠扩展名。
    "log",
}
# --- 白名单：无扩展名但必须入库的固定文件名 -----------------------------------
ALLOW_NAMES = {"LICENSE", ".gitignore", ".gitattributes", ".editorconfig", "Makefile"}
# --- 白名单：目录前缀（钩子脚本无扩展名） -------------------------------------
ALLOW_PREFIX = (".githooks/",)

# --- 拒绝表：无论扩展名如何，这些路径一律不进仓库 -----------------------------
DENY_GLOBS = (
    "*.key", "*.pem", "*.pfx", "*.p12", "*.jks", "*.keystore",
    ".env", ".env.*", "*.env",
    "*secret*", "*credential*", "*password*", "*passwd*", "*apikey*",
    "id_rsa*", "id_ed25519*", ".netrc", ".npmrc", ".pypirc",
    "*.cfg", ".ds_req.json", ".ds_resp.json", ".ds_out.txt",
    "data/*", "logs/*", "screenshots/*", "dist/*", "target/*", "build/*",
)

# --- 内容扫描：已知密钥格式 + 赋值式（值必须够长，避免把普通词判成密钥） -------
PATTERNS = (
    ("openai/deepseek-sk", re.compile(rb"sk-[A-Za-z0-9_\-]{20,}")),
    ("github-pat", re.compile(rb"ghp_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}")),
    ("aws-akia", re.compile(rb"AKIA[0-9A-Z]{16}")),
    ("bearer", re.compile(rb"[Bb]earer\s+[A-Za-z0-9_\-\.]{24,}")),
    ("jwt", re.compile(rb"eyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}")),
    ("assigned", re.compile(
        rb"(?:api[_-]?key|apikey|access[_-]?token|auth[_-]?token|secret[_-]?key|password)"
        rb"\s*[:=]\s*[\"']?([A-Za-z0-9_\-\.]{20,})[\"']?", re.I)),
)
# 占位符/示例值：命中这些词的整行按"不是真密钥"处理
PLACEHOLDER = re.compile(
    rb"<[^>]{0,60}>|YOUR_|_HERE|xxx|XXXX|example|placeholder|REDACTED|redact|"
    rb"changeme|dummy|fake|local|test|sample", re.I)

# 内含密钥正则、必须豁免 R3 的文件
SELF_EXEMPT = {"tools/commit_guard.py", "tools/history_secret_audit.py"}

# 守卫系统自身的文件：文件名里带 secret/key 之类的词，会被下面的拒绝表误伤。
# **必须在拒绝表之前放行**，否则"守卫挡守卫"（实测 2026-09-22：history_secret_audit.py
# 被 `*secret*` 拦下）。这份清单是显式白名单，加文件要改代码 —— 即留痕。
ALLOW_PATHS = {
    "tools/commit_guard.py",
    "tools/history_secret_audit.py",
    ".githooks/pre-commit",
    ".githooks/pre-push",
}

SKIP_DIRS = {".git", "target", "node_modules", "dist", "__pycache__", "build", "logs", "screenshots"}
MAX_BYTES = 4 * 1024 * 1024


def deny_reason(path):
    """返回命中拒绝表的模式名，未命中返回 None。用 fnmatch 语义（与 .gitignore 一致）。"""
    import fnmatch
    p = path.replace("\\", "/")
    for g in DENY_GLOBS:
        if fnmatch.fnmatch(p, g) or fnmatch.fnmatch(os.path.basename(p), g):
            return g
    return None


def allow_reason(path):
    """返回 (是否放行, 依据)。"""
    p = path.replace("\\", "/")
    base = os.path.basename(p)
    if p in ALLOW_PATHS:
        return True, "guard-self"
    if p in SELF_EXEMPT:
        return True, "self-exempt"
    if base in ALLOW_NAMES:
        return True, "name"
    if any(p.startswith(pre) for pre in ALLOW_PREFIX):
        return True, "prefix"
    ext = base.rsplit(".", 1)[-1].lower() if "." in base else ""
    if ext in ALLOW_EXT:
        return True, "ext:" + ext
    return False, "未在白名单内"


def scan_bytes(path, blob):
    """扫一段字节，返回 [(kind, 脱敏预览)]。"""
    if path.replace("\\", "/") in SELF_EXEMPT:
        return []
    out = []
    for kind, pat in PATTERNS:
        for m in pat.finditer(blob):
            line_start = blob.rfind(b"\n", 0, m.start()) + 1
            line_end = blob.find(b"\n", m.end())
            line = blob[line_start: line_end if line_end != -1 else len(blob)]
            if PLACEHOLDER.search(line):
                continue
            tok = m.group(0)
            head = tok[:10].decode("latin1")
            out.append((kind, f"{head}…(len={len(tok)})"))
    return out


def staged_files():
    r = subprocess.run(["git", "diff", "--cached", "--name-only", "--diff-filter=ACMR"],
                       capture_output=True, text=True, encoding="utf-8", errors="replace")
    return [l for l in r.stdout.splitlines() if l.strip()]


def staged_blob(path):
    r = subprocess.run(["git", "show", f":{path}"], capture_output=True)
    return r.stdout if r.returncode == 0 else b""


def walk_files(roots):
    for root in roots:
        if os.path.isfile(root):
            yield root
            continue
        for dirpath, dirnames, filenames in os.walk(root):
            dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
            for fn in filenames:
                yield os.path.join(dirpath, fn)


def ignored_set(paths):
    """批量问 git 哪些路径已被 .gitignore 覆盖。

    巡检模式必须用它：`data/`、`logs/`、`screenshots/` 这些**本来就进不了仓库**的目录
    如果照扫，输出会被几十条正常噪声淹没 —— 而"会喊狼来了的守卫"等于没有守卫（教训 26）。
    只有 `--staged` 才是真正的门（那里 git 已经把忽略项过滤掉了）。

    🔴 两个踩过的坑（都会**静默**失效，别改回去）：
      1. **不要用 `text=True` 喂 stdin**：Windows 上 Python 会把 `\\n` 翻成 `\\r\\n`，
         于是 git 收到的每个路径都带 CR，它会把路径 C 引用化（输出 `"a/b\\r"`），
         集合比对全部不命中 ⇒ 过滤看着在跑、其实一个都没滤掉（实测 2026-09-22）。
         改用**字节 + `-z`（NUL 分隔、不引用）**，两端都不做换行翻译。
      2. **不要用 `lstrip("./")` 去前缀**：lstrip 收的是**字符集**，
         `".gitignore"` 会被削成 `"gitignore"`（前导点被吃掉）。
         只许 `startswith("./")` 后再切片。
    """
    if not paths:
        return set()
    r = subprocess.run(["git", "check-ignore", "--stdin", "-z"],
                       input=b"\0".join(p.encode("utf-8") for p in paths),
                       capture_output=True)
    return {p.replace("\\", "/") for p in r.stdout.decode("utf-8", "replace").split("\0") if p}


def norm_path(path):
    """统一成仓库相对、正斜杠的路径（只剥 `./` 前缀，不碰其它字符）。"""
    p = path.replace("\\", "/")
    while p.startswith("./"):
        p = p[2:]
    return p


def tracked_set():
    """**已提交**（在 HEAD 里）的路径。

    🔴 必须用 `ls-tree HEAD`，不能用 `ls-files`：后者把**暂存区**也算进来，
    于是「刚 `git add` 进来的 .env」会被误判成"已入库的老文件"而降级成警告 ⇒
    守卫对"新加的敏感文件"完全失效（实测 2026-09-22，一次就踩到）。
    """
    r = subprocess.run(["git", "ls-tree", "-r", "--name-only", "HEAD"],
                       capture_output=True, text=True, encoding="utf-8", errors="replace")
    return {norm_path(l) for l in r.stdout.splitlines() if l.strip()}



def report(findings, checked, mode, warned=()):
    for p, d in warned:
        print(f"commit-guard[{mode}]: WARN 已入库路径命中拒绝表（历史决定，不拦）：{p}  ({d})")
    if not findings:
        print(f"commit-guard[{mode}]: OK — {checked} 个文件通过白名单与密钥扫描")
        return 0
    print(f"commit-guard[{mode}]: 拒绝 —— {len(findings)} 处问题（已检查 {checked} 个文件）")
    for kind, path, detail in findings:
        print(f"  [{kind}] {path}: {detail}")
    print("\n下一步：")
    print("  路径被拒 → 移出仓库目录，或（确认无密）在 .gitignore 里加规则；")
    print("  内容被拒 → 改成从环境变量读取，别把明文写进文件；")
    print("  白名单本身要扩 → 显式改 tools/commit_guard.py 的 ALLOW_*（改动即留痕）。")
    bypass = os.environ.get("COMMIT_GUARD_BYPASS", "").strip()
    if bypass:
        print(f"\n!! COMMIT_GUARD_BYPASS 生效，本次放行。理由：{bypass}")
        return 0
    return 1


def main(argv):
    mode = "staged" if "--staged" in argv else "scan"
    findings = []
    checked = 0

    if mode == "staged":
        files = staged_files()
        ignored = set()
    else:
        roots = [a for a in argv[1:] if not a.startswith("--")] or ["."]
        files = list(walk_files(roots))
        ignored = ignored_set([norm_path(f) for f in files])
    tracked = tracked_set()
    warned = []

    for path in files:
        p = norm_path(path)
        if p in ignored:
            continue
        # 守卫自身的文件先于拒绝表放行（否则 `*secret*` 会把守卫自己拦下）
        if p in ALLOW_PATHS:
            checked += 1
            continue
        d = deny_reason(p)
        if d:
            # 已入库的老文件（例如 data/ 下 3 个被代码引用的 jsonl）：路径拒绝表只对新文件
            # 生效 —— 否则守卫会挡住这个仓库自己的历史决定。**内容扫描仍然是硬门**。
            if p in tracked:
                warned.append((p, d))
            else:
                findings.append(("DENY", p, f"命中拒绝表 {d}"))
            continue
        if d:
            findings.append(("DENY", p, f"命中拒绝表 {d}"))
            continue
        ok, why = allow_reason(p)
        if not ok:
            findings.append(("NOT-ALLOWED", p, why))
            continue
        if mode == "staged":
            blob = staged_blob(p)
        else:
            try:
                if os.path.getsize(p) > MAX_BYTES:
                    continue
                blob = open(p, "rb").read()
            except OSError:
                continue
        checked += 1
        for kind, detail in scan_bytes(p, blob):
            findings.append((f"SECRET:{kind}", p, detail))

    return report(findings, checked, mode, warned)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
