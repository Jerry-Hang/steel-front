# run_smoke_pm.ps1 - PostMessage-based smoke launcher. Pure ASCII on purpose.
#
# Differences from run_gameplay_smoke.ps1, and only these two:
#   1. Injection goes through PostMessage (gameplay_smoke_pm.py), not SendInput.
#      Measured 2026-09-12: SendInput delivers 0 of its events to this game, which is
#      why the smoke gate has been red with kills=0.
#   2. It touches the heartbeat file so the resident scripts/play_watchdog.ps1 covers
#      it, and releases the claim on the way out.
#
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File scripts\run_smoke_pm.ps1
$ErrorActionPreference = "Continue"
$repo = "D:\Rust\steel-front"
$exe  = Join-Path $repo "target\release\steel-front.exe"
$LOG  = Join-Path $repo "logs\smoke_pm.log"
$LOGERR = "$LOG.err"
$beat = "$env:TEMP\sf_play.beat"

Get-Process -Name steel-front -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 2
Remove-Item -Force $LOG, $LOGERR -ErrorAction SilentlyContinue

# Wave mode: in stress mode the player is an invincible spectator, so the kill
# criterion cannot be satisfied by construction.
$env:RV3D_STRESS_AI = "0"
$env:RV3D_AUTOSTART  = "1"

Set-Content -Path $beat -Value (Get-Date -Format o) -ErrorAction SilentlyContinue
$rc = 1
try {
    Start-Process -FilePath $exe -WorkingDirectory $repo -WindowStyle Hidden `
        -RedirectStandardOutput $LOG -RedirectStandardError $LOGERR | Out-Null
    Write-Host "launched; waiting 8s"
    for ($i = 0; $i -lt 8; $i++) { Set-Content $beat (Get-Date -Format o); Start-Sleep -Seconds 1 }

    python (Join-Path $repo "scripts\gameplay_smoke_pm.py") $LOG
    $rc = $LASTEXITCODE
}
finally {
    Get-Process -Name steel-front -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Seconds 1
    Remove-Item Env:RV3D_STRESS_AI, Env:RV3D_AUTOSTART -ErrorAction SilentlyContinue
    Remove-Item $beat -ErrorAction SilentlyContinue
    & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $repo "scripts\release_input.ps1") -Quiet
}
exit $rc
