param([int]$Seconds = 22)

# PT 开/关 A/B 实测 GPU 功耗·利用率·显存·帧率（回答"全景路径追踪到底有没有在跑"）
function Run-Case([string]$name, [string]$ptLive, [string]$spp, [string]$size = '512') {
    # Start-Job 默认工作目录是用户主目录，相对路径会把 CSV 写到别处 -> 必须绝对路径
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
        "{0,-7} n={1}  功耗 avg={2:F1}W max={3:F1}W | 利用率 avg={4:F0}% | 显存 avg={5:F0}MiB" -f `
            $name, $rows.Count, `
            (($rows | Measure-Object W -Average).Average), (($rows | Measure-Object W -Maximum).Maximum), `
            (($rows | Measure-Object U -Average).Average), (($rows | Measure-Object M -Average).Average)
    } else { "$name 无 nvidia-smi 采样" }

    $f = Select-String -Path $log -Pattern 'fps=([0-9\.]+)' -ErrorAction SilentlyContinue | ForEach-Object { [double]$_.Matches[0].Groups[1].Value }
    if ($f) { "        游戏 fps avg={0:F1} max={1:F1}（n={2}）" -f (($f | Measure-Object -Average).Average), (($f | Measure-Object -Maximum).Maximum), $f.Count }
    Select-String -Path $log -Pattern 'PT-RESIDENT|RT: 路径追踪' -ErrorAction SilentlyContinue | ForEach-Object { "        " + $_.Line }
}

Run-Case 'ptoff' '0' '256' '512'
Start-Sleep -Seconds 3
Run-Case 'pton512' '1' '4096' '512'
Start-Sleep -Seconds 3
# 可证伪对照：PT 若真在算，像素 ×9 必然掉帧抬功耗；若纹丝不动则说明通道是假的
Run-Case 'pton1536' '1' '4096' '1536'
