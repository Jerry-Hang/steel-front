@echo off
REM ============================================================================
REM  Steel Front launcher / build-and-run
REM ============================================================================
REM  ASCII ONLY. Windows cmd reads .bat in the OEM codepage; non-ASCII bytes in
REM  echo strings can corrupt the parser and the console output. Keep it ASCII.
REM
REM  Why this file exists:
REM  AGENTS.md has referenced "SteelFront.bat" for a long time, but the file was
REM  never in the repository -- the reference was stale. This is the real one.
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

echo [steel-front] touching build inputs...
REM ---------------------------------------------------------------------------
REM Touching is done with PowerShell, NOT with cmd's `copy /b FILE +,,` trick.
REM That trick is a trap: with a quoted path it does NOT merely update the
REM timestamp -- cmd resolves the destination from the source basename in the
REM CURRENT directory, so `copy /b "assets\x.spv" +,,` silently CREATES a stray
REM copy `x.spv` in the repo root. (Measured 2026-09-12: 7 junk files.)
REM The lines below only set LastWriteTime and never write file contents.
REM ---------------------------------------------------------------------------
if exist build.rs         powershell -NoProfile -Command "(Get-Item 'build.rs').LastWriteTime = Get-Date" >nul 2>&1
if exist build_spv_rt.rs  powershell -NoProfile -Command "(Get-Item 'build_spv_rt.rs').LastWriteTime = Get-Date" >nul 2>&1

REM assets\*.spv are read from disk AT RUNTIME by the engine, so they do NOT
REM need touching for correctness. AGENTS.md only mandates the two files above.
REM They are listed here for visibility, deliberately NOT touched.

echo [steel-front] building (release)...
cargo build --release
if errorlevel 1 (
    echo.
    echo [steel-front] BUILD FAILED - not launching.
    endlocal
    exit /b 1
)

set "EXE=target\release\steel-front.exe"
if not exist "%EXE%" (
    echo [steel-front] ERROR: %EXE% not found after a successful build.
    endlocal
    exit /b 1
)

echo [steel-front] launching...
REM Pass through any extra arguments, e.g.  SteelFront.bat --help
start "" "%EXE%" %*

endlocal
