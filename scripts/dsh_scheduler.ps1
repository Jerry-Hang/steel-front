
$logFile = 'D:\Rust\steel-front\scripts\scheduler.log'
function Log($msg) {
  $line = (Get-Date -Format 'yyyy-MM-dd HH:mm:ss') + ' ' + $msg
  Add-Content -Path $logFile -Value $line -Encoding UTF8
}
try {
  Add-Type -Path 'D:\Rust\steel-front\scripts\DSHHelper.cs'
  Log 'scheduler v3 started (C# helper loaded)'
} catch {
  Log ('FATAL: ' + $_.Exception.Message)
  exit 1
}
$sent12 = $false
$sent1354 = $false
$sent1605 = $false
while ($true) {
  $now = Get-Date
  $minutes = $now.Hour * 60 + $now.Minute
  try {
    if ($minutes -ge 720 -and -not $sent12) {
      $ok = [DSHHelper]::SendToDsh('开始吧')
      Log ('12:00 send ok=' + $ok)
      $sent12 = $true
    }
    if ($minutes -ge 1074 -and -not $sent1354) {
      $ok = [DSHHelper]::SendToDsh('结束了')
      Log ('17:54 send ok=' + $ok)
      $sent1354 = $true
    }
    if ($minutes -ge 1085 -and -not $sent1605) {
      $ok = [DSHHelper]::SendToDsh('开始吧')
      Log ('18:05 send ok=' + $ok)
      $sent1605 = $true
    }
  } catch {
    Log ('ERR: ' + $_.Exception.Message)
  }
  # 全部任务完成（16:05 已发）→ 退出，不再空转
  if ($sent12 -and $sent1354 -and $sent1605) {
    Log 'scheduler all tasks done, exiting'
    exit 0
  }
  Start-Sleep -Seconds 30
}
