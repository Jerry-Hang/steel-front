$ErrorActionPreference = "Stop"
Set-Location "D:\Rust\steel-front"
powershell -ExecutionPolicy Bypass -File scripts\make_release.ps1 | Out-Null
$exe = "D:\Rust\steel-front\release_dist\game\steel-front.exe"
$desktop = [Environment]::GetFolderPath('Desktop')
$lnk = Join-Path $desktop '枪械检视.lnk'
$target = "`$s=(New-Object -ComObject WScript.Shell).CreateShortcut('$lnk');`$s.TargetPath='$exe';`$s.Arguments='--inspect 1';`$s.WorkingDirectory='D:\\Rust\\steel-front\\release_dist\\game';`$s.Save()"
powershell -NoProfile -Command $target | Out-Null
Write-Host ('inspect shortcut done')