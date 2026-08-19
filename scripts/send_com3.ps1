
$ws = New-Object -ComObject WScript.Shell
foreach ($title in @('阅读文件', 'DeepSeek', 'Steel Front', '千问')) {
  $r = $ws.AppActivate($title)
  Write-Output ($title + ' -> ' + $r)
  if ($r) { break }
}
Start-Sleep -Milliseconds 1200
$ws.SendKeys('__sched4_test__')
Start-Sleep -Milliseconds 800
$ws.SendKeys('~')
Write-Output 'sent'
