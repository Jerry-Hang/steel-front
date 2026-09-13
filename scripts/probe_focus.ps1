# Focus verdict probe (ASCII only).
#
# WHY: the engine grabs the cursor only while its window is the FOREGROUND window,
# and the user reports mouse-look dead while the keyboard works. That combination is
# odd: real keyboard input implies the window HAS focus. So this probe prints the
# verdict the engine computed (cam: line: focus/cap/lock/fgpid/mypid) after trying to
# put the game window in the foreground.
#
# It does NOT press R, so the game stays in the main menu and never grabs the cursor.
#
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File scripts\probe_focus.ps1 [-Secs 10]

param([int]$Secs = 10)

$ErrorActionPreference = "Continue"
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo
$exe = Join-Path $repo "target\release\steel-front.exe"
$log = Join-Path $repo "logs\focustest.log.err"
$out = Join-Path $repo "logs\focustest.log"

if (Test-Path $log) { Remove-Item $log -Force }
if (Test-Path $out) { Remove-Item $out -Force }

# One flat P/Invoke surface: no overloads, so Add-Type cannot get confused.
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
using System.Text;

public class FG {
    public delegate bool EnumProc(IntPtr h, IntPtr p);

    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr p);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, ref uint pid);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr SetFocus(IntPtr h);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool f);

    public static IntPtr GameWindow(uint want) {
        IntPtr found = IntPtr.Zero;
        EnumWindows(delegate(IntPtr h, IntPtr p) {
            uint pid = 0;
            GetWindowThreadProcessId(h, ref pid);
            if (pid == want) {
                StringBuilder c = new StringBuilder(256);
                GetClassNameW(h, c, 256);
                if (c.ToString() == "Window Class") { found = h; return false; }
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }

    // A background process may not call SetForegroundWindow successfully; attaching our
    // input queue to the current foreground thread first makes the call legal.
    public static bool ForceForeground(IntPtr target) {
        IntPtr fg = GetForegroundWindow();
        uint fgPid = 0;
        uint fgThread = GetWindowThreadProcessId(fg, ref fgPid);
        uint myThread = GetCurrentThreadId();
        bool attached = false;
        if (fgThread != myThread && fgThread != 0) {
            attached = AttachThreadInput(fgThread, myThread, true);
        }
        BringWindowToTop(target);
        SetFocus(target);
        bool ok = SetForegroundWindow(target);
        if (attached) { AttachThreadInput(fgThread, myThread, false); }
        return ok;
    }
}
"@

if (-not ("FG" -as [type])) {
    Write-Host "Add-Type failed; cannot probe."
    exit 1
}

$p = Start-Process -FilePath $exe -WorkingDirectory $repo -PassThru `
    -RedirectStandardError $log -RedirectStandardOutput $out

$hwnd = [IntPtr]::Zero
for ($i = 0; $i -lt 40; $i++) {
    Start-Sleep -Milliseconds 250
    $hwnd = [FG]::GameWindow([uint32]$p.Id)
    if ($hwnd -ne [IntPtr]::Zero) { break }
}
Write-Host ("game window handle: " + $hwnd)
if ($hwnd -ne [IntPtr]::Zero) {
    $ok = [FG]::ForceForeground($hwnd)
    Write-Host ("ForceForeground returned: " + $ok)
}
Start-Sleep -Seconds $Secs

$fg = [FG]::GetForegroundWindow()
$fgPid = 0
[void][FG]::GetWindowThreadProcessId($fg, [ref]$fgPid)
Write-Host ("foreground pid: " + $fgPid + " | game pid: " + $p.Id)

Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 2

Write-Host "==== cam: lines ===="
if (Test-Path $log) {
    $lines = [System.IO.File]::ReadAllLines($log, [System.Text.Encoding]::UTF8)
    $cam = @($lines | Where-Object { $_ -match "cam:" })
    Write-Host ("  cam: lines: " + $cam.Count)
    $cam | Select-Object -Last 5 | ForEach-Object {
        $i = $_.IndexOf("]")
        Write-Host ("    " + $_.Substring([Math]::Max(0, $i + 1)).Trim())
    }
    $last = $cam | Select-Object -Last 1
    if ($last -match "focus=(\w+) cap=(\w+) lock=(\w+)") {
        Write-Host ("  VERDICT focus=" + $Matches[1] + " cap=" + $Matches[2] + " lock=" + $Matches[3])
    }
} else {
    Write-Host "  log not found"
}
