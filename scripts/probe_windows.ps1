# Enumerate the game process's top-level windows (ASCII only).
#
# WHY: the engine reports focus=false and fgpid != mypid, yet the user can see and
# click the game -- so the window that receives input is not the one we think it is,
# or it is not a top-level foreground window at all. AGENTS.md already warns that
# Process.MainWindowHandle is wrong for this winit program.
#
# Prints, for every top-level window belonging to the game process:
#   hwnd, visible, enabled, title, class, and whether it is the foreground window.
#
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File scripts\probe_windows.ps1 [-Secs 8]

param([int]$Secs = 8)

$ErrorActionPreference = "Continue"
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo
$exe = Join-Path $repo "target\release\steel-front.exe"

Add-Type @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public class W {
    public delegate bool EnumProc(IntPtr h, IntPtr p);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr p);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    public static List<string> ForPid(uint want) {
        var res = new List<string>();
        IntPtr fg = GetForegroundWindow();
        EnumWindows((h, p) => {
            uint pid; GetWindowThreadProcessId(h, out pid);
            if (pid == want) {
                var t = new StringBuilder(512); GetWindowTextW(h, t, 512);
                var c = new StringBuilder(512); GetClassNameW(h, c, 512);
                res.Add(string.Format("hwnd={0} vis={1} en={2} fg={3} class='{4}' title='{5}'",
                    h, IsWindowVisible(h), IsWindowEnabled(h), (h == fg), c, t));
            }
            return true;
        }, IntPtr.Zero);
        return res;
    }
}
"@

$p = Start-Process -FilePath $exe -WorkingDirectory $repo -PassThru `
    -RedirectStandardError (Join-Path $repo "logs\winprobe.log.err") `
    -RedirectStandardOutput (Join-Path $repo "logs\winprobe.log")

Start-Sleep -Seconds $Secs

Write-Host ("game pid: " + $p.Id)
$fg = [W]::GetForegroundWindow()
$fgPid = 0
[void][W]::GetWindowThreadProcessId($fg, [ref]$fgPid)
Write-Host ("foreground hwnd=" + $fg + " pid=" + $fgPid)
Write-Host "==== windows owned by the game process ===="
$list = [W]::ForPid([uint32]$p.Id)
if ($list.Count -eq 0) {
    Write-Host "  (none)"
} else {
    foreach ($l in $list) { Write-Host ("  " + $l) }
}

Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 2
