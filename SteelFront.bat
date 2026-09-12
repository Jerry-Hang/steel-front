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
if exist build.rs         copy /b build.rs         +,, >nul 2>&1
if exist build_spv_rt.rs  copy /b build_spv_rt.rs  +,, >nul 2>&1

REM Any .spv under assets/ is read from disk AT RUNTIME by the engine.
REM If they changed, the build must re-run so the embedded copy stays in sync.
REM
REM NOTE: this must be a per-file loop. `copy /b assets\*.spv +,,` is WRONG --
REM with a wildcard, copy treats the extra names as additional SOURCES and tries
REM to write, which can clobber the shader files. One file per iteration only.
if exist assets\*.spv (
    for %%F in (assets\*.spv) do copy /b "%%F" +,, >nul 2>&1
)

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
