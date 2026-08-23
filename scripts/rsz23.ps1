Add-Type -AssemblyName System.Drawing
$b = [System.Drawing.Bitmap]::FromFile('D:\Rust\steel-front\screenshots\2-3.png')
$nw = 1600
$nh = [int]($b.Height * $nw / $b.Width)
$nb = New-Object System.Drawing.Bitmap($nw, $nh)
$g = [System.Drawing.Graphics]::FromImage($nb)
$g.DrawImage($b, 0, 0, $nw, $nh)
$nb.Save('D:\Rust\steel-front\screenshots\2-3s.png', [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $nb.Dispose(); $b.Dispose()
Write-Host ok
