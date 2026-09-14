# Run the red/blue asymmetry experiment properly: N repetitions per arm.
#
# WHY REPETITIONS
# ---------------
# A previous single-run A/B was written up as a conclusion and then had to be
# retracted: arm C, run twice with an identical command, produced blue-side
# margins of +51 and +11 -- an intra-arm spread of ~40 points, an order of
# magnitude larger than the 8-point difference between arms A and B. Single runs
# of this scenario cannot answer the question. This script runs each arm several
# times and reports the DISTRIBUTION.
#
# ARMS
#   baseline  : red spawns at +X, blue at -X (production behaviour, default env)
#   swapped   : RV3D_SWAP_SIDES=1 -- the two spawn halves exchange places.
#               Nothing else changes: same teams, same counts, same roles. This is
#               the clean single-variable contrast.
#
# MEASUREMENT
#   The `ai:` log line prints `red=<alive> blue=<alive>` once per second. Casualties
#   are (first sample - last sample) per team. `blue margin` = red_loss - blue_loss,
#   so a POSITIVE margin means blue did better.
#
# Usage:  powershell -NoProfile -ExecutionPolicy Bypass -File scripts\ab_reps.ps1
#         -Reps 3 -Secs 125 -Tag ab1

param(
    [int]$Reps = 3,
    [int]$Secs = 125,
    [string]$Tag = "ab"
)

$ErrorActionPreference = "Continue"
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo
$outDir = Join-Path $repo "logs\ab_$Tag"
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

function Invoke-Arm([string]$arm, [int]$rep) {
    # cap_safe writes logs/<tag>.log.err, and the tag it is given is built here.
    # Reading from a different path than cap_safe writes to was the first version's
    # bug: all six runs completed and then reported "no usable samples".
    $tagFull = "ab_$Tag`_$arm`_$rep"
    $log = Join-Path $repo "logs\$tagFull.log.err"
    if (Test-Path $log) { Remove-Item $log -Force }

    if ($arm -eq "swapped") { $env:RV3D_SWAP_SIDES = "1" }
    else { Remove-Item Env:RV3D_SWAP_SIDES -ErrorAction SilentlyContinue }
    $env:RV3D_STRESS_AI = "128"

    & powershell -NoProfile -ExecutionPolicy Bypass -File scripts\cap_safe.ps1 `
        -Tag $tagFull -WarmupSec 4 -HoldSec 1 -Keys @(82) -AfterKeysSec $Secs 2>&1 | Out-Null

    Remove-Item Env:RV3D_STRESS_AI -ErrorAction SilentlyContinue
    Remove-Item Env:RV3D_SWAP_SIDES -ErrorAction SilentlyContinue

    $p = @(Get-Process -Name steel-front -ErrorAction SilentlyContinue)
    if ($p.Count -gt 0) { $p | Stop-Process -Force; Start-Sleep -Seconds 2 }

    if (-not (Test-Path $log)) { return $null }
    $t = [System.IO.File]::ReadAllLines($log, [System.Text.Encoding]::UTF8)
    $lines = @($t | Where-Object { $_ -match 'ai: npcs=.*red=(\d+) blue=(\d+)' })
    if ($lines.Count -lt 10) { return $null }
    $r0 = 0; $b0 = 0; $r1 = 0; $b1 = 0
    if ($lines[0]  -match 'red=(\d+) blue=(\d+)') { $r0 = [int]$Matches[1]; $b0 = [int]$Matches[2] }
    if ($lines[-1] -match 'red=(\d+) blue=(\d+)') { $r1 = [int]$Matches[1]; $b1 = [int]$Matches[2] }
    return [pscustomobject]@{
        Arm = $arm; Rep = $rep
        Red0 = $r0; Blue0 = $b0; Red1 = $r1; Blue1 = $b1
        RedLoss = $r0 - $r1; BlueLoss = $b0 - $b1
        Margin = ($r0 - $r1) - ($b0 - $b1)
        Samples = $lines.Count
    }
}

$rows = @()
foreach ($arm in @("baseline", "swapped")) {
    for ($i = 1; $i -le $Reps; $i++) {
        Write-Host ("  running {0} rep {1}/{2} ..." -f $arm, $i, $Reps)
        $r = Invoke-Arm $arm $i
        if ($null -ne $r) {
            $rows += $r
            Write-Host ("    redLoss={0,3}  blueLoss={1,3}  blueMargin={2,4}" -f $r.RedLoss, $r.BlueLoss, $r.Margin)
        } else {
            Write-Host "    no usable samples -- skipped"
        }
    }
}

$csv = Join-Path $outDir "results.csv"
$rows | Export-Csv -Path $csv -NoTypeInformation -Encoding UTF8

Write-Host ""
Write-Host "================ SUMMARY ================"
foreach ($arm in @("baseline", "swapped")) {
    $a = @($rows | Where-Object { $_.Arm -eq $arm })
    if ($a.Count -eq 0) { continue }
    $m = $a | Measure-Object -Property Margin -Average -Minimum -Maximum
    $spread = $m.Maximum - $m.Minimum
    Write-Host ("  {0,-9} n={1}  margin mean={2,6:N1}  min={3,4}  max={4,4}  spread={5}" -f `
        $arm, $a.Count, $m.Average, $m.Minimum, $m.Maximum, $spread)
}
$all = @($rows)
if ($all.Count -ge 4) {
    $within = ($all | Group-Object Arm | ForEach-Object {
        $g = $_.Group | Measure-Object -Property Margin -Minimum -Maximum
        $g.Maximum - $g.Minimum
    } | Measure-Object -Average).Average
    $b = @($rows | Where-Object { $_.Arm -eq "baseline" } | Measure-Object -Property Margin -Average).Average
    $s = @($rows | Where-Object { $_.Arm -eq "swapped" }  | Measure-Object -Property Margin -Average).Average
    $between = [Math]::Abs($b - $s)
    Write-Host ""
    Write-Host ("  within-arm spread (avg) : {0:N1}" -f $within)
    Write-Host ("  between-arm difference   : {0:N1}" -f $between)
    if ($between -lt $within) {
        Write-Host "  => the effect is SMALLER than the noise. This cannot settle the question."
    } else {
        Write-Host "  => the effect EXCEEDS the noise. Direction: " +
            $(if ($s -gt $b) { "swapping helps blue" } else { "swapping hurts blue" })
    }
}
Write-Host ""
Write-Host ("  csv: " + $csv)
