# run_survive_pm.ps1 - drive the `survive` rule map end to end. Pure ASCII on purpose.
#
# Unresolved item #17: the 5-wave survive match had never been played to its victory
# state on the real build. This launches the defense_line map (which declares
# [rule] kind = "survive" waves = 5) and hands the match to scripts/survive_pm.py.
#
# RV3D_INVINCIBLE=1 is deliberate: it removes the "player dies at wave N" branch so the
# run can reach wave 5 at all. The defeat branch is covered by unit tests.
#
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File scripts\run_survive_pm.ps1 [-Secs 780]
param([int]$Secs = 780, [int]$ShotEvery = 90)
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
$env:RV3D_INVINCIBLE  = "1"

Set-Content -Path $beat -Value (Get-Date -Format o) -ErrorAction SilentlyContinue
$rc = 1
try {
    Start-Process -FilePath $exe -WorkingDirectory $repo -WindowStyle Hidden `
        -RedirectStandardOutput $LOG -RedirectStandardError $LOGERR | Out-Null
    Write-Host "launched RV3D_MAP=$env:RV3D_MAP ; waiting 10s"
    for ($i = 0; $i -lt 10; $i++) { Set-Content $beat (Get-Date -Format o); Start-Sleep -Seconds 1 }

    python (Join-Path $repo "scripts\survive_pm.py") $LOG --secs $Secs --shot-every $ShotEvery
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
