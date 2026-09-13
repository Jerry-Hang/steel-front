# Weapon-switch probe: does the first-person gun MESH actually change? (ASCII only)
#
# USER REPORT (2026-09-13): "使用鼠标中键滚动切枪，还有输入数字用指令切枪的时候，
# 枪的模型没有变化" -- the weapon switches (HUD name changes) but the model on screen
# does not.
#
# WHY A DEDICATED SCRIPT: the two ways to switch are different code paths
#   * number keys  -> KeyCode path in main.rs
#   * mouse wheel  -> MouseWheel event path
# so both must be exercised separately. Between each switch we screenshot the SAME
# window with PrintWindow, and afterwards I compare the images myself.
#
# It also prints `weapons:` lines from the game log, which record the switch, so the
# question "did the game switch but draw the old mesh" is answerable without guessing.
#
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File scripts\probe_weapons.ps1

param([int]$WarmupSec = 9)

$ErrorActionPreference = "Continue"
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo
$exe = Join-Path $repo "target\release\steel-front.exe"

Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class WPN {
    public delegate bool EnumProc(IntPtr h, IntPtr p);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr p);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, ref uint pid);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr SetFocus(IntPtr h);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool f);
    [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint msg, IntPtr wp, IntPtr lp);
    [DllImport("user32.dll")] public static extern uint MapVirtualKeyW(uint code, uint mapType);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int l; public int t; public int r; public int b; }
    public static void Dpi() { try { SetProcessDPIAware(); } catch {} }
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
    public static void Key(IntPtr h, uint vk) {
        uint sc = MapVirtualKeyW(vk, 0);
        PostMessageW(h, 0x0100, (IntPtr)vk, (IntPtr)((sc << 16) | 1));
        System.Threading.Thread.Sleep(50);
        PostMessageW(h, 0x0101, (IntPtr)vk, (IntPtr)((sc << 16) | 1 | 0xC0000000));
    }
    // WM_MOUSEWHEEL: wParam high word = signed delta, lParam = screen coords.
    public static void Wheel(IntPtr h, int delta) {
        IntPtr wp = (IntPtr)((delta & 0xFFFF) << 16);
        PostMessageW(h, 0x020A, wp, IntPtr.Zero);
    }
}
"@

function Save-Shot([IntPtr]$h, [string]$path) {
    $r = New-Object WPN+RECT
    if (-not [WPN]::GetClientRect($h, [ref]$r)) { return $false }
    $w = $r.r - $r.l; $ht = $r.b - $r.t
    if ($w -le 0 -or $ht -le 0) { return $false }
    $bmp = New-Object System.Drawing.Bitmap($w, $ht)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $dc = $g.GetHdc()
    [void][WPN]::PrintWindow($h, $dc, 2)
    $g.ReleaseHdc($dc); $g.Dispose()
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    return $true
}

if (-not ("WPN" -as [type])) { Write-Host "Add-Type failed"; exit 1 }
[WPN]::Dpi()

$log = Join-Path $repo "logs\weapons.log.err"
$out = Join-Path $repo "logs\weapons.log.out"
if (Test-Path $log) { Remove-Item $log -Force }

$p = Start-Process -FilePath $exe -WorkingDirectory $repo -PassThru `
    -RedirectStandardError $log -RedirectStandardOutput $out
$h = [IntPtr]::Zero
for ($i = 0; $i -lt 60; $i++) {
    Start-Sleep -Milliseconds 250
    $h = [WPN]::GameWindow([uint32]$p.Id)
    if ($h -ne [IntPtr]::Zero) { break }
}
Write-Host ("window: " + $h)
if ($h -eq [IntPtr]::Zero) { Stop-Process -Id $p.Id -Force; exit 1 }
[void][WPN]::Force($h)
Start-Sleep -Seconds $WarmupSec
[WPN]::Key($h, 0x52)          # R -> enter Playing
Start-Sleep -Seconds 2

# Shot 0: the weapon we start with.
[void](Save-Shot $h (Join-Path $repo "screenshots\wpn_0_start.png"))
Write-Host "  shot 0 (start)"

# VK_1..VK_4 = 0x31..0x34 : the documented number-key switch.
$n = 1
foreach ($vk in @(0x31, 0x32, 0x33, 0x34)) {
    [WPN]::Key($h, [uint32]$vk)
    Start-Sleep -Milliseconds 900
    [void](Save-Shot $h (Join-Path $repo ("screenshots\wpn_" + $n + "_key" + $n + ".png")))
    Write-Host ("  shot " + $n + " (key " + $n + ")")
    $n++
}

# Mouse wheel: one notch up, one notch down.
[WPN]::Wheel($h, 120); Start-Sleep -Milliseconds 900
[void](Save-Shot $h (Join-Path $repo ("screenshots\wpn_" + $n + "_wheelup.png")))
Write-Host ("  shot " + $n + " (wheel up)")
$n++
[WPN]::Wheel($h, -120); Start-Sleep -Milliseconds 900
[void](Save-Shot $h (Join-Path $repo ("screenshots\wpn_" + $n + "_wheeldown.png")))
Write-Host ("  shot " + $n + " (wheel down)")

Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 2

Write-Host "==== weapons / gun log lines ===="
if (Test-Path $log) {
    $lines = [System.IO.File]::ReadAllLines($log, [System.Text.Encoding]::UTF8)
    $w = @($lines | Where-Object { $_ -match "weapons|gun|切枪|first_person" })
    if ($w.Count -eq 0) { Write-Host "  (none)" }
    foreach ($l in ($w | Select-Object -Last 16)) {
        $i = $l.IndexOf("]"); Write-Host ("  " + $l.Substring([Math]::Max(0, $i + 1)).Trim())
    }
}
