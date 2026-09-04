param(
    [Parameter(Mandatory = $true)][string]$A,
    [Parameter(Mandatory = $true)][string]$B,
    [int]$Grid = 4,
    [int]$Threshold = 8
)

# 两张截图的分区像素差分：给出"改动落在画面哪一块"的客观答案，
# 避免只靠肉眼在两张几乎一样的图里找不同。
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing

function Load([string]$p) {
    if (-not (Test-Path $p)) { throw "找不到图片: $p" }
    # 用内存副本打开，避免 GDI+ 锁住文件句柄
    $src = [System.IO.File]::ReadAllBytes($p)
    $ms = New-Object System.IO.MemoryStream(, $src)
    return [System.Drawing.Bitmap]::FromStream($ms)
}

$ba = Load $A
$bb = Load $B
try {
    Write-Host ("A: {0}  {1}x{2}" -f $A, $ba.Width, $ba.Height)
    Write-Host ("B: {0}  {1}x{2}" -f $B, $bb.Width, $bb.Height)
    if ($ba.Width -ne $bb.Width -or $ba.Height -ne $bb.Height) {
        Write-Host "尺寸不同，先把 B 缩放到 A 的尺寸再比"
        $bb = New-Object System.Drawing.Bitmap($bb, $ba.Width, $ba.Height)
    }

    $w = $ba.Width; $h = $ba.Height
    # 必须 Floor：PowerShell 的 [int] 是四舍五入，1600/6=266.67 会变成 267，
    # 6 格就累加到 1602 越出图片高度（Grid 取 4 恰好整除所以没暴露）。
    $cw = [int][Math]::Floor($w / $Grid); $ch = [int][Math]::Floor($h / $Grid)
    # 每格按步长抽样，全像素遍历 2560x1600 在 PS 里太慢
    $step = 7
    $total = 0.0; $totalN = 0
    $cells = @()
    for ($gy = 0; $gy -lt $Grid; $gy++) {
        for ($gx = 0; $gx -lt $Grid; $gx++) {
            $sum = 0.0; $n = 0; $maxd = 0
            for ($y = $gy * $ch; $y -lt ($gy + 1) * $ch; $y += $step) {
                for ($x = $gx * $cw; $x -lt ($gx + 1) * $cw; $x += $step) {
                    $pa = $ba.GetPixel($x, $y); $pb = $bb.GetPixel($x, $y)
                    $d = [Math]::Abs($pa.R - $pb.R) + [Math]::Abs($pa.G - $pb.G) + [Math]::Abs($pa.B - $pb.B)
                    $d = $d / 3.0
                    $sum += $d; $n++
                    if ($d -gt $maxd) { $maxd = $d }
                }
            }
            $m = if ($n) { $sum / $n } else { 0 }
            $cells += [pscustomobject]@{
                row = $gy; col = $gx; mean = [math]::Round($m, 2); max = [math]::Round($maxd, 1)
            }
            $total += $sum; $totalN += $n
        }
    }

    Write-Host ("`n整图平均差值: {0:N2} / 255   (抽样 {1} 点)" -f ($total / $totalN), $totalN)
    Write-Host "`n分区平均差值（行=上→下，列=左→右；> $Threshold 视为显著变化）:"
    for ($r = 0; $r -lt $Grid; $r++) {
        $line = ($cells | Where-Object { $_.row -eq $r } | Sort-Object col | ForEach-Object {
            if ($_.mean -gt $Threshold) { ("[{0,6:N2}]*" -f $_.mean) } else { ("[{0,6:N2}] " -f $_.mean) }
        }) -join ""
        Write-Host ("  row {0}: {1}" -f $r, $line)
    }
    $sig = @($cells | Where-Object { $_.mean -gt $Threshold })
    Write-Host ("`n显著变化格数: {0} / {1}" -f $sig.Count, $cells.Count)
    if ($sig.Count -eq 0) {
        Write-Host "=> 两张图在抽样精度内一致：这次改动没有影响画面。"
    } else {
        Write-Host ("=> 变化集中在: " + (($sig | ForEach-Object { ('r{0}c{1}' -f $_.row, $_.col) }) -join ', '))
    }
}
finally {
    $ba.Dispose(); $bb.Dispose()
}
