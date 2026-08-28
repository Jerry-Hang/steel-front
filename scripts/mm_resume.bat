@echo off
setlocal
set URL=https://gh-proxy.com/https://github.com/RodZill4/material-maker/releases/download/1.7/material_maker_1_7_windows.zip
set OUT=D:\3D_Work\material_maker_1_7_windows.zip
for /l %%i in (1,1,8) do (
  curl.exe -L -C - -o "%OUT%" "%URL%" --ssl-no-revoke --silent --show-error --connect-timeout 20 --max-time 570
  for %%f in ("%OUT%") do set SZ=%%~zf
  if !SZ! GTR 109000000 goto done
  echo attempt %%i done, size !SZ!, resume...
  timeout /t 3 /nobreak >nul
)
:done
echo FINISHED size %SZ%
endlocal