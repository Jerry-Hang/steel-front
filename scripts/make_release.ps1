# =============================================================================
#  make_release.ps1 -- build the Windows portable folder used for releases
# =============================================================================
#  ASCII ONLY. Windows PowerShell 5.1 reads a BOM-less .ps1 as ANSI; non-ASCII
#  bytes inside string literals break quote pairing and silently swallow lines.
#
#  Usage:
#     powershell -NoProfile -ExecutionPolicy Bypass -File scripts\make_release.ps1
#     ... -SkipBuild            reuse target\release\*.exe
#     ... -Version v0.2.0       override the version printed in README/zip name
#
#  Output layout (matches the launcher's expectations and the v0.1.0 package):
#     release_dist\SteelFrontLauncher.exe
#     release_dist\README.txt  README.md  LICENSE
#     release_dist\game\steel-front.exe  rdv.exe  assets\  logs\  联机主机.bat  联机加入.bat
#
#  The zip itself is produced by scripts\package_windows_zip.ps1 (root entries
#  must stay exactly like this folder: the launcher's updater looks for game\).
# =============================================================================
[CmdletBinding()]
param(
    [string]$Version = '',
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

if (-not $Version) {
    $verLine = Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version\s*=\s*"([0-9][0-9.]*)"' | Select-Object -First 1
    $Version = if ($verLine -and $verLine.Line -match '"([0-9][0-9.]*)"') { 'v' + $Matches[1] } else { 'v0.0.0' }
}
Write-Host "[release] version = $Version"

if (-not $SkipBuild) {
    Write-Host '[1/4] cargo build --release'
    & cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "game build failed ($LASTEXITCODE)" }

    Write-Host '[2/4] launcher build'
    Push-Location (Join-Path $root 'launcher')
    & cargo build --release
    $launcherCode = $LASTEXITCODE
    Pop-Location
    if ($launcherCode -ne 0) { throw "launcher build failed ($launcherCode)" }
}
else { Write-Host '[1-2/4] build skipped (-SkipBuild)' }

Write-Host '[3/4] assembling release_dist'
$dist = Join-Path $root 'release_dist'
if (Test-Path $dist) { Remove-Item $dist -Recurse -Force }
$game = Join-Path $dist 'game'
New-Item -ItemType Directory -Force -Path $game | Out-Null

Copy-Item (Join-Path $root 'target\release\steel-front.exe') $game
$rdv = Join-Path $root 'target\release\rdv.exe'
if (Test-Path $rdv) { Copy-Item $rdv $game }
Copy-Item (Join-Path $root 'assets') $game -Recurse
New-Item -ItemType Directory -Force -Path (Join-Path $game 'logs') | Out-Null

# Multiplayer helper scripts live in the repo so releases are reproducible.
$relAssets = Join-Path $root 'release_assets'
if (Test-Path $relAssets) {
    Get-ChildItem $relAssets -File | ForEach-Object { Copy-Item $_.FullName $game }
}

Copy-Item (Join-Path $root 'launcher\target\release\steel_front_launcher.exe') (Join-Path $dist 'SteelFrontLauncher.exe')
foreach ($f in @('LICENSE', 'README.md')) {
    $p = Join-Path $root $f
    if (Test-Path $p) { Copy-Item $p $dist }
}

$readme = @()
$readme += "Steel Front - portable build ($Version, $(Get-Date -Format 'yyyy-MM-dd'))"
$readme += '================================================='
$readme += ''
$readme += '1. Run SteelFrontLauncher.exe (first run shows the install wizard).'
$readme += '2. The launcher copies game\ to a folder of your choice and creates a desktop shortcut.'
$readme += '3. Or run game\steel-front.exe directly (portable: keep assets\ next to the exe).'
$readme += ''
$readme += 'Multiplayer:'
$readme += '  game\联机主机.bat   host on 0.0.0.0:27015'
$readme += '  game\联机加入.bat   join a host (asks for IP:port)'
$readme += '  game\rdv.exe         tiny UDP rendezvous for NAT punching (optional)'
$readme += ''
$readme += 'Settings are written to %USERPROFILE%\.steel_front.cfg on first run.'
$readme += 'Controls and troubleshooting: README.md (Chinese).'
$readme += 'Feedback: https://github.com/Jerry-Hang/steel-front/issues/new'
Set-Content -Path (Join-Path $dist 'README.txt') -Encoding UTF8 -Value $readme

Write-Host '[4/4] sanity checks'
$need = @(
    'SteelFrontLauncher.exe',
    'game\steel-front.exe',
    'game\assets\mesh.spv',
    'game\assets\triangle.frag.spv',
    'game\assets\maps\index.toml',
    'game\assets\props\building_tall.glb'
)
$missing = @($need | Where-Object { -not (Test-Path (Join-Path $dist $_)) })
if ($missing.Count -gt 0) { throw ('package is missing required files: ' + ($missing -join ', ')) }

$size = (Get-ChildItem $dist -Recurse -File | Measure-Object Length -Sum).Sum / 1MB
Write-Host ''
Write-Host ("=== RELEASE FOLDER OK: {0} ({1:N1} MB) ===" -f $dist, $size)
