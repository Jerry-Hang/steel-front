# 本地启动器更新：重新打包 release_dist -> 桌面快捷方式 -> 删旧 BAT
$ErrorActionPreference = "Stop"
Set-Location "D:\Rust\steel-front"

Write-Host "[1/3] packaging launcher + game..."
powershell -ExecutionPolicy Bypass -File scripts\make_release.ps1 | Out-Null

Write-Host "[2/3] creating desktop shortcut..."
$exe = "D:\Rust\steel-front\release_dist\SteelFrontLauncher.exe"
$work = "D:\Rust\steel-front\release_dist"
$desktop = [Environment]::GetFolderPath('Desktop')
$lnk = Join-Path $desktop '钢铁前线.lnk'
$ps = "`$s=(New-Object -ComObject WScript.Shell).CreateShortcut('$lnk');`$s.TargetPath='$exe';`$s.WorkingDirectory='$work';`$s.Save()"
powershell -NoProfile -Command $ps | Out-Null
Write-Host "shortcut: $lnk"

Write-Host "[3/3] removing old BAT..."
Remove-Item -Force "D:\Rust\steel-front\SteelFront.bat" -ErrorAction SilentlyContinue
git rm --cached SteelFront.bat 2>&1 | Out-Null
Write-Host "DONE"