# =============================================================================
#  package_windows_zip.ps1 -- zip the assembled release_dist for distribution
# =============================================================================
#  ASCII ONLY (see make_release.ps1 header for why).
#
#  Usage:
#     powershell -NoProfile -ExecutionPolicy Bypass -File scripts\package_windows_zip.ps1
#     ... -Version v0.2.0 -SkipBuild   (assume release_dist is already assembled)
#
#  Output: dist\steel-front-<version>.zip
#  Root entries mirror release_dist exactly (SteelFrontLauncher.exe, README.txt,
#  game\) -- that is what the launcher's updater expects.
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

if (-not $SkipBuild) {
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $root 'scripts\make_release.ps1') -Version $Version
    if ($LASTEXITCODE -ne 0) { throw "make_release failed ($LASTEXITCODE)" }
}

$dist = Join-Path $root 'release_dist'
if (-not (Test-Path (Join-Path $dist 'SteelFrontLauncher.exe'))) { throw 'release_dist is not assembled' }
if (-not (Test-Path (Join-Path $dist 'game\steel-front.exe'))) { throw 'release_dist\game\steel-front.exe missing' }

$outDir = Join-Path $root 'dist'
New-Item -ItemType Directory -Force -Path $outDir | Out-Null
$zip = Join-Path $outDir ("steel-front-{0}.zip" -f $Version)
if (Test-Path $zip) { Remove-Item $zip -Force }

# Python zipfile sets the UTF-8 entry-name flag; tar/Compress-Archive do not on
# Windows, which garbles the Chinese .bat file names on extraction.
$zipper = Join-Path $root 'tools\zip_release.py'
if (-not (Test-Path $zipper)) { throw "missing $zipper" }
& python -X utf8 $zipper $dist $zip
if ($LASTEXITCODE -ne 0 -or -not (Test-Path $zip)) { throw "zip creation failed ($LASTEXITCODE)" }

$sizeMb = (Get-Item $zip).Length / 1MB
$entries = @(& tar.exe -tf $zip 2>$null)
$hasGame = @($entries | Where-Object { $_ -match '^game/steel-front\.exe$' }).Count -gt 0
$hasLauncher = @($entries | Where-Object { $_ -match '^SteelFrontLauncher\.exe$' }).Count -gt 0
Write-Host ''
Write-Host ("=== ZIP OK: {0} ({1:N1} MB, {2} entries) ===" -f $zip, $sizeMb, $entries.Count)
Write-Host ("    launcher={0} game={1}" -f $hasLauncher, $hasGame)
