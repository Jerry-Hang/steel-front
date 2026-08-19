@echo off
chcp 65001 >nul
setlocal enabledelayedexpansion
title Steel Front Launcher
cd /d D:\Rust\steel-front
if not defined HOME set "HOME=%USERPROFILE%"

echo ============================================
echo   Steel Front - Launcher
echo ============================================
echo.
echo   [Controls]
echo     / or Enter : command window (type 1-35 to switch weapon)
echo     B : fire mode  R : reload  G : grenade  F5 : map hot reload
echo     Death automatically refills all ammo
echo.

rem ---- 0. kill leftover game process (exe locked otherwise) ----
taskkill /f /im steel-front.exe >nul 2>&1
timeout /t 1 /nobreak >nul

rem ---- 1. pull latest code from GitHub ----
if exist .git (
    git fetch origin master >nul 2>&1
    for /f %%i in ('git rev-parse HEAD') do set LOCAL=%%i
    for /f %%i in ('git rev-parse origin/master') do set REMOTE=%%i
    if not "!LOCAL!"=="!REMOTE!" (
        echo [update] new version !REMOTE! found, pulling...
        git pull origin master >nul 2>&1
        echo [update] pulled.
    ) else (
        echo [status] code up to date: !LOCAL!
    )
)
echo.

rem ---- 2. always sync-build (incremental: seconds when nothing changed) ----
echo [sync] syncing release build...
call cargo build --release
if errorlevel 1 (
    echo [error] build failed! see messages above
    pause
    exit /b 1
)
echo.

rem ---- 3. show version and launch ----
for /f %%i in ('git rev-parse --short HEAD 2^>nul') do set VER=%%i
echo [version] %VER%
echo [launch] starting game...
start "" target\release\steel-front.exe
echo.
echo [done] game launched, press any key to close this window...
exit /b 0

