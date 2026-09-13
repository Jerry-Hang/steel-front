# AI thread affinity test (requested by the user, 2026-09-13).
#
# WHY: we want to know whether the AI pool can be dumped onto E-cores.
# HOW: bind ai_pool to 1C2T / 2C4T / 3C6T / 4C8T in turn -- i.e. simulate an Intel
#      part with 4 / 8 / 12 / 16 E-cores -- under a HEAVY AI load
#      (RV3D_STRESS_AI=128, 128 NPCs per side) and measure fps.
#
# READING THE RESULT:
#   * fps stops improving at 1C2T  => the AI never needed more than 1 core; the
#     current "hand the whole CCD1 to AI" allocation is oversized and can be cut.
#   * fps climbs with core count  => the AI really does saturate; keep the allocation.
#
# fps is taken from the game's `cam:` log line: fps = 1e6 / cyc (cyc is cycle_us).
# MEDIAN, not mean: the first frames and scene transitions are outliers.
#
# ASCII ONLY. Windows PowerShell 5.1 reads a BOM-less .ps1 as ANSI, so non-ASCII
# inside a string literal breaks quote pairing (AGENTS.md lesson 7).

param(
    [int]$WarmupSec = 18
)

$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo
$exe = Join-Path $repo "target\release\steel-front.exe"

# name + RV3D_AI_CPUS value ("" = leave the default ai_set alone)
$levels = @(
    @{ Name = "default (8 cores, CCD1)"; Cpus = "" },
    @{ Name = "1C2T (sim 4 E-cores)";    Cpus = "16,17" },
    @{ Name = "2C4T (sim 8 E-cores)";    Cpus = "16,17,18,19" },
    @{ Name = "3C6T (sim 12 E-cores)";   Cpus = "16,17,18,19,20,21" },
    @{ Name = "4C8T (sim 16 E-cores)";   Cpus = "16,17,18,19,20,21,22,23" }
)

Write-Host "==== AI thread affinity test (heavy AI load, RV3D_STRESS_AI=128) ===="
Write-Host ("{0,-26} {1,10} {2,10} {3,8}" -f "config", "med fps", "min fps", "samples")

$results = @()
foreach ($lv in $levels) {
    $tag = "aff_" + ($lv.Name -replace '[^0-9A-Za-z]', '_')
    $log = Join-Path $repo "logs\$tag.log.err"
    if (Test-Path $log) { Remove-Item $log -Force }

    $env:RV3D_STRESS_AI = "128"
    if ($lv.Cpus -ne "") { $env:RV3D_AI_CPUS = $lv.Cpus }
    else { Remove-Item Env:RV3D_AI_CPUS -ErrorAction SilentlyContinue }

    # -RedirectStandardError is essential: without it the game's log (and therefore the
    # `cam:` line we measure fps from) goes to a console that dies with the process.
    $p = Start-Process -FilePath $exe -WorkingDirectory $repo -WindowStyle Hidden `
        -RedirectStandardError $log -RedirectStandardOutput "$log.out" -PassThru
    Start-Sleep -Seconds $WarmupSec
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 2
    Remove-Item Env:RV3D_STRESS_AI, Env:RV3D_AI_CPUS -ErrorAction SilentlyContinue

    $fps = @()
    if (Test-Path $log) {
        foreach ($line in [System.IO.File]::ReadAllLines($log, [System.Text.Encoding]::UTF8)) {
            if ($line -match 'cyc=(\d+)') {
                $c = [double]$Matches[1]
                if ($c -gt 0) { $fps += (1000000.0 / $c) }
            }
        }
    }
    if ($fps.Count -ge 3) {
        $sorted = $fps | Sort-Object
        $med = $sorted[[int]($sorted.Count / 2)]
        $mn = $sorted[0]
        Write-Host ("{0,-26} {1,10:N1} {2,10:N1} {3,8}" -f $lv.Name, $med, $mn, $fps.Count)
        $results += [pscustomobject]@{ Name = $lv.Name; Median = $med }
    } else {
        Write-Host ("{0,-26} {1,10} {2,10} {3,8}" -f $lv.Name, "-", "-", $fps.Count)
    }
}

Write-Host ""
Write-Host "==== verdict ===="
if ($results.Count -ge 2) {
    $base = $results[0].Median
    foreach ($r in $results) {
        $d = 0.0
        if ($base -gt 0) { $d = ($r.Median - $base) / $base * 100.0 }
        Write-Host ("  {0,-26} vs default {1,6:N1}%" -f $r.Name, $d)
    }
    $one = 0.0
    $four = 0.0
    foreach ($r in $results) {
        if ($r.Name -like "1C2T*") { $one = $r.Median }
        if ($r.Name -like "4C8T*") { $four = $r.Median }
    }
    if ($one -gt 0 -and $four -gt 0) {
        $gain = ($four - $one) / $one * 100.0
        Write-Host ""
        Write-Host ("  1C2T -> 4C8T gain: {0:N1}%" -f $gain)
        if ($gain -lt 3.0) {
            Write-Host "  => AI already saturates 1C2T: allocation is OVERSIZED, shrink it and free the cores."
        } elseif ($gain -gt 10.0) {
            Write-Host "  => AI scales with core count: allocation is NEEDED, do not shrink."
        } else {
            Write-Host "  => moderate gain, borderline case."
        }
    }
}
