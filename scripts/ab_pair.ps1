# ab_pair.ps1 - interleaved A/B driver for two engine binaries (ASCII only).
#
# Why this exists
# ---------------
# Repo lesson 24: a single A/B pair proves nothing. The only design that survives a drifting
# machine is (a) alternate the two arms run-by-run so drift hits both equally, (b) repeat,
# (c) rank on the median of the PAIRED differences, and (d) run the same exe in both arms
# once to measure the A/A floor. Lessons 43/45 add the rest: the ruler is the engine's own
# window-fps column, and a batch whose control arm is not clearly faster must be discarded.
#
# Until now that was rebuilt by hand every time (2026-09-26: the terrain density re-measure
# needed it and had to be written from scratch in logs/). This script is that harness, kept.
#
# Ruler: scripts/perf_run.ps1 is used unchanged (same env, same log, same steady-state rule
# t >= 3s), so numbers stay comparable with every earlier perf_run / aa_probe measurement.
#
# Usage
# -----
#   # 1) stage two binaries (build them however you like, e.g. flip one const and rebuild)
#   #    logs/ab/old.exe, logs/ab/new.exe
#   # 2) A/A floor first (same file twice) -- 3 pairs is enough to see the spread
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\ab_pair.ps1 -Pairs 3 `
#       -ExeA logs\ab\new.exe -ExeB logs\ab\new.exe -LabelA newA -LabelB newB
#   # 3) then the real thing, both orders (lesson 24: swap the arms and check it holds)
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\ab_pair.ps1 -Pairs 5
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\ab_pair.ps1 -Pairs 3 `
#       -ExeA logs\ab\new.exe -ExeB logs\ab\old.exe -LabelA new129 -LabelB old257
#
# Read the output as: "MEDIAN PAIRED DELTA" is the effect, "arm range" is the drift, and the
# sign test is how many pairs agreed. A delta inside the A/A floor is not an effect.
#
# One instance at a time: two games would contend for the GPU and both fps numbers are noise.
param(
    [int]$Pairs = 4,
    [int]$Secs = 25,
    [string]$ExeA = "logs\ab\old.exe",
    [string]$ExeB = "logs\ab\new.exe",
    [string]$LabelA = "A",
    [string]$LabelB = "B",
    # Engine env switches per arm, in perf_run.ps1 -Extra syntax ("K=V,K2=V2").
    # For a COST MAP both arms use the same exe and only -ExtraB differs; the arm that
    # removes work must be measurably faster -- if it is not, the whole batch is void
    # (repo lesson 45: a batch whose control arm does not move is measuring drift).
    [string]$ExtraA = "",
    [string]$ExtraB = ""
)
$ErrorActionPreference = "Continue"
$repo = "D:\Rust\steel-front"
$target = Join-Path $repo "target\release\steel-front.exe"
$pathA = Join-Path $repo $ExeA
$pathB = Join-Path $repo $ExeB
foreach ($p in @($pathA, $pathB)) {
    if (-not (Test-Path $p)) { Write-Host "ab_pair: missing $p"; exit 1 }
}

function Run-Arm([string]$exe, [string]$label, [int]$secs, [string]$extra) {
    Copy-Item -Force $exe $target
    # 2026-09-26: an EMPTY string cannot be passed as an argument here.
    # `powershell -File perf_run.ps1 -Extra ""` fails parameter binding with
    # "Missing an argument for parameter 'Extra'", so perf_run never runs and prints
    # nothing -- every pair then reports "incomplete" (the first cost map burned ten
    # minutes spinning like that). The no-switch arm must OMIT -Extra entirely.
    # (Keep this file pure ASCII: PS 5.1 reads BOM-less .ps1 as ANSI.)
    $psArgs = @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File")
    $psArgs += (Join-Path $repo "scripts\perf_run.ps1")
    $psArgs += @("-Secs", "$secs")
    if ($extra -ne "") { $psArgs += @("-Extra", $extra) }
    $out = & powershell @psArgs 2>&1
    $line = ($out | Select-String -Pattern 'fps\s+mean' | Select-Object -First 1)
    if (-not $line) {
        Write-Host ("ab_pair: {0} produced no fps line; tail:" -f $label)
        $out | Select-Object -Last 6 | ForEach-Object { Write-Host ("    " + $_) }
        return $null
    }
    $m = [regex]::Match($line.Line, 'mean\s+([0-9,\.]+)\s+median\s+([0-9,\.]+)')
    if (-not $m.Success) { Write-Host ("ab_pair: cannot parse: " + $line.Line); return $null }
    return [double]($m.Groups[2].Value -replace ',', '')
}

function Median($vals) {
    $s = @($vals | Sort-Object)
    $n = $s.Count
    if ($n % 2 -eq 1) { return $s[[int](($n - 1) / 2)] }
    return ($s[$n / 2 - 1] + $s[$n / 2]) / 2
}

Write-Host ("ab_pair: {0} pairs, {1}s per run; A={2} ({3}){6}  B={4} ({5}){7}" -f $Pairs, $Secs, $LabelA, $ExeA,
    $LabelB, $ExeB, $(if ($ExtraA -ne "") { " [$ExtraA]" } else { "" }), $(if ($ExtraB -ne "") { " [$ExtraB]" } else { "" }))
$deltas = @(); $as = @(); $bs = @()
for ($i = 1; $i -le $Pairs; $i++) {
    $a = Run-Arm $pathA $LabelA $Secs $ExtraA
    $b = Run-Arm $pathB $LabelB $Secs $ExtraB
    if ($null -eq $a -or $null -eq $b) { Write-Host "ab_pair: pair $i incomplete, skipping"; continue }
    $as += $a; $bs += $b
    $d = $b - $a
    $deltas += $d
    Write-Host ("  pair {0}: {1}={2:N2}  {3}={4:N2}  delta={5:+0.00;-0.00} ({6:+0.0;-0.0}%)" -f `
        $i, $LabelA, $a, $LabelB, $b, $d, (100.0 * $d / $a))
}

if ($deltas.Count -eq 0) { Write-Host "ab_pair: no complete pairs"; exit 1 }

$mA = Median $as
$mB = Median $bs
$md = Median $deltas
$sortedA = @($as | Sort-Object); $sortedB = @($bs | Sort-Object)
$spreadA = 100.0 * ($sortedA[-1] - $sortedA[0]) / $mA
$spreadB = 100.0 * ($sortedB[-1] - $sortedB[0]) / $mB
Write-Host ""
Write-Host ("==== ab_pair result ({0} complete pairs) ====" -f $deltas.Count)
Write-Host ("  {0}: median {1:N2} fps (arm range {2:N1}%, n={3})" -f $LabelA, $mA, $spreadA, $as.Count)
Write-Host ("  {0}: median {1:N2} fps (arm range {2:N1}%, n={3})" -f $LabelB, $mB, $spreadB, $bs.Count)
Write-Host ("  paired deltas (B-A): " + (($deltas | ForEach-Object { "{0:+0.00;-0.00}" -f $_ }) -join "  "))
Write-Host ("  MEDIAN PAIRED DELTA = {0:+0.00;-0.00} fps ({1:+0.00;-0.00}% of {2})" -f $md, (100.0 * $md / $mA), $LabelA)
$signs = @($deltas | ForEach-Object { [math]::Sign($_) })
$pos = @($signs | Where-Object { $_ -gt 0 }).Count
Write-Host ("  sign test: {0} of {1} pairs favour {2}" -f $pos, $deltas.Count, $LabelB)
Write-Host "  NOTE: compare |MEDIAN PAIRED DELTA| against the A/A floor measured with -ExeA == -ExeB."
exit 0
