# perf_run.ps1 - Windows-native performance harness for Steel Front.
#
# Why this exists
# ---------------
# scripts/playtest_perf.py is the long-standing perf harness, but it is X11-only:
# libX11/XTest input, XImage screen capture, pgrep/pkill, /proc/<pid>/status.
# Porting it is not a port, it is a rewrite. This script instead reuses what this
# repo already has and has actually been measured:
#   * the engine writes logs/perf_<stamp>.log once a second, with fps plus every
#     render stage in microseconds (see src/perf_log.rs);
#   * launching and killing is plain Get-Process/Stop-Process (see run_smoke_pm.ps1).
# So this needs no input injection and no screen capture at all.
#
# Stress mode is ON by default (RV3D_STRESS_AI=128 per side), because measuring the
# 8-NPC tutorial path tells you nothing about the frame budget that matters.
#
# Usage:
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\perf_run.ps1
#   ... -Secs 60                 # shorter run
#   ... -Stress 0                # traditional wave mode instead
#   ... -NoShadow                # RV3D_NO_SHADOW=1 (shadow cost A/B)
#   ... -Cam "0,0:0,0"           # RV3D_CAM fixed viewpoint (reproducible framing)
#
# Exit code 0 when a perf log was produced and parsed; 1 otherwise.
#
# MEASURED NOISE FLOOR (2026-09-15)
# ---------------------------------
# Two consecutive runs of the SAME binary (stress=128, 25s) gave median fps 69.7 and
# 71.8 -- a 2.8% spread. Treat any between-run difference below ~5% as unmeasured
# until you have several repetitions per arm; a single A/B here proves nothing.
# (This is the repo's own lesson 24: one run per arm is not evidence.)
param(
    [int]$Secs = 60,
    [int]$Stress = 128,
    [switch]$NoShadow,
    [string]$Cam = "",
    [string]$Res = "",
    # -CullDiag sets RV3D_CULL_DIAG=1: the engine then logs one `cull-diag:` line per second
    # with the measured CPU cost of the NPC occlusion culling (us/s, call count, NPC and
    # obstacle-body counts). Use it before touching that code path -- see lesson 20/25.
    [switch]$CullDiag
)
$ErrorActionPreference = "Continue"

$repo = "D:\Rust\steel-front"
$exe  = Join-Path $repo "target\release\steel-front.exe"
$LOG    = Join-Path $repo "logs\perf_run.log"
$LOGERR = "$LOG.err"

if (-not (Test-Path $exe)) { Write-Host "perf_run: missing $exe (run cargo build --release)"; exit 1 }

# Record which perf logs exist BEFORE the run, so afterwards we can tell exactly
# which one this run produced instead of guessing by timestamp.
$before = @(Get-ChildItem (Join-Path $repo "logs") -Filter "perf_*.log" -ErrorAction SilentlyContinue |
            ForEach-Object { $_.FullName })

Get-Process -Name steel-front -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 2
Remove-Item -Force $LOG, $LOGERR -ErrorAction SilentlyContinue

# One instance only: two would contend for the GPU and both fps numbers become noise.
$env:RV3D_AUTOSTART = "1"
$env:RV3D_STRESS_AI = "$Stress"
if ($NoShadow) { $env:RV3D_NO_SHADOW = "1" }
if ($Cam -ne "") { $env:RV3D_CAM = $Cam }
if ($Res -ne "") { $env:RV3D_RES = $Res }
if ($CullDiag) { $env:RV3D_CULL_DIAG = "1" }

Write-Host "perf_run: stress=$Stress secs=$Secs$(if ($NoShadow) { ' noshadow' })$(if ($Cam -ne '') { " cam=$Cam" })$(if ($CullDiag) { ' culldiag' })"

$rc = 1
try {
    $p = Start-Process -FilePath $exe -WorkingDirectory $repo -WindowStyle Hidden `
                       -RedirectStandardOutput $LOG -RedirectStandardError $LOGERR -PassThru
    Write-Host ("perf_run: pid {0}, sampling {1}s" -f $p.Id, $Secs)
    for ($i = 0; $i -lt $Secs; $i++) {
        Start-Sleep -Seconds 1
        if ($p.HasExited) { Write-Host ("perf_run: game exited early (code {0})" -f $p.ExitCode); break }
    }
}
finally {
    # Release the machine no matter what: kill the game, then verify via the repo's
    # own checker (it also lifts ClipCursor) so a crashed run cannot leave the
    # pointer captured.
    Get-Process -Name steel-front -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Seconds 1
    Remove-Item Env:RV3D_AUTOSTART, Env:RV3D_STRESS_AI, Env:RV3D_NO_SHADOW, Env:RV3D_CAM, Env:RV3D_RES, Env:RV3D_CULL_DIAG -ErrorAction SilentlyContinue
}

$perf = @(Get-ChildItem (Join-Path $repo "logs") -Filter "perf_*.log" -ErrorAction SilentlyContinue |
          Where-Object { $before -notcontains $_.FullName } |
          Sort-Object LastWriteTime -Descending)
if ($perf.Count -eq 0) {
    Write-Host "perf_run: FAIL - no new logs/perf_*.log was produced. Check logs\perf_run.log.err"
    exit 1
}
$perfPath = $perf[0].FullName
Write-Host ("perf_run: perf log = {0}" -f (Split-Path $perfPath -Leaf))

# Columns: 0=t 1=fps 2=frame_us 3=cull 4=terrain 5=wait 6=acquire 7=record 8=submit 9=present 10=near
$rows = @()
foreach ($line in Get-Content $perfPath) {
    $f = $line -split "`t"
    if ($f.Count -lt 11) { continue }
    $fps = 0.0
    if (-not [double]::TryParse($f[1], [ref]$fps)) { continue }
    $rows += [pscustomobject]@{
        t        = [double]$f[0]
        fps      = $fps
        frame_us = [double]$f[2]
        cull     = [double]$f[3]
        terrain  = [double]$f[4]
        wait     = [double]$f[5]
        record   = [double]$f[7]
        submit   = [double]$f[8]
        present  = [double]$f[9]
        near     = [int]$f[10]
    }
}

if ($rows.Count -lt 5) {
    Write-Host ("perf_run: FAIL - only {0} sample row(s) parsed; the run was too short or the game never rendered" -f $rows.Count)
    exit 1
}

# The first sample is not representative: SPIR-V regeneration makes the driver JIT
# the pipelines on the very first frames, and the reported fps there is a cold-cache
# artefact (this is the documented "first-frame window" trap in AGENTS.md). Report
# both, but rank on the steady-state numbers so a run is not judged by its first second.
$warm = @($rows | Where-Object { $_.t -ge 3.0 })
if ($warm.Count -lt 3) { $warm = $rows }

function Stat($vals, $name, $unit) {
    $sorted = @($vals | Sort-Object)
    $n = $sorted.Count
    $mean = ($vals | Measure-Object -Average).Average
    $median = if ($n % 2 -eq 1) { $sorted[[int](($n - 1) / 2)] } else { ($sorted[$n / 2 - 1] + $sorted[$n / 2]) / 2 }
    $p95 = $sorted[[int][math]::Min($n - 1, [math]::Floor(0.95 * $n))]
    Write-Host ("  {0,-9} mean {1,8:N2}{4}  median {2,8:N2}{4}  p95 {3,8:N2}{4}  min {5:N2}  max {6:N2}" -f `
        $name, $mean, $median, $p95, $unit, $sorted[0], $sorted[$n - 1])
}

Write-Host ""
Write-Host ("==== perf_run result: {0} samples over {1:N0}s (steady-state = t >= 3s, n={2}) ====" -f `
    $rows.Count, $rows[-1].t, $warm.Count)
Stat @($warm | ForEach-Object { $_.fps }) "fps" ""
Stat @($warm | ForEach-Object { $_.frame_us }) "frame_us" "us"
Stat @($warm | ForEach-Object { $_.cull }) "cull" "us"
Stat @($warm | ForEach-Object { $_.terrain }) "terrain" "us"
Stat @($warm | ForEach-Object { $_.present }) "present" "us"
Write-Host ("  npc boxes  near={0}" -f $rows[-1].near)
Write-Host ("  full log   {0}" -f $perfPath)
if ($rows.Count -ne $warm.Count) {
    $cold = $rows[0]
    Write-Host ("  (first sample t={0:N1}s fps={1:N1} excluded as cold cache)" -f $cold.t, $cold.fps)
}

# Hand the machine back, and say so out loud rather than assuming it worked.
& powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $repo "scripts\release_input.ps1") 2>&1 |
    Select-Object -Last 1 | ForEach-Object { Write-Host ("  " + $_) }

exit 0
