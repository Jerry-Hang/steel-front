@echo off
chcp 65001 >nul
title Steel Front Launcher
cd /d D:\Rust\steel-front
if not defined HOME set "HOME=%USERPROFILE%"

echo ============================================
echo   Steel Front - 钢铁前线 启动器
echo ============================================
echo.

rem ---- 0. 先杀残留游戏进程（否则 exe 被占用，cargo build 会失败 → 永远跑旧版本）----
taskkill /f /im steel-front.exe >nul 2>&1
timeout /t 1 /nobreak >nul

rem ---- 1. 检查仓库更新 ----
if exist .git (
    git fetch origin master >nul 2>&1
    for /f %%i in ('git rev-parse HEAD') do set LOCAL=%%i
    for /f %%i in ('git rev-parse origin/master') do set REMOTE=%%i
    if not "%LOCAL%"=="%REMOTE%" (
        echo [更新] 发现新版本 %REMOTE%，正在拉取...
        git pull origin master >nul 2>&1
        echo [更新] 拉取完成，重新构建中（约1-2分钟）...
        call cargo build --release
        if errorlevel 1 (
            echo [错误] 构建失败！请查看上方错误信息
            pause
            exit /b 1
        )
    ) else (
        echo [状态] 已是最新版本 %LOCAL%
    )
)
echo.

rem ---- 2. 兜底：无 exe 时构建 ----
if not exist target\release\steel-front.exe (
    echo [构建] 首次运行，正在构建（约1-2分钟）...
    call cargo build --release
    if errorlevel 1 (
        echo [错误] 构建失败！
        pause
        exit /b 1
    )
)

rem ---- 3. 显示当前版本并启动 ----
for /f %%i in ('git rev-parse --short HEAD 2^>nul') do set VER=%%i
echo [版本] %VER%
echo [启动] 正在启动游戏...
start "" target\release\steel-front.exe
echo.
echo [完成] 游戏已启动，此窗口将自动关闭...
exit /b 0
