param([int]$Seconds = 22)

# PT on/off A/B: measures GPU power, utilisation, VRAM and fps, to answer the question
# "is the full-scene path tracer actually running?".
#
# ASCII ONLY (2026-09-26): this file used to have Chinese comments, and Windows PowerShell 5.1
# reads a BOM-less script as ANSI -- the trailing character of line 3 was decoded as a GBK lead
# byte whose trail byte became the line ending, so `function Run-Case(...)` on the next line was
# glued onto the comment and silently commented out. The script could never have worked: every
# Run-Case call at the bottom would fail with "not recognized". Same for line 5 ($csv) and the
# last case (pton1536). See docs/PROGRESS.md 2026-09-26.
function Run-Case([string]$name, [string]$ptLive, [string]$spp, [string]$size = '512') {
    # Start-Job runs in the user's home directory, so a relative path would write the CSV
    # somewhere else -- use an absolute path.
    $csv = Join-Path (Get-Location).Path "data\gpu_$name.csv"
    Remove-Item $csv -ErrorAction SilentlyContinue
    $job = Start-Job -ArgumentList $csv -ScriptBlock {
        param($csv)
        & nvidia-smi --query-gpu=timestamp,power.draw,utilization.gpu,memory.used,clocks.sm --format=csv,noheader -l 1 *> $csv
    }
    Start-Sleep -Seconds 2
    $env:RV3D_AUTOSTART = '1'
    $env:RV3D_STRESS_AI = '1'
    $env:RV3D_PT_LIVE  = $ptLive
    $env:RV3D_PT_SPP   = $spp
    $env:RV3D_PT_SIZE  = $size
    $log = "data\run_$name.log"
    $p = Start-Process -FilePath 'target\release\steel-front.exe' -RedirectStandardError $log -PassThru -WindowStyle Hidden
    Start-Sleep -Seconds $Seconds
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 2
    Stop-Job $job -ErrorAction SilentlyContinue
    Remove-Job $job -Force -ErrorAction SilentlyContinue

    $rows = Get-Content $csv -ErrorAction SilentlyContinue | Where-Object { $_ -match ',' } | ForEach-Object {
        $c = $_ -split ','
        if ($c.Count -lt 4) { return }
        $w = ($c[1] -replace '[^0-9\.]', '')
        $u = ($c[2] -replace '[^0-9]', '')
        $m = ($c[3] -replace '[^0-9\.]', '')
        if (-not $w) { return }
        [pscustomobject]@{ W = [double]$w; U = [int]$u; M = [double]$m }
    }
    if ($rows) {
        "{0,-7} n={1}  power avg={2:F1}W max={3:F1}W | util avg={4:F0}% | vram avg={5:F0}MiB" -f `
            $name, $rows.Count, `
            (($rows | Measure-Object W -Average).Average), (($rows | Measure-Object W -Maximum).Maximum), `
            (($rows | Measure-Object U -Average).Average), (($rows | Measure-Object M -Average).Average)
    } else { "$name : no nvidia-smi samples" }

    $f = Select-String -Path $log -Pattern 'fps=([0-9\.]+)' -ErrorAction SilentlyContinue | ForEach-Object { [double]$_.Matches[0].Groups[1].Value }
    if ($f) { "        game fps avg={0:F1} max={1:F1} (n={2})" -f (($f | Measure-Object -Average).Average), (($f | Measure-Object -Maximum).Maximum), $f.Count }
    # Match on the ASCII prefix only: the engine's own line is "RT: ... = on/off" (UTF-8 log),
    # and a Chinese pattern inside an ANSI-read script would be mojibake and never match anyway.
    Select-String -Path $log -Pattern 'PT-RESIDENT|RT: ' -ErrorAction SilentlyContinue | ForEach-Object { "        " + $_.Line }
}

Run-Case 'ptoff' '0' '256' '512'
Start-Sleep -Seconds 3
Run-Case 'pton512' '1' '4096' '512'
Start-Sleep -Seconds 3
# Falsifiable contrast: if PT really computes, 9x the pixels must cost fps and raise power;
# if both stay flat, the PT channel is not doing anything.
Run-Case 'pton1536' '1' '4096' '1536'
