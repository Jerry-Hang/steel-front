Add-Type -AssemblyName System.Drawing
$src = Get-ChildItem 'D:\Rust\steel-front\screenshots\steel_front_*.png' | Sort-Object LastWriteTime -Descending | Select-Object -First 1
$b = [System.Drawing.Bitmap]::FromFile($src.FullName)
$nw = 1600
$nh = [int]($b.Height * $nw / $b.Width)
$nb = New-Object System.Drawing.Bitmap($nw, $nh)
$g = [System.Drawing.Graphics]::FromImage($nb)
$g.DrawImage($b, 0, 0, $nw, $nh)
$nb.Save('D:\Rust\steel-front\screenshots\view.png', [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $nb.Dispose(); $b.Dispose()
Write-Host ok $src.Name
