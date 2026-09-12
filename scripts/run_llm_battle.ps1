# run_llm_battle.ps1 - live test of the RV3D_LLM tactical command channel. Pure ASCII.
#
# Starts scripts/llm_commander.py (OpenAI-compatible endpoint on 127.0.0.1:8080) and the
# game with RV3D_LLM=1, so the game's commander thread POSTs each side's battle situation
# and applies whatever comes back. Success = the game log shows an adopted-orders line for
# red/blue, i.e. parse_company_cmds() accepted them (src/llm_cmd.rs:454).
#
# Stress mode (RV3D_STRESS_AI unset -> 128) is what we WANT here: the LLM channel only has
# anything to command when two 128-strong armies exist. The usual "pin RV3D_STRESS_AI=0"
# advice is for kill-based smoke tests, not this one.
#
# NOTE ON ASCII: this file must stay ASCII-only. Windows PowerShell 5.1 reads a BOM-less
# .ps1 as ANSI, and non-ASCII inside it can decode into stray quote characters and break
# parsing (this file was rewritten for exactly that reason). The Chinese markers we need to
# match in the game log are therefore built from code points at runtime, not written here.
#
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File scripts\run_llm_battle.ps1
param(
    [int]$Secs = 150,
    [int]$Interval = 20,
    [string]$Tag = "llmbattle"
)
$ErrorActionPreference = "Continue"
$repo = "D:\Rust\steel-front"
$exe  = Join-Path $repo "target\release\steel-front.exe"
$LOG  = Join-Path $repo "logs\$Tag.log"
$LOGERR = "$LOG.log.err"
$beat = "$env:TEMP\sf_play.beat"

# Adopted-order marker: U+547D U+4EE4 U+5DF2 U+91C7 U+7EB3  (ming ling yi cai na)
$ADOPTED = -join [char[]](0x547D, 0x4EE4, 0x5DF2, 0x91C7, 0x7EB3)

Get-Process -Name steel-front -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 2
Remove-Item -Force $LOG, $LOGERR -ErrorAction SilentlyContinue
$srvLog = "$env:TEMP\llm_commander.out"

$env:RV3D_AUTOSTART    = "1"
$env:RV3D_LLM          = "1"
$env:RV3D_LLM_INTERVAL = "$Interval"
Remove-Item Env:RV3D_STRESS_AI -ErrorAction SilentlyContinue

Set-Content -Path $beat -Value (Get-Date -Format o) -ErrorAction SilentlyContinue
$srv = $null
try {
    $srv = Start-Process -FilePath "python" `
        -ArgumentList @((Join-Path $repo "scripts\llm_commander.py")) `
        -WorkingDirectory $repo -PassThru -WindowStyle Hidden `
        -RedirectStandardOutput $srvLog -RedirectStandardError "$srvLog.err"
    Start-Sleep -Seconds 3
    if ($srv.HasExited) { throw "commander server exited immediately (see $srvLog.err)" }
    Write-Host "commander server up (pid $($srv.Id)) on 127.0.0.1:8080"

    Start-Process -FilePath $exe -WorkingDirectory $repo -WindowStyle Hidden `
        -RedirectStandardOutput $LOG -RedirectStandardError $LOGERR | Out-Null
    Write-Host "game launched; RV3D_LLM_INTERVAL=${Interval}s, running ${Secs}s"

    for ($i = 0; $i -lt $Secs; $i++) {
        Set-Content $beat (Get-Date -Format o)
        Start-Sleep -Seconds 1
        if ($i % 20 -eq 19) {
            $ok = (Select-String -Path $LOGERR -Pattern $ADOPTED -ErrorAction SilentlyContinue | Measure-Object).Count
            Write-Host ("  t+{0,3}s  adopted orders: {1}" -f ($i + 1), $ok)
        }
    }
}
finally {
    Get-Process -Name steel-front -ErrorAction SilentlyContinue | Stop-Process -Force
    if ($srv -and -not $srv.HasExited) { Stop-Process -Id $srv.Id -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Seconds 1
    Remove-Item Env:RV3D_AUTOSTART, Env:RV3D_LLM, Env:RV3D_LLM_INTERVAL -ErrorAction SilentlyContinue
    Remove-Item $beat -ErrorAction SilentlyContinue
    & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $repo "scripts\release_input.ps1") -Quiet
}

Write-Host ""
Write-Host "=== commander server log (last 12) ==="
Get-Content $srvLog -Tail 12 -ErrorAction SilentlyContinue
Write-Host ""
Write-Host "=== game: llmcmd lines ==="
Select-String -Path $LOGERR -Pattern 'llmcmd' -ErrorAction SilentlyContinue |
    Select-Object -Last 16 | ForEach-Object { $_.Line -replace '^\[([^\]]+)\]\s*\S+\s*', '$1  ' }
