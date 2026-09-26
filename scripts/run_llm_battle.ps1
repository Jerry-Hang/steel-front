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
$LOGERR = "$LOG.err"
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
Write-Host "=== battle result (last command line) ==="
# The game logs one line per command tick:
#   command: <red camp>[situation X kills N] company... | <blue camp>[situation X kills N] ...
#
# !! The field logged as "kills" is NOT kills inflicted. game.rs accumulates it as
# !! round_kills_red / round_kills_blue counting the FALLEN BY TEAM -- the source
# !! comment there calls them the camp's own losses for this round --
# !! so it is each camp's OWN death toll. The side with the BIGGER "kills" number is
# !! the side that LOST more men. Never rank the two sides by this field; an earlier
# !! version of this block did exactly that and printed the winner INVERTED.
#
# Outcome metric = total company strength (qiang du). It starts at the full roster on
# both sides (128 = 36 + 36 + 56 for red, 127 for blue because the player fills one slot;
# before 2026-09-26 the tail 20 men were missing from the company rosters so it read 108)
# and only ever declines as that side takes losses, so higher = winning.
# The Chinese markers are built from code points so this file stays ASCII.
$KILLS = -join [char[]](0x51FB, 0x6740)          # ji sha -- own deaths, see note above
$STR   = [char]0x5F3A + [char]0x5EA6             # qiang du
$last = Select-String -Path $LOGERR -Pattern 'command: ' -ErrorAction SilentlyContinue |
        Select-Object -Last 1
if ($last) {
    $m = [regex]::Matches($last.Line, "${KILLS}(\d+)")
    # strengths come in the red-company block then the blue-company block; the log
    # line separates the two camps with ' | '
    $parts = $last.Line -split '\|'
    $redS = 0; $blueS = 0
    if ($parts.Count -ge 2) {
        foreach ($x in [regex]::Matches($parts[0], "${STR}(\d+)")) { $redS += [int]$x.Groups[1].Value }
        foreach ($x in [regex]::Matches($parts[1], "${STR}(\d+)")) { $blueS += [int]$x.Groups[1].Value }
    }
    $redK = 0; $blueK = 0
    if ($m.Count -ge 2) {
        $redK = [int]$m[0].Groups[1].Value
        $blueK = [int]$m[1].Groups[1].Value
    }
    Write-Host ("  RED  strength={0,-4} own_dead={1}" -f $redS, $redK)
    Write-Host ("  BLUE strength={0,-4} own_dead={1}" -f $blueS, $blueK)
    if ($redS -gt $blueS) { $w = "RED (higher strength = fewer losses)" }
    elseif ($blueS -gt $redS) { $w = "BLUE (higher strength = fewer losses)" }
    else { $w = "DRAW" }
    Write-Host ("  => winner: {0}" -f $w)
} else {
    Write-Host "  (no command line found in $LOGERR)"
}
Write-Host ""
Write-Host "=== game: llmcmd lines ==="
Select-String -Path $LOGERR -Pattern 'llmcmd' -ErrorAction SilentlyContinue |
    Select-Object -Last 16 | ForEach-Object { $_.Line -replace '^\[([^\]]+)\]\s*\S+\s*', '$1  ' }
