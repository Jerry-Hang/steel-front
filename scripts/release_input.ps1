#release_input.ps1 - hand input control back to the user. Idempotent; safe to run anytime.
#
# Why this exists
# ---------------
# On entering gameplay the game does two things (main.rs::sync_cursor):
#   1. window.set_cursor_grab(Locked, falling back to Confined)  -> Win32 ClipCursor
#   2. window.set_cursor_visible(false)                          -> Win32 ShowCursor(FALSE)
#
# When the process is force-killed:
#   * (1) the clip rect normally dies with the window, but a stale/confined rect is
#     the failure this script actually repairs (verified below).
#   * (2) ShowCursor keeps a *thread-scoped* display counter, so it dies with the
#     game thread. The ShowCursor loop here is therefore best-effort belt-and-braces,
#     not the main mechanism - it can only correct the counter of THIS thread.
#     Stated plainly rather than implied, because implying more would be a lie.
#
# What is checked, not inferred:
#   * no steel-front process remains
#   * GetClipCursor no longer confines the cursor to a sub-screen rect
#     (NOTE: a rect equal to the full screen is the NORMAL desktop state, not a
#      leak - an earlier version of this script wrongly treated it as one)
#   * GetCursorInfo still reports CURSOR_SHOWING
#
# NOTE: pure ASCII on purpose. Windows PowerShell 5.1 reads a BOM-less .ps1 as
# ANSI, so non-ASCII inside a *string literal* can corrupt quoting and break the
# script outright. cap_safe.ps1 is ASCII-only for the same reason.
#
# Usage
# -----
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\release_input.ps1
#   add -Quiet for a single-line verdict
param([switch]$Quiet)

$ErrorActionPreference = "Continue"

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class RlsInput {
  [DllImport("user32.dll")] public static extern int  ShowCursor(bool bShow);
  [DllImport("user32.dll")] public static extern bool ClipCursor(IntPtr lpRect);
  [DllImport("user32.dll")] public static extern bool GetClipCursor(out RECT r);
  [DllImport("user32.dll")] public static extern bool GetCursorInfo(ref CURSORINFO ci);
  [DllImport("user32.dll")] public static extern int  GetSystemMetrics(int i);

  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  [StructLayout(LayoutKind.Sequential)] public struct CURSORINFO {
    public int cbSize; public int flags; public IntPtr hCursor; public POINT pt;
  }
  public const int CURSOR_SHOWING = 0x0001;
  public const int SM_CXSCREEN = 0;
  public const int SM_CYSCREEN = 1;
}
"@

function Get-CursorVisible {
    $ci = New-Object RlsInput+CURSORINFO
    $ci.cbSize = [System.Runtime.InteropServices.Marshal]::SizeOf($ci)
    if (-not [RlsInput]::GetCursorInfo([ref]$ci)) { return $null }
    return (($ci.flags -band [RlsInput]::CURSOR_SHOWING) -ne 0)
}

function Get-ClipState {
    $r = New-Object RlsInput+RECT
    if (-not [RlsInput]::GetClipCursor([ref]$r)) { return $null }
    $sw = [RlsInput]::GetSystemMetrics([RlsInput]::SM_CXSCREEN)
    $sh = [RlsInput]::GetSystemMetrics([RlsInput]::SM_CYSCREEN)
    # Full-screen clip == normal desktop. Sub-screen clip == somebody confined it.
    $confined = -not ($r.L -le 0 -and $r.T -le 0 -and $r.R -ge $sw -and $r.B -ge $sh)
    return [pscustomobject]@{ Confined = $confined; Rect = "$($r.L),$($r.T)-$($r.R),$($r.B)" }
}

# -- 1. terminate whatever is holding the input ---------------------------------
$killed = @()
Get-Process -Name steel-front -ErrorAction SilentlyContinue | ForEach-Object {
    $killed += "pid $($_.Id)"
    try { $_.Kill(); $_.WaitForExit(5000) | Out-Null } catch {}
}
$killNote = if ($killed.Count) { $killed -join ', ' } else { '(no leftover process)' }

# -- 2. clear any cursor confinement --------------------------------------------
$clipBefore = Get-ClipState
[RlsInput]::ClipCursor([IntPtr]::Zero) | Out-Null
$clipAfter = Get-ClipState

# -- 3. best-effort: correct THIS thread's display counter ----------------------
$visible0 = Get-CursorVisible
$fixed = 0
for ($i = 0; $i -lt 16; $i++) {
    if ((Get-CursorVisible) -eq $true) { break }
    [RlsInput]::ShowCursor($true) | Out-Null
    $fixed++
}
if ($fixed -gt 1) { for ($i = 1; $i -lt $fixed; $i++) { [RlsInput]::ShowCursor($false) | Out-Null } }
$visible1 = Get-CursorVisible

# -- 4. verdict: every item verified, none inferred -----------------------------
$alive = @(Get-Process -Name steel-front -ErrorAction SilentlyContinue).Count
$clipOk = ($null -ne $clipAfter) -and (-not $clipAfter.Confined)
$ok = ($alive -eq 0) -and $clipOk -and ($visible1 -eq $true)

if ($Quiet) {
    Write-Host ("RELEASE " + $(if ($ok) { "OK" } else { "FAILED" }))
} else {
    Write-Host "=== input control handback ==="
    Write-Host ("  {0,-16} {1}" -f "process",   "$killNote   alive_now=$alive")
    Write-Host ("  {0,-16} {1}" -f "clip_rect", "$($clipBefore.Rect) -> $($clipAfter.Rect)$(if ($clipBefore.Confined) { '   (was CONFINED, cleared)' })")
    Write-Host ("  {0,-16} {1}" -f "cursor",    "visible before=$visible0 after=$visible1   (ShowCursor best-effort, $fixed correction(s); thread-scoped)")
    Write-Host ("  {0,-16} {1}" -f "verdict",   $(if ($ok) { "OK - machine handed back" } else { "FAILED - see items above, inspect manually" }))
}

exit $(if ($ok) { 0 } else { 1 })
