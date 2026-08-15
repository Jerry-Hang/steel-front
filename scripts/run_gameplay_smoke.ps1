# Steel Front Windows 原生冒烟启动器（替代 run_gameplay_smoke.sh）
# 用法: powershell -ExecutionPolicy Bypass -File scripts/run_gameplay_smoke.ps1
$ErrorActionPreference = "Continue"
Set-Location "$PSScriptRoot\.."
$EXE = (Join-Path (Get-Location) "target\release\steel-front.exe")
$LOG = (Join-Path (Get-Location) "smoke.log")
$LOGERR = "$LOG.err"

# 清场：杀残留游戏进程
Get-Process -Name steel-front -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 2
Remove-Item -Force $LOG, $LOGERR -ErrorAction SilentlyContinue

# 启动游戏：stdout -> smoke.log，stderr -> smoke.log.err（env_logger 走 stderr）
$p = Start-Process -FilePath $EXE -RedirectStandardOutput $LOG -RedirectStandardError $LOGERR -PassThru
Start-Sleep -Seconds 5

# 跑冒烟（SendInput 注入 + 日志断言，脚本内部合并两个日志）
python scripts\gameplay_smoke_win.py $LOG
$RC = $LASTEXITCODE

# 收尾
Get-Process -Name steel-front -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 1
Write-Host "=== log tail (last 20 lines) ==="
Get-Content $LOG -Tail 20 -ErrorAction SilentlyContinue
exit $RC