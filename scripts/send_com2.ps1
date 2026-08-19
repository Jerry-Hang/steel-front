
$ws = New-Object -ComObject WScript.Shell
$ok = $ws.AppActivate(27584)
Write-Output ('activate pid: ' + $ok)
Start-Sleep -Milliseconds 1200
$ws.SendKeys('__sched3_test__')
Start-Sleep -Milliseconds 800
$ws.SendKeys('{ENTER}')
Write-Output 'sent'
