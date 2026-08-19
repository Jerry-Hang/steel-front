
Add-Type -AssemblyName Microsoft.VisualBasic
Add-Type -AssemblyName System.Windows.Forms
$ok = [Microsoft.VisualBasic.Interaction]::AppActivate(27584)
Write-Output ('vb activate pid: ' + $ok)
if (-not $ok) {
  $ok2 = [Microsoft.VisualBasic.Interaction]::AppActivate('阅读文件了解项目状态')
  Write-Output ('vb activate title: ' + $ok2)
}
Start-Sleep -Milliseconds 1200
[System.Windows.Forms.SendKeys]::SendWait('__sched5_test__')
Start-Sleep -Milliseconds 800
[System.Windows.Forms.SendKeys]::SendWait('{ENTER}')
Write-Output 'sent v5'
