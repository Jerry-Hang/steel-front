@echo off
chcp 65001 >nul
setlocal
cd /d "%~dp0"
set RV3D_NET=server
set RV3D_NET_ADDR=0.0.0.0:27015
set RV3D_AUTOSTART=1
start "" steel-front.exe
echo.
echo Steel Front host started on UDP 27015 (all interfaces).
echo Friends on the LAN: run the join bat and enter YOUR_LAN_IP:27015
echo Friends over the internet: use rdv.exe relay + RV3D_NET_RDV/RV3D_NET_NAME
echo   (see README.md, section "Multiplayer").
echo.
pause
