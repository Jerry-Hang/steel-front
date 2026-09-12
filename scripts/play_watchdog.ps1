# play_watchdog.ps1 - heartbeat watchdog for steel-front play sessions. Pure ASCII.
#
# Why not "sleep N then kill"
# --------------------------
# The obvious watchdog (sleep N; kill; release) was armed per run and collided with
# later runs twice on 2026-09-12: a watchdog left over from an earlier run fired in the
# middle of a new one and killed the game, which then looked like "the window never
# appeared". That class of self-inflicted failure is avoided by making the watchdog
# depend on evidence of a LIVE session instead of on wall-clock time since it started.
#
# How it decides
# --------------
# A running player (pm_play.ps1) touches $Beat every couple of seconds. The watchdog
# only acts when BOTH hold:
#   * a steel-front process exists, and
#   * the heartbeat is missing or older than $StaleSec
# i.e. a game is up but nobody is driving it any more -- the pwsh that owned it died,
# hung, or was killed. Then it kills the game and runs release_input.ps1, which
# VERIFIES the handback rather than assuming it.
#
# Run it once per session as a background job; it is safe to leave running.
param(
    [int]$StaleSec = 30,
    [int]$PollSec  = 5,
    [string]$Beat  = "$env:TEMP\sf_play.beat"
)

$repo = "D:\Rust\steel-front"
Write-Host "watchdog up: stale=${StaleSec}s poll=${PollSec}s beat=$Beat"
while ($true) {
    Start-Sleep -Seconds $PollSec
    $g = Get-Process -Name steel-front -ErrorAction SilentlyContinue
    if (-not $g) { continue }

    $fresh = $false
    if (Test-Path $Beat) {
        $age = (Get-Date) - (Get-Item $Beat).LastWriteTime
        if ($age.TotalSeconds -lt $StaleSec) { $fresh = $true }
    }
    if ($fresh) { continue }

    Write-Host ("watchdog FIRING at {0}: game pid(s) {1} alive but heartbeat stale -> kill + release" -f `
        (Get-Date -Format HH:mm:ss), (($g | ForEach-Object { $_.Id }) -join ','))
    Get-Process -Name steel-front -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Milliseconds 400
    & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $repo "scripts\release_input.ps1")
    Remove-Item $Beat -ErrorAction SilentlyContinue
}
