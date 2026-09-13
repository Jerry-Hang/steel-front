# Autonomous visual sweep: play the game myself and capture many angles (ASCII only).
#
# WHY: the user is right that asking them for screenshots is the wrong loop -- I have
# multimodal input and a working PostMessage injection harness, so I should look at the
# game myself across MANY scenes instead of one frame at a time.
#
# WHAT IT DOES
#   1. launch, force the window to the foreground (AttachThreadInput workaround)
#   2. press R to enter Playing
#   3. repeatedly: turn the view by N degrees, optionally walk, then capture the window
#      via PrintWindow (cap_safe's method: no focus theft for the capture itself)
#   4. leave every PNG in screenshots/sweep_<tag>_<i>.png for me to read
#
# Turning uses the harness documented in AGENTS.md Iron Rule C: PostMessage + a fresh
# WM_LBUTTONDOWN before each move, offsets relative to the window centre (never
# accumulated), 300 ms apart. Anything else silently drops the input.
#
# Usage:
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\sweep.ps1 [-Tag s1] [-Shots 8] [-TurnDeg 45]

param(
    [string]$Tag = "sweep",
    [int]$Shots = 8,
    [double]$TurnDeg = 45.0,
    [int]$WalkMs = 0,
    [int]$WarmupSec = 9
)

$ErrorActionPreference = "Continue"
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo
$exe = Join-Path $repo "target\release\steel-front.exe"

# System.Drawing must be loaded BEFORE the P/Invoke block; and the bitmap work is done
# in PowerShell rather than inside the C# source -- that is exactly how the working
# cap_safe.ps1 does it. Putting `using System.Drawing` in the C# block fails to compile
# under PowerShell 7 because the assembly is not referenced by default.
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class SW {
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
    public static void KeyDown(IntPtr h, uint vk) {
        uint sc = MapVirtualKeyW(vk, 0);
        PostMessageW(h, 0x0100, (IntPtr)vk, (IntPtr)((sc << 16) | 1));
    }
    public static void KeyUp(IntPtr h, uint vk) {
        uint sc = MapVirtualKeyW(vk, 0);
        PostMessageW(h, 0x0101, (IntPtr)vk, (IntPtr)((sc << 16) | 1 | 0xC0000000));
    }
    public static void Click(IntPtr h, int x, int y) {
        PostMessageW(h, 0x0201, (IntPtr)1, (IntPtr)((y << 16) | (x & 0xFFFF)));
    }
    public static void Move(IntPtr h, int x, int y) {
        PostMessageW(h, 0x0200, IntPtr.Zero, (IntPtr)((y << 16) | (x & 0xFFFF)));
    }
    public static void LUp(IntPtr h, int x, int y) {
        PostMessageW(h, 0x0202, IntPtr.Zero, (IntPtr)((y << 16) | (x & 0xFFFF)));
    }
}
"@

function Save-Shot([IntPtr]$h, [string]$path) {
    $r = New-Object SW+RECT
    if (-not [SW]::GetClientRect($h, [ref]$r)) { return $false }
    $w = $r.r - $r.l
    $ht = $r.b - $r.t
    if ($w -le 0 -or $ht -le 0) { return $false }
    $bmp = New-Object System.Drawing.Bitmap($w, $ht)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $dc = $g.GetHdc()
    # flag 2 = PW_RENDERFULLCONTENT (the DWM-composed surface; flag 0 returns black for
    # some GPU-composited windows).
    [void][SW]::PrintWindow($h, $dc, 2)
    $g.ReleaseHdc($dc)
    $g.Dispose()
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    return $true
}

if (-not ("SW" -as [type])) { Write-Host "Add-Type failed"; exit 1 }
[SW]::Dpi()

# Sensitivity from the real config (never hard-code; AGENTS.md Iron Rule C).
$sens = 0.5
$cfg = Join-Path $env:USERPROFILE ".steel_front.cfg"
if (Test-Path $cfg) {
    foreach ($l in [System.IO.File]::ReadAllLines($cfg)) {
        if ($l -match "^sensitivity=([0-9.]+)") { $sens = [double]$Matches[1] }
    }
}
# dx pixels for a wanted yaw change: yaw_rad = dx * (0.0005 + sens*0.002)
$radPerPx = 0.0005 + $sens * 0.002
$dx = [int][Math]::Round(($TurnDeg * [Math]::PI / 180.0) / $radPerPx)
# MAX_LOOK_DELTA_PX = 512: split into steps, and each step must differ by 1px from the
# previous to survive winit's position dedup.
$steps = [Math]::Max(1, [int][Math]::Ceiling([Math]::Abs($dx) / 400.0))
$perStep = [int]($dx / $steps)

$log = Join-Path $repo "logs\$Tag.log.err"
$out = Join-Path $repo "logs\$Tag.log.out"
if (Test-Path $log) { Remove-Item $log -Force }
if (Test-Path $out) { Remove-Item $out -Force }

$p = Start-Process -FilePath $exe -WorkingDirectory $repo -PassThru `
    -RedirectStandardError $log -RedirectStandardOutput $out

$h = [IntPtr]::Zero
for ($i = 0; $i -lt 60; $i++) {
    Start-Sleep -Milliseconds 250
    $h = [SW]::GameWindow([uint32]$p.Id)
    if ($h -ne [IntPtr]::Zero) { break }
}
Write-Host ("window: " + $h)
if ($h -eq [IntPtr]::Zero) { Stop-Process -Id $p.Id -Force; exit 1 }
[void][SW]::Force($h)
Start-Sleep -Seconds $WarmupSec

# Enter Playing.
[SW]::KeyDown($h, 0x52); Start-Sleep -Milliseconds 60; [SW]::KeyUp($h, 0x52)
Start-Sleep -Seconds 2

$rect = New-Object SW+RECT
[void][SW]::GetClientRect($h, [ref]$rect)
$cw = $rect.r - $rect.l
$ch = $rect.b - $rect.t
$cx = [int]($cw / 2)
$cy = [int]($ch / 2)
Write-Host ("client " + $cw + "x" + $ch + "  centre " + $cx + "," + $cy + "  dx=" + $dx + " in " + $steps + " steps of " + $perStep)

for ($s = 0; $s -lt $Shots; $s++) {
    # walk forward a little if asked
    if ($WalkMs -gt 0) {
        [SW]::KeyDown($h, 0x57)                     # VK_W
        Start-Sleep -Milliseconds $WalkMs
        [SW]::KeyUp($h, 0x57)
        Start-Sleep -Milliseconds 250
    }
    # turn
    for ($k = 1; $k -le $steps; $k++) {
        $off = $perStep * $k
        [SW]::Click($h, $cx, $cy)
        $nx = $cx + $off
        if ($nx -eq $cx + $perStep * ($k - 1)) { $nx += 1 }   # 1px apart, or winit dedups
        if ($nx -lt 1) { $nx = 1 }
        if ($nx -gt $cw - 2) { $nx = $cw - 2 }
        [SW]::Move($h, $nx, $cy)
        Start-Sleep -Milliseconds 300
        [SW]::LUp($h, $nx, $cy)
    }
    Start-Sleep -Milliseconds 250
    $shot = Join-Path $repo ("screenshots\" + $Tag + "_" + $s + ".png")
    if (Save-Shot $h $shot) { Write-Host ("  shot " + $s + " -> " + $shot) }
    else { Write-Host ("  shot " + $s + " FAILED") }
}

Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 2

Write-Host "==== cam: lines (last 3) ===="
if (Test-Path $log) {
    $lines = [System.IO.File]::ReadAllLines($log, [System.Text.Encoding]::UTF8)
    $cam = @($lines | Where-Object { $_ -match "cam:" })
    $cam | Select-Object -Last 3 | ForEach-Object {
        $i = $_.IndexOf("]"); Write-Host ("  " + $_.Substring([Math]::Max(0, $i + 1)).Trim())
    }
}
