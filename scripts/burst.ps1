# Burst capture: several frames ~40 ms apart WHILE the view is turning (ASCII only).
#
# WHY: the user reports the first-person gun swinging side to side with an obvious
# ghosting trail during fast view swings and while running. A single still cannot show a
# temporal artifact, and the normal sweep.ps1 waits 300 ms between injected moves
# (required by the injection recipe), which is far too slow to catch it. This grabs N
# frames back to back during ONE turn.
#
# It also prints the gun's screen position indirectly: the `cam:` line's yaw per frame, so
# a frame that was captured mid-turn is identifiable.
#
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File scripts\burst.ps1 [-Tag b1] [-Frames 8] [-GapMs 40]

param(
    [string]$Tag = "burst",
    [int]$Frames = 8,
    [int]$GapMs = 40,
    [int]$WarmupSec = 9
)

$ErrorActionPreference = "Continue"
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo
$exe = Join-Path $repo "target\release\steel-front.exe"

Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class BR {
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
        System.Threading.Thread.Sleep(40);
        PostMessageW(h, 0x0101, (IntPtr)vk, (IntPtr)((sc << 16) | 1 | 0xC0000000));
    }
    public static void Down(IntPtr h, uint vk) {
        uint sc = MapVirtualKeyW(vk, 0);
        PostMessageW(h, 0x0100, (IntPtr)vk, (IntPtr)((sc << 16) | 1));
    }
    public static void Up(IntPtr h, uint vk) {
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
    $r = New-Object BR+RECT
    if (-not [BR]::GetClientRect($h, [ref]$r)) { return $false }
    $w = $r.r - $r.l; $ht = $r.b - $r.t
    if ($w -le 0 -or $ht -le 0) { return $false }
    $bmp = New-Object System.Drawing.Bitmap($w, $ht)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $dc = $g.GetHdc()
    [void][BR]::PrintWindow($h, $dc, 2)
    $g.ReleaseHdc($dc); $g.Dispose()
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    return $true
}

if (-not ("BR" -as [type])) { Write-Host "Add-Type failed"; exit 1 }
[BR]::Dpi()

$log = Join-Path $repo "logs\$Tag.log.err"
$out = Join-Path $repo "logs\$Tag.log.out"
if (Test-Path $log) { Remove-Item $log -Force }

$p = Start-Process -FilePath $exe -WorkingDirectory $repo -PassThru `
    -RedirectStandardError $log -RedirectStandardOutput $out
$h = [IntPtr]::Zero
for ($i = 0; $i -lt 60; $i++) {
    Start-Sleep -Milliseconds 250
    $h = [BR]::GameWindow([uint32]$p.Id)
    if ($h -ne [IntPtr]::Zero) { break }
}
if ($h -eq [IntPtr]::Zero) { Stop-Process -Id $p.Id -Force; exit 1 }
[void][BR]::Force($h)
Start-Sleep -Seconds $WarmupSec
[BR]::Key($h, 0x52)
Start-Sleep -Seconds 2

$rect = New-Object BR+RECT
[void][BR]::GetClientRect($h, [ref]$rect)
$cw = $rect.r - $rect.l; $ch = $rect.b - $rect.t
$cx = [int]($cw / 2); $cy = [int]($ch / 2)

# Walk forward (W held) so the gait sway is active, then burst-capture during ONE move.
[BR]::Down($h, 0x57)                     # VK_W down, held through the burst
Start-Sleep -Milliseconds 400

[BR]::Click($h, $cx, $cy)
[BR]::Move($h, $cx + 300, $cy)           # a big single horizontal jump, i.e. a fast yaw change
for ($i = 0; $i -lt $Frames; $i++) {
    $shot = Join-Path $repo ("screenshots\" + $Tag + "_" + $i + ".png")
    [void](Save-Shot $h $shot)
    Start-Sleep -Milliseconds $GapMs
}
[BR]::LUp($h, $cx + 300, $cy)
[BR]::Up($h, 0x57)

Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 2
Write-Host ("captured " + $Frames + " frames, gap " + $GapMs + " ms, while walking + turning")
