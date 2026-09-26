param(
    [int]$WarmupSec = 10,
    [int]$HoldSec = 4,
    [string]$Tag = "cap",
    # ASCII ONLY in this file -- Windows PowerShell 5.1 reads a BOM-less .ps1 as ANSI,
    # and non-ASCII bytes here silently swallowed the next line (2026-09-12: a Chinese
    # comment above this line ate `[int[]]$Keys = @(),` so -Keys was always empty).
    #
    # -Keys takes WINDOWS VIRTUAL-KEY CODES, not winit KeyCode enum indices.
    # winit KeyR=36 but Windows VK_R=82(0x52); 36 is VK_HOME. Z=90, C=67, B=66,
    # W=87 A=65 S=83 D=68 Space=32 Tab=9 Esc=27 Shift=16.
    # lParam bit16-23 carries the scancode (MapVirtualKey) -- winit needs it.
    [int[]]$Keys = @(),
    [int]$AfterKeysSec = 3,
    # Seconds to wait BETWEEN posted keys. Default 1s is too fast for a multi-step UI
    # sequence: entering a game goes through LoadingMap (map generation), and a key sent
    # while still loading is dropped -- e.g. "R then ESC" never reached the pause menu
    # (2026-09-26). Use -KeyGapSec 5 for sequences that cross a state change.
    [int]$KeyGapSec = 1,
    [switch]$Stress,
    # -NoAuto: do NOT force RV3D_AUTOSTART=1, so the game stays in its menu state.
    # Needed to screenshot the frosted-glass menu (every cap_safe run before 2026-09-19
    # auto-started a run, so the menu was never capturable). Kill-in-finally unchanged.
    [switch]$NoAuto,
    # VRAM admission budget in MiB (2026-09-19, PROGRESS 19.2). 0 = skip the gate.
    [int]$MaxGpuMib = 3200
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

# VRAM admission gate. When an external workload (user ML eval) holds the card, the
# game dies allocating the depth image BEFORE any window appears -- indistinguishable
# from a code regression from the outside (cost a whole debugging detour 2026-09-19).
# Refuse loudly instead of producing garbage. No nvidia-smi / unparseable => pass:
# this gate must never block hosts where it cannot measure.
if ($MaxGpuMib -gt 0 -and (Get-Command nvidia-smi -ErrorAction SilentlyContinue)) {
    $gpuLine = (& nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits 2>$null | Select-Object -First 1)
    $gpuUsed = 0
    if ($gpuLine -and [int]::TryParse([string]$gpuLine.Trim(), [ref]$gpuUsed)) {
        if ($gpuUsed -gt $MaxGpuMib) {
            Write-Host "GPU-BUSY: ${gpuUsed}MiB used > ${MaxGpuMib}MiB budget - refusing to launch (external VRAM load?)"
            exit 3
        }
        Write-Host "gpu-gate: ok (${gpuUsed}MiB used)"
    }
}

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
  [DllImport("user32.dll", CharSet = CharSet.Unicode, EntryPoint = "FindWindowW")]
  public static extern IntPtr FindWindowW(IntPtr cls, string title);
  [DllImport("user32.dll")] public static extern uint MapVirtualKey(uint uCode, uint uMapType);
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
    # 2026-09-26: $bmp.Save is a .NET call, and this script runs with
    # $ErrorActionPreference = "Continue" -- so when Save throws, execution falls through and
    # the old code printed "SAVED" for a file that was never written (the 5th instance today
    # of "the tool said it wrote X, but X is not there"). Screenshots are this repo's primary
    # evidence, so the FILE decides: exists and non-empty == SAVED, otherwise say so.
    if ((Test-Path $out) -and ((Get-Item $out).Length -gt 0)) {
        Write-Host "SAVED $out (${w}x${ht})"
    } else {
        Write-Host "SHOT-FAILED $out (Save produced no file) -- NOT evidence"
    }
}

function Post-Key([IntPtr]$h, [int]$vk) {
    # lParam bits 16-23 MUST carry the SCANCODE: winit's Windows backend resolves a
    # WM_KEYDOWN into a KeyCode from it. Before 2026-09-12 this only sent `1 | bit30`
    # (no scancode), so winit could not resolve the key and dropped the event.
    # cap_safe's -Keys had therefore never worked, while pm_play /
    # gameplay_smoke_pm.py used `lp = 1 | (scan << 16)` all along.
    # Do not regress to the scancode-less form.
    $scan = [int64][W32S]::MapVirtualKey([uint32]$vk, 0)   # MAPVK_VK_TO_VSC
    $base = [int64](1 -bor ($scan -shl 16))
    $downL = [IntPtr]$base
    $upL = [IntPtr]([int64]($base -bor [int64]0xC0000000))  # bit31 prior state + bit30 transition
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
if ($NoAuto) {
    Remove-Item Env:RV3D_AUTOSTART -ErrorAction SilentlyContinue
} else {
    $env:RV3D_AUTOSTART = "1"
}
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
        # Find the HWND by WINDOW TITLE, never via Process.MainWindowHandle: for a
        # winit app the latter is not the window that receives input, which is why
        # cap_safe's -Keys never worked (VK 36 was posted, the game stayed silent,
        # while the same key through pm_play / gameplay_smoke_pm's FindWindowW path
        # changed game state deterministically).
        # Poll, and normalise $h to [IntPtr]::Zero first: when the call returns null
        # the `-eq Zero` test is false and null reaches Screenshot, which then throws
        # "cannot convert null to IntPtr".
        $h = [IntPtr]::Zero
        for ($i = 0; $i -lt 40; $i++) {
            $h = [W32S]::FindWindowW([IntPtr]::Zero, "Steel Front - Vulkan")
            if ($h -ne [IntPtr]::Zero) { break }
            Start-Sleep -Milliseconds 250
        }
        if ($h -eq [IntPtr]::Zero) {
            $exitNote = "NO WINDOW HANDLE YET"
            Write-Host "!! $exitNote"
        } else {
            Screenshot $h (Join-Path $shots "$Tag`_a.png")

            foreach ($vk in $Keys) {
                Write-Host "POST VK $vk"
                Post-Key $h $vk
                Start-Sleep -Seconds $KeyGapSec
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
