# Verify the cursor-clip fix while the game is actually PLAYING (ASCII only).
#
# THE BUG (2026-09-13): winit's Windows backend, in window_state.rs::refresh_os_cursor,
# does:
#     if locked        { clip to 1x1 at window centre }
#     else if HIDDEN   { clip to 1x1 at window centre }   <-- this one
#     else             { clip to client_rect }
# Our engine used Confined (Windows cannot use Locked -- no DeviceEvent::MouseMotion)
# AND hid the cursor, so the pointer was pinned to a single pixel. The absolute-position
# look path reads CursorMoved deltas, so dx was always 0 => the view never turned, while
# the keyboard and mouse buttons kept working. Measured live: GetClipCursor returned
# 853,533 - 854,534 -- exactly 1x1.
#
# FIX: on the absolute path keep the cursor VISIBLE, so winit clips to client_rect.
#
# This script enters Playing (press R), waits, then measures GetClipCursor and prints the
# engine's own cam: verdict. A clip rect that equals the window size == fixed.
#
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File scripts\probe_clip.ps1

param([int]$WarmupSec = 8, [int]$PlaySec = 6)

$ErrorActionPreference = "Continue"
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo
$exe = Join-Path $repo "target\release\steel-front.exe"
$log = Join-Path $repo "logs\cliptest.log.err"
$out = Join-Path $repo "logs\cliptest.log"

if (Test-Path $log) { Remove-Item $log -Force }
if (Test-Path $out) { Remove-Item $out -Force }

Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class CV {
    public delegate bool EnumProc(IntPtr h, IntPtr p);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr p);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, ref uint pid);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr SetFocus(IntPtr h);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern IntPtr GetFocus();
    [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool f);
    [DllImport("user32.dll")] public static extern bool GetClipCursor(out RECT r);
    [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint msg, IntPtr wp, IntPtr lp);
    [DllImport("user32.dll")] public static extern uint MapVirtualKeyW(uint code, uint mapType);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int l; public int t; public int r; public int b; }

    public static IntPtr GameWindow(uint want) {
        IntPtr found = IntPtr.Zero;
        EnumWindows(delegate(IntPtr h, IntPtr p) {
            uint pid = 0; GetWindowThreadProcessId(h, ref pid);
            if (pid == want) {
                var c = new StringBuilder(256); GetClassNameW(h, c, 256);
                if (c.ToString() == "Window Class") { found = h; return false; }
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }
    public static bool Force(IntPtr t) {
        IntPtr fg = GetForegroundWindow();
        uint fgPid = 0; uint fgThread = GetWindowThreadProcessId(fg, ref fgPid);
        uint my = GetCurrentThreadId();
        bool att = false;
        if (fgThread != my && fgThread != 0) { att = AttachThreadInput(fgThread, my, true); }
        BringWindowToTop(t); SetFocus(t);
        bool ok = SetForegroundWindow(t);
        if (att) { AttachThreadInput(fgThread, my, false); }
        return ok;
    }
    // PostMessage a key by VK code (the engine uses PostMessage-safe input).
    public static void Key(IntPtr h, uint vk) {
        uint sc = MapVirtualKeyW(vk, 0);
        IntPtr lp = (IntPtr)((sc << 16) | 1);
        PostMessageW(h, 0x0100, (IntPtr)vk, lp);           // WM_KEYDOWN
        System.Threading.Thread.Sleep(60);
        PostMessageW(h, 0x0101, (IntPtr)vk, (IntPtr)((sc << 16) | 1 | 0xC0000000)); // WM_KEYUP
    }
}
"@

$p = Start-Process -FilePath $exe -WorkingDirectory $repo -PassThru `
    -RedirectStandardError $log -RedirectStandardOutput $out

$hwnd = [IntPtr]::Zero
for ($i = 0; $i -lt 40; $i++) {
    Start-Sleep -Milliseconds 250
    $hwnd = [CV]::GameWindow([uint32]$p.Id)
    if ($hwnd -ne [IntPtr]::Zero) { break }
}
Write-Host ("game window: " + $hwnd)
[void][CV]::Force($hwnd)
Start-Sleep -Seconds $WarmupSec

# Enter Playing (VK_R = 0x52) -- same key the docs use.
[CV]::Key($hwnd, 0x52)
Start-Sleep -Seconds $PlaySec

$rc = New-Object CV+RECT
[void][CV]::GetClipCursor([ref]$rc)
$w = $rc.r - $rc.l
$h = $rc.b - $rc.t
Write-Host ("ClipCursor: " + $rc.l + "," + $rc.t + " - " + $rc.r + "," + $rc.b + "   (" + $w + " x " + $h + ")")
if ($w -le 2 -and $h -le 2) {
    Write-Host "  >>> STILL 1x1: the cursor is pinned, mouse-look CANNOT work. NOT fixed."
} elseif ($w -gt 100 -and $h -gt 100) {
    Write-Host "  >>> cursor is NOT pinned: the clip rect is a real area. FIXED."
} else {
    Write-Host "  >>> unexpected size."
}

Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 2

Write-Host "==== cam: lines ===="
if (Test-Path $log) {
    $lines = [System.IO.File]::ReadAllLines($log, [System.Text.Encoding]::UTF8)
    $cam = @($lines | Where-Object { $_ -match "cam:" })
    $cam | Select-Object -Last 4 | ForEach-Object {
        $i = $_.IndexOf("]"); Write-Host ("    " + $_.Substring([Math]::Max(0, $i + 1)).Trim())
    }
    $last = $cam | Select-Object -Last 1
    if ($last -match "focus=(\w+) cap=(\w+) lock=(\w+)") {
        Write-Host ("  VERDICT focus=" + $Matches[1] + " cap=" + $Matches[2] + " lock=" + $Matches[3])
    }
    Write-Host "---- input: ----"
    $lines | Where-Object { $_ -match "input:" } | Select-Object -First 4 | ForEach-Object {
        $i = $_.IndexOf("]"); Write-Host ("    " + $_.Substring([Math]::Max(0, $i + 1)).Trim())
    }
}
