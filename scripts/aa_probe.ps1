# aa_probe.ps1 - A/A noise floor probe: run the SAME binary N times and report the spread.
#
# Why this exists
# ---------------
# 2026-09-26: a shadow-pass A/B produced "median fps 101.8 -> 161.2" (+58%), which was then
# questioned and re-checked with four IDENTICAL runs (no code change at all):
#     run1 mean 131.3   run2 mean 130.6   run3 mean 137.9   run4 mean 146.8
# i.e. the same binary spans ~12% on that metric, and the single pair that "proved" +58% was
# just two samples from that spread. AGENTS.md lesson 24/35 says a single A/B proves nothing,
# but the repo had no tool to MEASURE the floor -- only a comment quoting "2.8% from two runs".
# This script is that tool: run it before claiming any perf delta.
#
# Usage:
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\aa_probe.ps1
#   ... -Runs 6 -Secs 20 -Stress 128            # the usual spine
#   ... -Extra "RV3D_SHADOW_EVERY=1"            # A/A of a specific configuration
#
# Reading the result: the printed spread (max-min)/min is the floor. A claimed improvement
# below that floor is unmeasured, no matter how clean the two logs look.
#
# Exit codes (2026-09-26). The header used to CLAIM "0 when every run produced a parsed fps
# line; 1 otherwise", while the code only failed when fewer than 2 runs parsed -- so 4 failed
# runs out of 6 still printed "the floor" and exited 0. Now the contract is the code:
#   0 = every requested run produced a steady-state fps line
#   1 = fewer than 2 usable runs (nothing can be said about spread)
#   2 = partial batch: the spread is printed, but it is NOT the floor for this binary+flags
param(
    [int]$Runs = 4,
    [int]$Secs = 20,
    [int]$Stress = 128,
    [string]$Extra = ""
)

$ErrorActionPreference = "Continue"
$repo = "D:\Rust\steel-front"
$out = Join-Path $repo "logs\aa_probe_raw.txt"
"" | Set-Content -Encoding ascii $out

$means = @()
$medians = @()
$failed = 0
for ($i = 1; $i -le $Runs; $i++) {
    # NOT $args: that is an automatic variable in PowerShell (script arguments), do not shadow it.
    $pArgs = @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", (Join-Path $repo "scripts\perf_run.ps1"),
               "-Secs", "$Secs", "-Stress", "$Stress")
    if ($Extra -ne "") { $pArgs += @("-Extra", $Extra) }
    $text = & powershell @pArgs 2>&1 | Out-String
    $runRc = $LASTEXITCODE
    $perf = [regex]::Match($text, "perf log = (\S+)").Groups[1].Value
    $m = [regex]::Match($text, "mean\s+([\d.]+)\s+median\s+([\d.]+)")
    if ($runRc -ne 0 -or -not $m.Success) {
        $failed++
        Write-Host ("aa_probe: run {0} unusable (perf_run exit code {1}); tail:" -f $i, $runRc)
        ($text -split "`n" | Select-Object -Last 6) | ForEach-Object { Write-Host ("    " + $_) }
        Add-Content -Encoding ascii $out ("run {0}`tUNUSABLE`trc={1}`t{2}" -f $i, $runRc, $perf)
        continue
    }
    $mean = [double]$m.Groups[1].Value
    $median = [double]$m.Groups[2].Value
    $means += $mean
    $medians += $median
    Add-Content -Encoding ascii $out ("run {0}`tmean {1:N2}`tmedian {2:N2}`t{3}" -f $i, $mean, $median, $perf)
    Write-Host ("aa_probe: run {0}/{1}  mean {2:N2}  median {3:N2}  ({4})" -f $i, $Runs, $mean, $median, $perf)
}

if ($means.Count -lt 2) {
    Write-Host "aa_probe: FAIL - need at least 2 parsed runs to say anything about spread"
    exit 1
}

function Spread($vals, $name) {
    $min = ($vals | Measure-Object -Minimum).Minimum
    $max = ($vals | Measure-Object -Maximum).Maximum
    $avg = ($vals | Measure-Object -Average).Average
    $pct = if ($min -gt 0) { 100.0 * ($max - $min) / $min } else { 0.0 }
    Write-Host ("  {0,-8} min {1,7:N2}  max {2,7:N2}  mean {3,7:N2}  spread {4:N1}%" -f $name, $min, $max, $avg, $pct)
}

Write-Host ""
Write-Host ("==== A/A noise floor: {0} usable runs of {1} requested (secs={2} stress={3}{4}) ====" -f `
    $means.Count, $Runs, $Secs, $Stress, $(if ($Extra -ne "") { " extra=$Extra" } else { "" }))
Spread $means "mean"
Spread $medians "median"
Write-Host "  a claimed delta below the spread above is NOT measured (AGENTS lesson 24/35)"
Write-Host ("  raw log   {0}" -f $out)

if ($failed -gt 0) {
    Write-Host ("aa_probe: exit 2 - {0} of {1} runs were unusable; the spread above is NOT the floor" -f `
        $failed, $Runs)
    exit 2
}
exit 0
