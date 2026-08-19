# DSH 定时消息发送脚本（计划任务调用，按当前时间自选消息）
Add-Type -AssemblyName Microsoft.VisualBasic
Add-Type -AssemblyName System.Windows.Forms
$now = Get-Date
$hm = $now.Hour * 60 + $now.Minute
if ($hm -ge 720 -and $hm -lt 800) { $msg = '开始吧' }
elseif ($hm -ge 1074 -and $hm -lt 1110) { $msg = '结束了' }
else { $msg = '开始吧' }
$proc = Get-Process | Where-Object { $_.ProcessName -eq 'msedge' -and $_.MainWindowTitle -match 'DeepSeek' } | Select-Object -First 1
if (-not $proc) { Write-Output 'ERR: edge not found'; exit 1 }
[Microsoft.VisualBasic.Interaction]::AppActivate($proc.Id)
Start-Sleep -Milliseconds 1200
[System.Windows.Forms.SendKeys]::SendWait($msg)
Start-Sleep -Milliseconds 800
[System.Windows.Forms.SendKeys]::SendWait('{ENTER}')
$logLine = (Get-Date -Format 'HH:mm:ss') + ' sent: ' + $msg
Add-Content -Path 'D:\Rust\steel-front\scripts\scheduler.log' -Value $logLine -Encoding UTF8
Write-Output $logLine
