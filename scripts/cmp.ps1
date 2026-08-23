$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
$ref = [System.Drawing.Bitmap]::FromFile('D:\Rust\steel-front\screenshots\AK-12M.png')
$gun = [System.Drawing.Bitmap]::FromFile('D:\Rust\steel-front\screenshots\gun_crop.png')
$W = 1051
$H = 429 + 490 + 30
$canvas = New-Object System.Drawing.Bitmap($W, $H)
$g2 = [System.Drawing.Graphics]::FromImage($canvas)
$g2.Clear([System.Drawing.Color]::White)
$g2.DrawImage($ref, 0, 0, 1051, 429)
$g2.DrawImage($gun, 0, 459, $W, 490)
$g2.Dispose()
$canvas.Save('D:\Rust\steel-front\screenshots\compare.png', [System.Drawing.Imaging.ImageFormat]::Png)
$canvas.Dispose()
Write-Host ok
