param(
    [Parameter(Mandatory = $true)][string]$A,
    [Parameter(Mandatory = $true)][string]$B,
    [int]$Grid = 4,
    [int]$Threshold = 8
)

# Per-cell pixel diff of two screenshots: answers "which part of the frame changed"
# objectively, instead of eyeballing two nearly identical images.
#
# This file is PURE ASCII on purpose. Windows PowerShell 5.1 decodes a BOM-less .ps1
# with the system ANSI codepage (GBK here), and UTF-8 Chinese bytes then break it in
# two ways: a line ending in Chinese eats its own newline (the following statement is
# swallowed into the comment -- this file used to lose $ErrorActionPreference, $src and
# $step that way), and Chinese inside a "quoted string" can eat the closing quote.
# Guard = powershell_scripts_never_end_a_line_with_a_non_ascii_byte (now scans tools/).
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing

function Load([string]$p) {
    if (-not (Test-Path $p)) { throw "missing image: $p" }
    # Open from a memory copy so GDI+ does not keep a file handle.
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
        Write-Host "sizes differ -- rescaling B to A first"
        $bb = New-Object System.Drawing.Bitmap($bb, $ba.Width, $ba.Height)
    }

    $w = $ba.Width; $h = $ba.Height
    # Floor is required: PowerShell's [int] rounds, so 1600/6 = 266.67 becomes 267 and
    # 6 cells would add up to 1602, past the image height. Grid = 4 divides evenly,
    # which is why this was never noticed.
    $cw = [int][Math]::Floor($w / $Grid); $ch = [int][Math]::Floor($h / $Grid)
    # Sample inside each cell: walking all 2560x1600 pixels in PS is far too slow.
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

    Write-Host ("`nwhole-frame mean diff: {0:N2} / 255   (sampled {1} points)" -f ($total / $totalN), $totalN)
    Write-Host "`nper-cell mean diff (rows top->bottom, cols left->right; > $Threshold = significant):"
    for ($r = 0; $r -lt $Grid; $r++) {
        $line = ($cells | Where-Object { $_.row -eq $r } | Sort-Object col | ForEach-Object {
            if ($_.mean -gt $Threshold) { ("[{0,6:N2}]*" -f $_.mean) } else { ("[{0,6:N2}] " -f $_.mean) }
        }) -join ""
        Write-Host ("  row {0}: {1}" -f $r, $line)
    }
    $sig = @($cells | Where-Object { $_.mean -gt $Threshold })
    Write-Host ("`nsignificant cells: {0} / {1}" -f $sig.Count, $cells.Count)
    if ($sig.Count -eq 0) {
        Write-Host "=> the two frames agree within sampling error: this change did not alter the image."
    } else {
        Write-Host ("=> changes concentrate in: " + (($sig | ForEach-Object { ('r{0}c{1}' -f $_.row, $_.col) }) -join ', '))
    }
}
finally {
    $ba.Dispose(); $bb.Dispose()
}
