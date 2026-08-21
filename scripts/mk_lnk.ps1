$ErrorActionPreference = "Stop"
$exe = "D:\Rust\steel-front\release_dist\game\steel-front.exe"
$desktop = [Environment]::GetFolderPath('Desktop')
$lnk = Join-Path $desktop '枪械检视.lnk'
$s = (New-Object -ComObject WScript.Shell).CreateShortcut($lnk)
$s.TargetPath = $exe
$s.Arguments = '--inspect 1'
$s.WorkingDirectory = 'D:\Rust\steel-front\release_dist\game'
$s.Save()
Write-Host ('created: ' + $lnk)
