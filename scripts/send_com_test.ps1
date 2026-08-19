
$ws = New-Object -ComObject WScript.Shell
$ok = $ws.AppActivate('DeepSeek')
Write-Output ('activate: ' + $ok)
Start-Sleep -Milliseconds 1000
$ws.SendKeys('__sched2_test__')
Start-Sleep -Milliseconds 800
$ws.SendKeys('{ENTER}')
Write-Output 'sent via COM'
