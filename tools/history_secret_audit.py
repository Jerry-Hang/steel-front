# -*- coding: utf-8 -*-
"""history_secret_audit.py — 扫 git **历史**（不只是 HEAD）里有没有明文凭据

为什么不能只查 HEAD
------------------
`git grep HEAD` 干净 ≠ 干净过。密钥一旦进过任何一个被推送的 commit，它就永远留在这个
仓库的对象库里：改文件、`git rm`、甚至 force-push 都删不掉（除非重写历史 + GC）。
**判断"有没有泄漏到公网"的唯一判据是：含密钥的那条 commit 在不在已推送的分支历史里。**

判据口径
--------
    --exposed-in origin/master   （默认）：命中项会额外标注该 commit 是否已在该分支历史里。
    ⇒ 标 EXPOSED 的 = 已经推上去了，必须按"已泄漏"处理（轮换密钥 + 视需要重写历史）。

用法
----
    python tools/history_secret_audit.py                      # 全历史扫描
    python tools/history_secret_audit.py --exposed-in origin/master
    python tools/history_secret_audit.py --paths-only         # 只按文件名判（快）

⚠ 输出**永远脱敏**：只打印前 10 个字符与长度，绝不打印完整凭据（终端记录、日志、
  工单系统都会留存这些输出）。
"""

import argparse
import re
import subprocess
import sys

sys.stdout.reconfigure(encoding="utf-8", errors="replace")

# 与 tools/commit_guard.py 同源的格式表（两处任一改动都要同步：这是刻意的"双份"，
# 因为审计脚本要能独立跑，不依赖守卫的 CLI 形态）
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
PLACEHOLDER = re.compile(
    rb"<[^>]{0,60}>|YOUR_|_HERE|xxx|XXXX|example|placeholder|REDACTED|redact|"
    rb"changeme|dummy|fake|local|test|sample", re.I)

SUSPICIOUS_PATH = re.compile(
    rb"(?i)(^|/)(\.?env|.*key.*|.*token.*|.*secret.*|.*credential.*|\.netrc|\.npmrc|"
    rb"id_rsa.*|id_ed25519.*|\.ds_(req|resp)\.json|\.ds_out\.txt)$")

SKIP_EXT = {".glb", ".spv", ".png", ".jpg", ".jpeg", ".bmp", ".exe", ".dll", ".zip", ".ico", ".pdf"}
MAX_BLOB = 2 * 1024 * 1024


def git(*args, binary=False):
    r = subprocess.run(["git", *args], capture_output=True)
    if r.returncode != 0:
        return b"" if binary else ""
    return r.stdout if binary else r.stdout.decode("utf-8", "replace")


def all_blobs():
    """(blob_sha, first_seen_path) —— 用 rev-list --objects 枚举所有可达对象。"""
    out = git("rev-list", "--objects", "--all")
    for line in out.splitlines():
        parts = line.split(" ", 1)
        if len(parts) == 2:
            yield parts[0], parts[1]


def blob_bytes(sha):
    r = subprocess.run(["git", "cat-file", "blob", sha], capture_output=True)
    return r.stdout if r.returncode == 0 else b""


def scan(blob):
    hits = []
    if len(blob) > MAX_BLOB:
        return hits
    for kind, pat in PATTERNS:
        for m in pat.finditer(blob):
            ls = blob.rfind(b"\n", 0, m.start()) + 1
            le = blob.find(b"\n", m.end())
            line = blob[ls: le if le != -1 else len(blob)]
            if PLACEHOLDER.search(line):
                continue
            tok = m.group(0)
            hits.append((kind, tok[:10].decode("latin1"), len(tok)))
    return hits


def committing_commit(path):
    """第一个加入该路径的 commit（--diff-filter=A 反向找最后一个 A 也行，这里取最早）。"""
    out = git("log", "--all", "--reverse", "--oneline", "--diff-filter=A", "--format=%h %ad %s", "--date=short", "--", path)
    return out.splitlines()[0] if out.strip() else "?"


def exposed_in(ref, commit_hash):
    r = subprocess.run(["git", "merge-base", "--is-ancestor", commit_hash, ref], capture_output=True)
    return r.returncode == 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--exposed-in", default="origin/master")
    ap.add_argument("--paths-only", action="store_true", help="只按文件名判（不看内容，快）")
    args = ap.parse_args()

    seen = set()
    findings = []
    scanned = 0
    for sha, path in all_blobs():
        if (sha, path) in seen:
            continue
        seen.add((sha, path))
        ext = ("." + path.rsplit(".", 1)[-1].lower()) if "." in path else ""
        if ext in SKIP_EXT:
            continue
        suspicious = bool(SUSPICIOUS_PATH.search(path.encode("utf-8", "replace")))
        if args.paths_only:
            if suspicious:
                findings.append((path, sha, "suspicious-path", "按文件名", 0))
            continue
        blob = blob_bytes(sha)
        if not blob:
            continue
        scanned += 1
        for kind, head, n in scan(blob):
            findings.append((path, sha, kind, head, n))
        if suspicious and not any(f[0] == path for f in findings):
            findings.append((path, sha, "suspicious-path(no-match)", "-", 0))

    print(f"history_secret_audit: 扫描 {scanned} 个 blob（{len(seen)} 个 路径×版本）")
    if not findings:
        print("结论：历史里没有明文凭据命中")
        return 0

    print(f"命中 {len(findings)} 条：\n")
    exposed_any = False
    for path, sha, kind, head, n in findings:
        cc = committing_commit(path)
        ch = cc.split(" ")[0] if cc != "?" else "?"
        exp = exposed_in(args.exposed_in, ch) if ch != "?" else False
        exposed_any |= exp
        tag = f"🔴 EXPOSED({args.exposed_in})" if exp else "（未在该分支历史里）"
        print(f"  [{kind}] {path}")
        print(f"      blob={sha[:10]}  引入提交={cc}")
        print(f"      {tag}   预览={head}…(len={n})")
    print()
    if exposed_any:
        print("⇒ 至少有一条命中已经进入已推送分支的历史：**按已泄漏处理**（轮换该凭据；")
        print("   如需清除，得重写历史 + 强推 + 通知协作者，且假设旧历史已被人抓取过）。")
    return 1


if __name__ == "__main__":
    sys.exit(main())
