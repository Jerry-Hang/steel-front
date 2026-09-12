# =============================================================================
#  package_release.ps1 -- build a distributable Steam Front folder + zip
# =============================================================================
#  ASCII ONLY. Windows PowerShell 5.1 reads a BOM-less .ps1 as ANSI; non-ASCII
#  bytes inside string literals break quote pairing and silently swallow lines
#  (this repo has paid for that twice -- see docs/PROGRESS.md 2026-09-12).
#
#  Usage:
#     powershell -NoProfile -ExecutionPolicy Bypass -File scripts\package_release.ps1
#     powershell ... -File scripts\package_release.ps1 -SkipBuild -Tag rc1
#
#  Output:
#     dist\steel-front-<tag>\        (runnable folder)
#     dist\steel-front-<tag>.zip     (distributable)
#
#  What goes in:  the release exe, assets\ (shaders + maps + props, all read
#  from disk at RUNTIME), README.md, LICENSE.
#  What stays out: target\, logs\, screenshots\, data\, tools\, src\, docs\,
#  and every *.spv under assets is INCLUDED (the engine loads them at runtime).
# =============================================================================

[CmdletBinding()]
param(
    [string]$Tag = (Get-Date -Format 'yyyyMMdd-HHmm'),
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

if (-not $SkipBuild) {
    Write-Host '[release] cargo build --release'
    & cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" }
}

$exe = Join-Path $root 'target\release\steel-front.exe'
if (-not (Test-Path $exe)) { throw "missing $exe" }

$outName = "steel-front-$Tag"
$dist    = Join-Path $root "dist\$outName"
if (Test-Path $dist) { Remove-Item $dist -Recurse -Force }
New-Item -ItemType Directory -Path $dist -Force | Out-Null

Copy-Item $exe $dist
foreach ($f in @('README.md', 'LICENSE')) {
    $p = Join-Path $root $f
    if (Test-Path $p) { Copy-Item $p $dist }
}

# assets\ is loaded from disk at runtime (shaders, maps, GLB props).
$assets = Join-Path $root 'assets'
if (-not (Test-Path $assets)) { throw 'missing assets\' }
Copy-Item $assets (Join-Path $dist 'assets') -Recurse

# --- sanity checks: a package that cannot start is worse than no package -----
$need = @(
    'steel-front.exe',
    'assets\mesh.spv',
    'assets\triangle.vert.spv',
    'assets\triangle.frag.spv',
    'assets\maps\index.toml'
)
$missing = @($need | Where-Object { -not (Test-Path (Join-Path $dist $_)) })
if ($missing.Count -gt 0) {
    throw ('package is missing required files: ' + ($missing -join ', '))
}

$spv  = @(Get-ChildItem (Join-Path $dist 'assets') -Filter *.spv -Recurse)
$maps = @(Get-ChildItem (Join-Path $dist 'assets\maps') -Filter *.toml -ErrorAction SilentlyContinue)
$glb  = @(Get-ChildItem (Join-Path $dist 'assets\props') -Filter *.glb -Recurse -ErrorAction SilentlyContinue)

$zip = Join-Path $root "dist\$outName.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path (Join-Path $dist '*') -DestinationPath $zip -CompressionLevel Optimal

$size = (Get-Item $zip).Length / 1MB
Write-Host ''
Write-Host "=== PACKAGE OK: $outName ==="
Write-Host ("  folder : {0}" -f $dist)
Write-Host ("  zip    : {0}  ({1:N1} MB)" -f $zip, $size)
Write-Host ("  shaders: {0} spv" -f $spv.Count)
Write-Host ("  maps   : {0} toml" -f $maps.Count)
Write-Host ("  props  : {0} glb" -f $glb.Count)
Write-Host ''
Write-Host 'Next: copy the zip anywhere, extract, run steel-front.exe.'
Write-Host 'First run writes %USERPROFILE%\.steel_front.cfg (resolution defaults to'
Write-Host 'the primary monitor aspect ratio on first use only).'
