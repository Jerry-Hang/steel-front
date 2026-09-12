param(
    [int]$Secs = 34,
    [string]$Tag = "perf",
    [switch]$Stress,
    [int]$WarmupSec = 8
)
# perf_probe.ps1 - launch the game, sample GPU telemetry every second, summarise.
#
# WHY: "fps low / GPU utilisation low / power low" cannot be diagnosed by reading the
# game's own timers alone -- those say how long the CPU waited, not what the GPU was
# doing. This correlates the engine's fps counter against real hardware telemetry
# (utilisation, board power, SM clock, temperature) so a power/utilisation claim has
# evidence behind it.
#
# Mouse safety: the launch is wrapped in try/finally that ALWAYS kills the process,
# refreshes the heartbeat so the resident watchdog does not fire, and hands the cursor
# back through release_input.ps1 at the end.
$ErrorActionPreference = "Continue"
$repo = "D:\Rust\steel-front"
$exe = Join-Path $repo "target\release\steel-front.exe"
$log = Join-Path $repo "logs\$Tag.log"
$logErr = "$log.err"
$beat = "$env:TEMP\sf_play.beat"

function Stat($name, $arr, $unit) {
    if ($null -eq $arr -or $arr.Count -eq 0) { Write-Host ("  {0,-9} n/a" -f $name); return }
    $m = $arr | Measure-Object -Average -Minimum -Maximum
    Write-Host ("  {0,-9} avg={1,7:N1}{4}  min={2,7:N1}  max={3,7:N1}  n={5}" -f `
        $name, $m.Average, $m.Minimum, $m.Maximum, $unit, $arr.Count)
}

Get-Process -Name steel-front -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 1
Remove-Item -Force $log, $logErr -ErrorAction SilentlyContinue

$env:RV3D_AUTOSTART = "1"
if ($Stress) { Remove-Item Env:RV3D_STRESS_AI -ErrorAction SilentlyContinue }
else { $env:RV3D_STRESS_AI = "0" }

$samples = @()
try {
    Set-Content $beat (Get-Date -Format o) -ErrorAction SilentlyContinue
    $p = Start-Process -FilePath $exe -WorkingDirectory $repo -PassThru -WindowStyle Hidden `
        -RedirectStandardOutput $log -RedirectStandardError $logErr
    for ($i = 0; $i -lt $Secs; $i++) {
        Set-Content $beat (Get-Date -Format o) -ErrorAction SilentlyContinue
        Start-Sleep -Seconds 1
        if ($i -lt $WarmupSec) { continue }
        $s = & nvidia-smi --query-gpu=utilization.gpu,power.draw,clocks.sm,temperature.gpu --format=csv,noheader,nounits 2>$null
        if ($s) { $samples += ($s | Select-Object -First 1).Trim() }
    }
}
finally {
    Get-Process -Name steel-front -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Seconds 1
    Remove-Item Env:RV3D_AUTOSTART, Env:RV3D_STRESS_AI -ErrorAction SilentlyContinue
    Remove-Item $beat -ErrorAction SilentlyContinue
    & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $repo "scripts\release_input.ps1") -Quiet
}

$util = @(); $pwr = @(); $clk = @(); $tmp = @()
foreach ($s in $samples) {
    $f = $s -split ','
    if ($f.Count -ge 4) {
        $util += [double]$f[0].Trim()
        $pwr += [double]$f[1].Trim()
        $clk += [double]$f[2].Trim()
        $tmp += [double]$f[3].Trim()
    }
}

Write-Host ("=== GPU telemetry [{0}]  stress={1} ===" -f $Tag, [bool]$Stress)
Stat "gpu_util" $util " %"
Stat "power" $pwr " W"
Stat "sm_clock" $clk " MHz"
Stat "temp" $tmp " C"

$t = @()
if (Test-Path $logErr) { $t = [System.IO.File]::ReadAllLines($logErr, [System.Text.Encoding]::UTF8) }
$fps = @(); $wfc = @(); $cyc = @(); $ai = @()
foreach ($l in $t) {
    $m = [regex]::Match($l, 'fps=([0-9.]+)'); if ($m.Success) { $fps += [double]$m.Groups[1].Value }
    $m = [regex]::Match($l, 'wait_fence_us=([0-9]+)'); if ($m.Success) { $wfc += [double]$m.Groups[1].Value }
    $m = [regex]::Match($l, 'cycle_us=([0-9]+)'); if ($m.Success) { $cyc += [double]$m.Groups[1].Value }
    $m = [regex]::Match($l, 'ai_us=([0-9]+)'); if ($m.Success) { $ai += [double]$m.Groups[1].Value }
}
if ($fps.Count -gt $WarmupSec) { $fps = $fps[$WarmupSec..($fps.Count-1)] }
if ($wfc.Count -gt $WarmupSec) { $wfc = $wfc[$WarmupSec..($wfc.Count-1)] }
if ($cyc.Count -gt $WarmupSec) { $cyc = $cyc[$WarmupSec..($cyc.Count-1)] }
if ($ai.Count  -gt $WarmupSec) { $ai  = $ai[$WarmupSec..($ai.Count-1)] }
Write-Host "=== engine timers (warmup skipped) ==="
Stat "fps" $fps ""
Stat "wait_fence" $wfc " us"
Stat "cycle" $cyc " us"
Stat "ai_us" $ai " us"
