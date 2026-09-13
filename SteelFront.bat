@echo off
REM ============================================================================
REM  Steel Front launcher  (build + run, and friends)
REM ============================================================================
REM  ASCII ONLY. Windows cmd reads .bat in the OEM codepage; non-ASCII bytes in
REM  echo strings can corrupt the parser. Keep this file ASCII.
REM
REM  Usage:
REM    SteelFront.bat              build, then launch (the normal path)
REM    SteelFront.bat play         same as no argument
REM    SteelFront.bat fast         launch WITHOUT rebuilding (use the current exe)
REM    SteelFront.bat smoke        run the PostMessage smoke gate, do not launch
REM    SteelFront.bat package      build + assemble dist\steel-front-<tag>.zip
REM    SteelFront.bat diag         launch with the diagnostics switches on
REM    SteelFront.bat --anything   build, then launch passing the args through
REM
REM  The touch list below is the important part. cargo decides whether to rebuild
REM  from file mtimes; when ONLY a shader / build script changes, the .rs sources
REM  are untouched, so cargo can silently skip the rebuild and you end up
REM  launching an OLD binary while believing you tested a new shader.
REM  Touching build.rs and build_spv_rt.rs forces the build script to re-run.
REM  See AGENTS.md (Iron Rule D) -- do not remove entries from this list.
REM ============================================================================

setlocal
cd /d "%~dp0"

set "MODE=%~1"
if "%MODE%"=="" set "MODE=play"

REM ---- smoke / package are their own scripts; hand over early ----------------
if /i "%MODE%"=="smoke" (
    echo [steel-front] running the smoke gate ^(PostMessage injection^)...
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\run_smoke_pm.ps1
    set "RC=%ERRORLEVEL%"
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\release_input.ps1 -Quiet
    endlocal & exit /b %RC%
)

if /i "%MODE%"=="package" (
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\package_release.ps1
    set "RC=%ERRORLEVEL%"
    endlocal & exit /b %RC%
)

REM ---- a stale instance is the #1 cause of "the window never came up" -------
tasklist /fi "imagename eq steel-front.exe" 2>nul | find /i "steel-front.exe" >nul
if not errorlevel 1 (
    echo [steel-front] an old instance is running - closing it first.
    taskkill /f /im steel-front.exe >nul 2>&1
    REM give the window time to go away before we might grab the cursor again
    ping -n 2 127.0.0.1 >nul
)

set "EXE=target\release\steel-front.exe"

if /i "%MODE%"=="fast" goto launch

echo [steel-front] touching build inputs...
REM ---------------------------------------------------------------------------
REM Touching uses PowerShell, NOT cmd's `copy /b FILE +,,` trick. That trick is
REM a trap: with a quoted path cmd resolves the destination from the source
REM basename in the CURRENT directory, so `copy /b "assets\x.spv" +,,` silently
REM CREATES a stray copy `x.spv` in the repo root. (Measured 2026-09-12: 7 junk
REM files.) The lines below only set LastWriteTime and never write contents.
REM ---------------------------------------------------------------------------
if exist build.rs         powershell -NoProfile -Command "(Get-Item 'build.rs').LastWriteTime = Get-Date" >nul 2>&1
if exist build_spv_rt.rs  powershell -NoProfile -Command "(Get-Item 'build_spv_rt.rs').LastWriteTime = Get-Date" >nul 2>&1

REM assets\*.spv are read from disk AT RUNTIME by the engine, so they do NOT need
REM touching for correctness. AGENTS.md only mandates the two files above.

echo [steel-front] building (release)...
cargo build --release
if errorlevel 1 (
    echo.
    echo [steel-front] BUILD FAILED - not launching.
    endlocal
    exit /b 1
)

:launch
if not exist "%EXE%" (
    echo [steel-front] ERROR: %EXE% not found.
    echo             Run  SteelFront.bat  without "fast" to build it first.
    endlocal
    exit /b 1
)

REM ---- tell the user exactly which binary they are about to run -------------
for %%F in ("%EXE%") do echo [steel-front] exe: %%~tF  %%~zF bytes

REM ---- missing assets would otherwise look like a mysterious empty world ----
if not exist "assets\props"    echo [steel-front] WARN: assets\props missing - city will be procedural only.
if not exist "assets\maps"     echo [steel-front] WARN: assets\maps missing - no TOML levels.
if not exist "assets\soldier\soldier.glb" echo [steel-front] note: no soldier.glb - NPCs use the 18-box path.

if /i "%MODE%"=="diag" (
    echo [steel-front] diagnostics on: RV3D_AI_PROF=1 RV3D_PROP_STATS=1
    set "RV3D_AI_PROF=1"
    set "RV3D_PROP_STATS=1"
)

REM Present with MAILBOX when actually playing.
REM
REM The engine defaults to IMMEDIATE (uncapped, no vsync) because that is the most
REM robust mode for its own benchmarks. On a real monitor that means constant TEARING,
REM which during a fast view swing reads exactly like the "ghosting trail" reported on
REM 2026-09-13 -- and it never shows up in a PrintWindow screenshot, because that
REM captures an already-composited frame.
REM MAILBOX neither tears nor blocks (FIFO deadlocks on a dGPU-direct setup waiting for
REM a vblank interrupt). To get the old behaviour:  set RV3D_PRESENT_MODE=immediate
if not defined RV3D_PRESENT_MODE set "RV3D_PRESENT_MODE=mailbox"

if not exist "logs" mkdir "logs"
REM Capture stderr to a file. Without this the game's own log (log::info!/ERROR) goes
REM to a console that closes with the window, so a play-test leaves NO evidence --
REM exactly what happened on 2026-09-13 when the mouse capture and a freeze had to be
REM diagnosed blind. `cam:` lines (yaw/pitch/focus/cap/lock/drag) land here.
set "PLAYLOG=logs\play_latest.log.err"
if exist "%PLAYLOG%" del "%PLAYLOG%" >nul 2>&1

echo [steel-front] launching...  (log: %PLAYLOG%)
REM Pass through any extra arguments, e.g.  SteelFront.bat play --help
REM
REM `/b` matters: `start "" cmd /c "..."` opens a NEW CONSOLE that takes the
REM foreground, so the game window never receives Focused(true) -- and
REM `main.rs::sync_cursor` requires `self.focused`, so the cursor would never be
REM grabbed and mouse-look would be dead with no error anywhere. That is exactly
REM the 2026-09-13 report. `/b` starts the child in THIS console (no new window,
REM no focus theft) while still allowing the stderr redirect.
REM
REM If mouse-look is ever dead again: click once on the game window. The engine
REM grabs the cursor only while it has focus (see AGENTS.md, Iron Rule C).
start "" /b cmd /c ""%EXE%" %* 2> "%PLAYLOG%""

endlocal
