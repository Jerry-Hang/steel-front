@echo off
chcp 65001 >nul
setlocal
set /p HOST=Host address (ip:port, e.g. 192.168.1.10:27015): 
cd /d "%~dp0"
set RV3D_NET=client
set RV3D_NET_ADDR=%HOST%
set RV3D_AUTOSTART=1
start "" steel-front.exe
echo.
echo Join request sent to %HOST%
echo If the host is behind NAT, both sides must also set:
echo   RV3D_NET_RDV=relay_ip:27020   RV3D_NET_NAME=roomname
echo (see README.md, section "Multiplayer").
echo.
pause
