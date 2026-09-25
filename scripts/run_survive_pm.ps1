# run_survive_pm.ps1 - drive the `survive` rule map end to end. Pure ASCII on purpose.
#
# Unresolved item #17: the 5-wave survive match had never been played to its victory
# state on the real build. This launches the defense_line map (which declares
# [rule] kind = "survive" waves = 5) and hands the match to scripts/survive_pm.py.
#
# RV3D_INVINCIBLE=1 is deliberate: it removes the "player dies at wave N" branch so the
# run can reach wave 5 at all. The defeat branch is covered by unit tests.
#
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File scripts\run_survive_pm.ps1 [-Secs 780] [-NoShot]
# -NoShot skips every PrintWindow capture: on the discrete GPU the combination of
# "Playing + capture" reproducibly loses the Vulkan device (2026-09-25), while the
# same scene without capture runs fine. Use it for dGPU runs; screenshots still work
# on the integrated GPU.
param([int]$Secs = 780, [int]$ShotEvery = 90, [switch]$NoShot, [string]$PresentMode = "mailbox", [switch]$NoInvincible)
$ErrorActionPreference = "Continue"
$repo = "D:\Rust\steel-front"
$exe  = Join-Path $repo "target\release\steel-front.exe"
$LOG  = Join-Path $repo "logs\survive_pm.log"
$LOGERR = "$LOG.err"
$beat = "$env:TEMP\sf_play.beat"

Get-Process -Name steel-front -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 2
Remove-Item -Force $LOG, $LOGERR -ErrorAction SilentlyContinue

$env:RV3D_STRESS_AI  = "0"
$env:RV3D_AUTOSTART   = "1"
$env:RV3D_MAP         = "assets/maps/defense_line.toml"
# RV3D_INVINCIBLE=1 is deliberate: it removes the "player dies at wave N" branch so the
# run can reach wave 5 at all. -NoInvincible flips it OFF to exercise the Defeat branch
# on real hardware (which otherwise only has unit-test coverage).
if ($NoInvincible) { $env:RV3D_INVINCIBLE = "0" } else { $env:RV3D_INVINCIBLE = "1" }
# Machine-readable live NPC positions (one `npcpos:` line per NPC per second).
# The harness aims by it: `npc: #N stand` is only a snapshot from the moment an NPC
# entered Attack, so moving targets were aimed at a stale point (12 shots/kill).
$env:RV3D_NPC_POS     = "1"
# !! 2026-09-25: the discrete GPU **hangs** (Windows TDR 0x141 VIDEO_ENGINE_TIMEOUT_DETECTED,
# 4 events in the Application log) when this map is played with the engine default
# IMMEDIATE present mode: the game logged `game: run started (wave 1)` and then died at the
# first Playing frame (no fps line, no panic, no VUID). The same map on the integrated GPU,
# and the city map on the discrete GPU, are both fine. SteelFront.bat already plays with
# mailbox (AGENTS.md, iron rule B) -- this harness now matches the player path instead of
# the benchmark default. `-PresentMode immediate` exists to re-test that finding.
$env:RV3D_PRESENT_MODE = $PresentMode

Set-Content -Path $beat -Value (Get-Date -Format o) -ErrorAction SilentlyContinue
$rc = 1
try {
    Start-Process -FilePath $exe -WorkingDirectory $repo -WindowStyle Hidden `
        -RedirectStandardOutput $LOG -RedirectStandardError $LOGERR | Out-Null
    Write-Host "launched RV3D_MAP=$env:RV3D_MAP ; waiting 10s"
    for ($i = 0; $i -lt 10; $i++) { Set-Content $beat (Get-Date -Format o); Start-Sleep -Seconds 1 }

    $pyargs = @((Join-Path $repo "scripts\survive_pm.py"), $LOG, "--secs", "$Secs", "--shot-every", "$ShotEvery")
    if ($NoShot) { $pyargs += "--no-shot" }
    python @pyargs
    $rc = $LASTEXITCODE
}
finally {
    Get-Process -Name steel-front -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Seconds 1
    Remove-Item Env:RV3D_STRESS_AI, Env:RV3D_AUTOSTART, Env:RV3D_MAP, Env:RV3D_INVINCIBLE -ErrorAction SilentlyContinue
    Remove-Item $beat -ErrorAction SilentlyContinue
    & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $repo "scripts\release_input.ps1") -Quiet
}
exit $rc
