
$ErrorActionPreference = "Continue"
Set-Location "D:\Rust\steel-front"
Get-Process -Name steel-front -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 1
Remove-Item -Force screenshots\steel_front_*.png, eye.log, eye.log.err -ErrorAction SilentlyContinue
Remove-Item Env:\RV3D_STRESS_AI -ErrorAction SilentlyContinue
$env:RV3D_AUTOSTART = "1"
Start-Process -FilePath "D:\Rust\steel-front\target\release\steel-front.exe" -WorkingDirectory "D:\Rust\steel-front" -RedirectStandardOutput eye.log -RedirectStandardError eye.log.err | Out-Null
Start-Sleep -Seconds 10
$wshell = New-Object -ComObject wscript.shell
$wshell.AppActivate("Steel Front") | Out-Null
Start-Sleep -Milliseconds 300
$wshell.SendKeys("{F12}")
Start-Sleep -Seconds 2
Get-Process -Name steel-front -ErrorAction SilentlyContinue | Stop-Process -Force
$shots = Get-ChildItem screenshots -Filter steel_front_*.png -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending
if ($shots) { Write-Host ("SHOT=" + $shots[0].FullName) } else { Write-Host "NO-SHOT" }
