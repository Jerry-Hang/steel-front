# Steel Front Windows-native smoke launcher (replaces run_gameplay_smoke.sh).
# Usage: powershell -ExecutionPolicy Bypass -File scripts/run_gameplay_smoke.ps1
#
# DEPRECATED: this one drives the game with SendInput, which structurally cannot work on this
# machine (input goes to whatever window owns the foreground). Use scripts\run_smoke_pm.ps1
# (PostMessage injection) instead -- see AGENTS.md, section 3. Kept only as a historical record.
#
# ASCII ONLY (2026-09-26): with Chinese comments, Windows PowerShell 5.1 read this file as ANSI
# and several trailing characters ate their line ending, gluing the NEXT line onto the comment.
# That silently commented out real code here: the pre-run `Stop-Process`, `$env:RV3D_STRESS_AI`,
# the `Start-Process` that launches the game and the `python ...` smoke call itself.
# See docs/PROGRESS.md 2026-09-26.
$ErrorActionPreference = "Continue"
Set-Location "$PSScriptRoot\.."
$ROOT = (Get-Location).Path
$EXE = (Join-Path $ROOT "target\release\steel-front.exe")
$LOG = (Join-Path $ROOT "smoke.log")
$LOGERR = "$LOG.err"

# Clean slate: kill any leftover game process.
Get-Process -Name steel-front -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 2
Remove-Item -Force $LOG, $LOGERR -ErrorAction SilentlyContinue

# Smoke uses fixed-wave mode: NPCs sit inside the central safe ring (no obstacle occlusion),
# which keeps the injected aim/kill chain deterministic. The default large battle
# (red 64 vs blue 63 + player) is covered by scripts/switch_smoke.ps1 and manual runs.
$env:RV3D_STRESS_AI = "0"
# Launch the game (-WorkingDirectory must be explicit: under -File, Set-Location does not change
# the child process cwd, and the game would fail to find assets/*.spv and exit during renderer init).
Start-Process -FilePath $EXE -WorkingDirectory $ROOT -RedirectStandardOutput $LOG -RedirectStandardError $LOGERR -PassThru | Out-Null
Start-Sleep -Seconds 8

# Run the smoke (SendInput injection + log assertions; the script merges both logs).
python scripts\gameplay_smoke_win.py $LOG
$RC = $LASTEXITCODE

# Teardown.
Get-Process -Name steel-front -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 1
Write-Host "=== log tail (last 20 lines) ==="
Get-Content $LOG -Tail 20 -ErrorAction SilentlyContinue
exit $RC
