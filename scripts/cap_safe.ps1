param(
    [int]$WarmupSec = 10,
    [int]$HoldSec = 4,
    [string]$Tag = "cap",
    [int[]]$Keys = @(),
    [int]$AfterKeysSec = 3,
    [switch]$Stress
)

# Mouse-safety harness for steel-front. The engine self-grabs the cursor on entering
# gameplay, so every launch is wrapped in try/finally that ALWAYS kills the process.
# Keys are posted to the game's own window handle (PostMessage) instead of being injected
# globally, so a mistimed run can never drive another window or steal the user's focus.
$ErrorActionPreference = "Stop"
$repo = "D:\Rust\steel-front"
$exe = Join-Path $repo "target\release\steel-front.exe"
$shots = Join-Path $repo "screenshots"
$logs = Join-Path $repo "logs"
New-Item -ItemType Directory -Force -Path $shots, $logs | Out-Null

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class W32S {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr wp, IntPtr lp);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@
[W32S]::SetProcessDPIAware() | Out-Null

$WM_KEYDOWN = 0x0100
$WM_KEYUP   = 0x0101

function Kill-Game {
    Get-Process -Name steel-front -ErrorAction SilentlyContinue | ForEach-Object {
        try { $_.Kill(); $_.WaitForExit(5000) | Out-Null } catch {}
    }
}

function Screenshot([IntPtr]$h, [string]$out) {
    $r = New-Object W32S+RECT
    [W32S]::GetWindowRect($h, [ref]$r) | Out-Null
    $w = $r.R - $r.L; $ht = $r.B - $r.T
    if ($w -lt 16 -or $ht -lt 16) { Write-Host "SKIP SHOT (window ${w}x${ht})"; return }
    $bmp = New-Object System.Drawing.Bitmap($w, $ht)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $dc = $g.GetHdc()
    [W32S]::PrintWindow($h, $dc, 2) | Out-Null
    $g.ReleaseHdc($dc); $g.Dispose()
    $bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    Write-Host "SAVED $out (${w}x${ht})"
}

function Post-Key([IntPtr]$h, [int]$vk) {
    $downL = [IntPtr](1 -bor 0x40000000)   # bit30 transition set => key was previously up
    $upL   = [IntPtr](1 -bor 0xC0000000)   # bit31 previous state + bit30 transition
    [W32S]::PostMessage($h, $WM_KEYDOWN, [IntPtr]$vk, $downL) | Out-Null
    Start-Sleep -Milliseconds 80
    [W32S]::PostMessage($h, $WM_KEYUP,   [IntPtr]$vk, $upL)   | Out-Null
}

$logOut = Join-Path $logs "$Tag.log"
$logErr = Join-Path $logs "$Tag.log.err"
Remove-Item $logOut, $logErr -ErrorAction SilentlyContinue

Kill-Game
Start-Sleep -Seconds 1

# Start-Process inherits this session's environment, so set the RV3D_* knobs directly.
$env:RV3D_AUTOSTART = "1"
if ($Stress) { $env:RV3D_STRESS_AI = "1" }

$exitNote = "ok"
try {
    $proc = Start-Process -FilePath $exe -WorkingDirectory $repo -PassThru `
        -RedirectStandardOutput $logOut -RedirectStandardError $logErr
    Write-Host "PID $($proc.Id) launched; cursor will be grabbed until cleanup"

    Start-Sleep -Seconds $WarmupSec

    if ($proc.HasExited) {
        $exitNote = "PROCESS EXITED EARLY (exit code $($proc.ExitCode))"
        Write-Host "!! $exitNote"
    } else {
        $p = Get-Process -Id $proc.Id -ErrorAction SilentlyContinue |
            Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
        if (-not $p) {
            $exitNote = "NO WINDOW HANDLE YET"
            Write-Host "!! $exitNote"
        } else {
            $h = $p.MainWindowHandle
            Screenshot $h (Join-Path $shots "$Tag`_a.png")

            foreach ($vk in $Keys) {
                Write-Host "POST VK $vk"
                Post-Key $h $vk
                Start-Sleep -Seconds 1
            }
            if ($Keys.Count -gt 0) { Start-Sleep -Seconds $AfterKeysSec }
            Screenshot $h (Join-Path $shots "$Tag`_b.png")

            Write-Host ("foreground is game window: " + ([W32S]::GetForegroundWindow() -eq $h))
        }
    }
    Start-Sleep -Seconds $HoldSec
}
catch {
    $exitNote = "HARNESS ERROR: $($_.Exception.Message)"
    Write-Host "!! $exitNote"
}
finally {
    Kill-Game
    Remove-Item Env:RV3D_AUTOSTART -ErrorAction SilentlyContinue
    Remove-Item Env:RV3D_STRESS_AI -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 500
    if (Get-Process -Name steel-front -ErrorAction SilentlyContinue) {
        Write-Host "!! STILL ALIVE after kill - inspect manually"
    } else {
        Write-Host "CLEANUP: no steel-front process remains (cursor released)"
    }
}

Write-Host "=== RESULT: $exitNote ==="
if (Test-Path $logErr) {
    Write-Host "=== stderr tail ==="
    Get-Content $logErr -Tail 10 | ForEach-Object { Write-Host $_ }
    $fps = Select-String -Path $logErr -Pattern 'fps' -SimpleMatch | Select-Object -Last 2
    if ($fps) { Write-Host "=== fps ==="; $fps | ForEach-Object { Write-Host $_.Line } }
    $bad = Select-String -Path $logErr -Pattern 'panic|device lost|ERROR|VUID' | Select-Object -Last 5
    if ($bad) { Write-Host "=== errors ==="; $bad | ForEach-Object { Write-Host $_.Line } }
}
