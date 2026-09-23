# install_git_hooks.ps1 - enable the in-repo commit guard (.githooks/), CHAINING any
# pre-existing hooks directory instead of replacing it.
#
# Why chaining: `core.hooksPath` can point at exactly one directory. The DSH Secret
# Gate lives in ~/.dsh/gates/hooks (its pre-push blocks pushing credentials to a public
# remote). Repointing hooksPath at .githooks would SILENTLY disable that gate -- the very
# gate that was added after the 2026-08-21 leak. So this script:
#   1) records the old hooksPath in `steelfront.baseHooksPath`;
#   2) points hooksPath at .githooks;
#   3) .githooks/pre-push forwards to the old pre-push explicitly.
#
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File scripts\install_git_hooks.ps1 [-Uninstall]
# NOTE: keep this file pure ASCII (PS 5.1 reads BOM-less .ps1 as ANSI; CJK inside string
#       literals breaks quote pairing -- see the repo lessons list).
param([switch]$Uninstall)
$ErrorActionPreference = "Continue"
$repo = (git rev-parse --show-toplevel)
Set-Location $repo
$base = (git config --get steelfront.baseHooksPath)
$cur = (git config --get core.hooksPath)

if ($Uninstall) {
    if ($base) { git config core.hooksPath $base; Write-Host "restored core.hooksPath = $base" }
    else { git config --unset core.hooksPath; Write-Host "core.hooksPath cleared (back to .git/hooks)" }
    git config --unset steelfront.baseHooksPath 2>$null
    exit 0
}

if ($cur -and $cur -ne ".githooks") {
    git config steelfront.baseHooksPath $cur
    Write-Host "chained base hooks = $cur"
} elseif ($base) {
    Write-Host "already chained to = $base"
} else {
    Write-Host "no previous hooksPath (nothing to chain)"
}

git config core.hooksPath .githooks
Write-Host "core.hooksPath = $(git config --get core.hooksPath)"

$hook = Join-Path $repo ".githooks/pre-commit"
if (-not (Test-Path $hook)) { Write-Host "FAIL: .githooks/pre-commit missing"; exit 1 }
Write-Host "hook present   = True"

$py = Get-Command python -ErrorAction SilentlyContinue
if (-not $py) { Write-Host "WARN: python not found -> hook will skip the scan" } else { Write-Host "python         = $($py.Source)" }

Write-Host "--- guard self-test (whole tree) ---"
python (Join-Path $repo "tools/commit_guard.py") --scan .
$rc = $LASTEXITCODE
Write-Host "guard scan exit = $rc"
exit 0
