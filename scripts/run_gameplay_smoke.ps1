# Steel Front Windows 原生冒烟启动器（替代 run_gameplay_smoke.sh）
# 用法: powershell -ExecutionPolicy Bypass -File scripts/run_gameplay_smoke.ps1
$ErrorActionPreference = "Continue"
Set-Location "$PSScriptRoot\.."
$ROOT = (Get-Location).Path
$EXE = (Join-Path $ROOT "target\release\steel-front.exe")
$LOG = (Join-Path $ROOT "smoke.log")
$LOGERR = "$LOG.err"

# 清场：杀残留游戏进程
Get-Process -Name steel-front -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 2
Remove-Item -Force $LOG, $LOGERR -ErrorAction SilentlyContinue

# 启动游戏（-WorkingDirectory 必须显式指定：-File 模式下 Set-Location 不改子进程 cwd，
# 否则游戏找不到 assets/*.spv 会渲染器初始化失败退出）
Start-Process -FilePath $EXE -WorkingDirectory $ROOT -RedirectStandardOutput $LOG -RedirectStandardError $LOGERR -PassThru | Out-Null
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