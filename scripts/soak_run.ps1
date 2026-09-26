# Soak run: long unattended session with drift verdicts (ASCII only).
#
# Why this exists (2026-09-26): every harness in this repo runs for 20-500 seconds. The bugs
# found that day were long-session bugs (an entity table that never shrank, a swapchain retry
# loop that spammed 5900 log lines) -- none of them would show up in a 30s perf run. This script
# answers three questions about a LONG run, with numbers:
#   1. does fps drift down?         (first third vs last third, median)
#   2. does memory grow?            (MB at t=30s vs at the end, plus MB/min)
#   3. does anything rot in the log?(VUID / device lost / panic counts)
#
# It deliberately does NOT inject input: it measures the idle-but-live game (stress AI keeps the
# simulation, the wave system and the renderer busy). Use run_survive_pm.ps1 / run_smoke_pm.ps1
# when the question is about gameplay instead.
#
# Usage:
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\soak_run.ps1 -Secs 600
#   ... -Secs 300 -Stress 255 -Validation      # with the validation layer on
param(
    [int]$Secs = 600,
    [int]$Stress = 128,
    [string]$Tag = "soak",
    [int]$PollSecs = 10,
    [switch]$Validation,
    [string]$Extra = ""
)
$ErrorActionPreference = "Continue"
$repo = "D:\Rust\steel-front"
$exe  = Join-Path $repo "target\release\steel-front.exe"
$logs = Join-Path $repo "logs"
$logOut = Join-Path $logs "$Tag.log"
$logErr = Join-Path $logs "$Tag.log.err"
$csv    = Join-Path $logs "$Tag.samples.csv"

if (-not (Test-Path $exe)) { Write-Host "soak_run: missing $exe (cargo build --release)"; exit 1 }

Get-Process -Name steel-front -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 2
Remove-Item -Force $logOut, $logErr, $csv -ErrorAction SilentlyContinue

$before = @(Get-ChildItem $logs -Filter "perf_*.log" -ErrorAction SilentlyContinue | ForEach-Object { $_.FullName })

# Stress mode keeps ~256 NPCs alive; the engine's own perf log gives fps per second.
$env:RV3D_AUTOSTART = "1"
$env:RV3D_STRESS_AI = "$Stress"
$env:RV3D_GPU = "dgpu"
$env:RV3D_PRESENT_MODE = "mailbox"
if ($Validation) {
    $env:RV3D_VALIDATION = "1"
    $env:DISABLE_RTSS_LAYER = "1"
    $env:DISABLE_GAMEPP_LAYER = "1"
}
$extraKeys = @()
if ($Extra -ne "") {
    foreach ($kv in $Extra.Split(",;")) {
        if ($kv.Trim() -eq "") { continue }
        $parts = $kv.Split("=")
        if ($parts.Count -ne 2) { Write-Host "soak_run: bad -Extra item '$kv'"; exit 1 }
        Set-Item -Path ("Env:" + $parts[0].Trim()) -Value $parts[1].Trim()
        $extraKeys += $parts[0].Trim()
    }
}

Write-Host ("soak_run: stress={0} secs={1} validation={2}{3}" -f $Stress, $Secs, $Validation, $(if ($Extra) { " extra=$Extra" } else { "" }))
"t_s,working_set_mb,private_mb,handles" | Set-Content -Encoding ASCII $csv

$exitNote = "ok"
try {
    $proc = Start-Process -FilePath $exe -WorkingDirectory $repo -WindowStyle Hidden `
        -RedirectStandardOutput $logOut -RedirectStandardError $logErr -PassThru
    Write-Host ("soak_run: pid {0}" -f $proc.Id)
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.Elapsed.TotalSeconds -lt $Secs) {
        Start-Sleep -Seconds $PollSecs
        if ($proc.HasExited) {
            $exitNote = "GAME EXITED EARLY (code $($proc.ExitCode))"
            Write-Host "!! $exitNote"
            break
        }
        try {
            $proc.Refresh()
            $ws = [math]::Round($proc.WorkingSet64 / 1MB, 1)
            $pv = [math]::Round($proc.PrivateMemorySize64 / 1MB, 1)
            $hd = $proc.HandleCount
            $t = [math]::Round($sw.Elapsed.TotalSeconds, 0)
            "$t,$ws,$pv,$hd" | Add-Content -Encoding ASCII $csv
            Write-Host ("  t={0,4}s  ws={1,7} MB  private={2,7} MB  handles={3}" -f $t, $ws, $pv, $hd)
        } catch {
            Write-Host "  (process counters unavailable this tick)"
        }
    }
}
catch {
    $exitNote = "HARNESS ERROR: $($_.Exception.Message)"
    Write-Host "!! $exitNote"
}
finally {
    Get-Process -Name steel-front -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Seconds 1
    Remove-Item Env:RV3D_AUTOSTART, Env:RV3D_STRESS_AI, Env:RV3D_GPU, Env:RV3D_PRESENT_MODE, Env:RV3D_VALIDATION, Env:DISABLE_RTSS_LAYER, Env:DISABLE_GAMEPP_LAYER -ErrorAction SilentlyContinue
    foreach ($k in $extraKeys) { Remove-Item Env:($k) -ErrorAction SilentlyContinue }
}

# ---------------- verdicts -------------------------------------------------------------------
$rows = @()
foreach ($line in Get-Content $csv) {
    $f = $line -split ","
    if ($f.Count -lt 4) { continue }
    $t = 0.0; $ws = 0.0; $pv = 0.0
    if (-not [double]::TryParse($f[0], [ref]$t)) { continue }
    [void][double]::TryParse($f[1], [ref]$ws)
    [void][double]::TryParse($f[2], [ref]$pv)
    $rows += [pscustomobject]@{ t = $t; ws = $ws; pv = $pv; handles = [int]$f[3] }
}
$txt = ""
if (Test-Path $logErr) { $txt = [System.IO.File]::ReadAllText($logErr) }
$vuid = ([regex]::Matches($txt, "VUID-")).Count
$lost = ([regex]::Matches($txt, "has been lost")).Count
$panic = ([regex]::Matches($txt, "panicked")).Count
$logMB = 0.0
if (Test-Path $logErr) { $logMB = [math]::Round((Get-Item $logErr).Length / 1MB, 2) }

$perf = @(Get-ChildItem $logs -Filter "perf_*.log" -ErrorAction SilentlyContinue |
          Where-Object { $before -notcontains $_.FullName } | Sort-Object LastWriteTime -Descending)
$fpsFirst = $null; $fpsLast = $null; $samples = 0
if ($perf.Count -gt 0) {
    $vals = @()
    foreach ($line in Get-Content $perf[0].FullName) {
        $f = $line -split "`t"
        if ($f.Count -lt 11) { continue }
        $v = 0.0
        if ([double]::TryParse($f[1], [ref]$v)) { $vals += [pscustomobject]@{ t = [double]$f[0]; fps = $v } }
    }
    $warm = @($vals | Where-Object { $_.t -ge 3.0 })
    $samples = $warm.Count
    if ($samples -ge 6) {
        $third = [int][math]::Floor($samples / 3)
        $first = @($warm[0..($third - 1)] | ForEach-Object { $_.fps } | Sort-Object)
        $last = @($warm[($samples - $third)..($samples - 1)] | ForEach-Object { $_.fps } | Sort-Object)
        $fpsFirst = [math]::Round($first[[int]($first.Count / 2)], 1)
        $fpsLast = [math]::Round($last[[int]($last.Count / 2)], 1)
    }
}

Write-Host ""
Write-Host "=== soak summary ($Tag) ==="
Write-Host "  exit note    : $exitNote"
Write-Host ("  samples      : {0} memory rows, {1} perf seconds" -f $rows.Count, $samples)
if ($rows.Count -ge 4) {
    $firstRow = $rows[0]; $lastRow = $rows[-1]
    $dtMin = ($lastRow.t - $firstRow.t) / 60.0
    $dWs = $lastRow.ws - $firstRow.ws
    $slope = if ($dtMin -gt 0.1) { [math]::Round($dWs / $dtMin, 1) } else { 0.0 }
    $peak = ($rows | Measure-Object -Property ws -Maximum).Maximum
    Write-Host ("  memory       : {0} MB -> {1} MB (peak {2} MB), drift {3} MB over {4:N1} min = {5} MB/min" -f `
        $firstRow.ws, $lastRow.ws, $peak, [math]::Round($dWs, 1), $dtMin, $slope)
} else {
    Write-Host "  memory       : not enough samples"
}
if ($null -ne $fpsFirst) {
    $drift = [math]::Round(($fpsLast - $fpsFirst) / $fpsFirst * 100.0, 1)
    Write-Host ("  fps          : first third {0} -> last third {1} ({2}%)" -f $fpsFirst, $fpsLast, $drift)
} else {
    Write-Host "  fps          : no perf log parsed"
}
Write-Host ("  log          : VUID {0} ; device lost {1} ; panics {2} ; log size {3} MB" -f $vuid, $lost, $panic, $logMB)
Write-Host ("  samples csv  : {0}" -f $csv)

# Verdicts. Memory drift threshold: 2 MB/min over a >=5 minute run is small enough to be page
# noise on this machine (measured: a clean run drifts a few tenths of MB/min), while a real leak
# of a per-frame allocation shows up as tens of MB/min. fps drift threshold -10% (the repo's own
# noise floor is ~3%, lesson 35).
$ok = ($exitNote -eq "ok") -and $vuid -eq 0 -and $lost -eq 0 -and $panic -eq 0
if ($rows.Count -ge 4) {
    $ok = $ok -and ([math]::Abs($dWs / [math]::Max($dtMin, 0.1)) -lt 2.0)
}
if ($null -ne $fpsFirst) {
    $ok = $ok -and ((($fpsLast - $fpsFirst) / $fpsFirst) -gt -0.10)
}
if ($ok) { Write-Host "RESULT: ALL-OK" } else { Write-Host "RESULT: CHECK" }
