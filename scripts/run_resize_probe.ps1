# Resize / swapchain-recreation probe (ASCII only: PS 5.1 reads BOM-less .ps1 as ANSI).
#
# Why this exists (2026-09-25): AGENTS iron rule B says "open RV3D_VALIDATION=1 and run one
# round before touching pipeline / swapchain / synchronisation". The swapchain recreation
# path (destroy + init swapchain,
# hud_framebuffers, render-finished semaphores, command buffers, MSAA/depth/framebuffers)
# was the one part of that rule nobody had actually exercised under the layer -- the
# validation runs were all "start once, never resize".
#
# This probe drives the resize path on purpose and reports the layer's verdict:
#   * RV3D_VALIDATION=1 + DISABLE_RTSS_LAYER/DISABLE_GAMEPP_LAYER=1 (the two implicit
#     layers on this machine inject VkSwapchainCreateInfoKHR::flags, see open item #23:
#     leave them enabled and you get 5 noise VUIDs that are not the engine's).
#   * window resizing goes through SetWindowPos with SWP_NOACTIVATE + SWP_NOZORDER +
#     SWP_NOMOVE: it never steals focus and never touches the cursor, so it respects the
#     mouse-safety protocol (no SendInput, no SetCursorPos, no SetForegroundWindow).
#   * the optional F12 goes through PostMessage(WM_KEYDOWN) WITH the scancode in
#     lParam bits 16-23 -- winit resolves the key from that field (see cap_safe.ps1).
#
# P/Invoke gotcha (cost one run): FindWindowW must take `IntPtr cls` and be called with
# [IntPtr]::Zero. Declaring the first parameter as `string` and passing $null marshals an
# empty class name, the lookup fails, and the probe reports "window not found".
#
# Usage:
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\run_resize_probe.ps1
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\run_resize_probe.ps1 -NoShot
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\run_resize_probe.ps1 -PT -NoShot
param(
    [string]$Tag = "resize_probe",
    [int]$WarmupSec = 15,
    [int]$AfterSecs = 12,
    [string]$Sizes = "1280x720,1600x900,1024x768,2560x1600,1280x800",
    [switch]$NoShot,
    # -PT also turns on the live path tracer (RV3D_PT_LIVE=1) at 512-wide internal
    # resolution (RV3D_PT_SIZE=512). The PT frame is blitted into the swapchain image,
    # and until 2026-09-26 that blit hardcoded a 2560x1600 destination -- so it only
    # stayed inside the destination image on the default window size, and any other
    # size (exactly what this probe produces) trips
    # VUID-vkCmdBlitImage-dstOffsets-00203. -PT is the regression repro for it:
    #   scripts\run_resize_probe.ps1 -PT -Tag pt_resize -NoShot
    [switch]$PT
)
$ErrorActionPreference = "Continue"
$repo = "D:\Rust\steel-front"
$exe  = Join-Path $repo "target\release\steel-front.exe"
$logs = Join-Path $repo "logs"
$logOut = Join-Path $logs "$Tag.log"
$logErr = Join-Path $logs "$Tag.log.err"

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class SFResize {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll", CharSet = CharSet.Unicode, EntryPoint = "FindWindowW")]
  public static extern IntPtr FindWindowW(IntPtr cls, string title);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr wp, IntPtr lp);
  [DllImport("user32.dll")] public static extern uint MapVirtualKey(uint uCode, uint uMapType);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@
[SFResize]::SetProcessDPIAware() | Out-Null

function Kill-Game {
    Get-Process -Name steel-front -ErrorAction SilentlyContinue | ForEach-Object {
        try { $_.Kill(); $_.WaitForExit(5000) | Out-Null } catch {}
    }
}

function Post-Key([IntPtr]$h, [int]$vk) {
    $scan = [int64][SFResize]::MapVirtualKey([uint32]$vk, 0)      # MAPVK_VK_TO_VSC
    $base = [int64](1 -bor ($scan -shl 16))
    [SFResize]::PostMessage($h, 0x0100, [IntPtr]$vk, [IntPtr]$base) | Out-Null
    Start-Sleep -Milliseconds 80
    [SFResize]::PostMessage($h, 0x0101, [IntPtr]$vk, [IntPtr]([int64]($base -bor [int64]0xC0000000))) | Out-Null
}

Remove-Item $logOut, $logErr -ErrorAction SilentlyContinue
Kill-Game
Start-Sleep -Seconds 1

$env:RV3D_AUTOSTART = "1"
$env:RV3D_STRESS_AI = "1"
$env:RV3D_GPU = "dgpu"
$env:RV3D_PRESENT_MODE = "mailbox"
$env:RV3D_VALIDATION = "1"
$env:DISABLE_RTSS_LAYER = "1"
$env:DISABLE_GAMEPP_LAYER = "1"
if ($PT) {
    $env:RV3D_PT_LIVE = "1"
    $env:RV3D_PT_SIZE = "512"
    $env:RV3D_PT_SPP = "16"
    Write-Host "PT live path enabled (RV3D_PT_LIVE=1 RV3D_PT_SIZE=512 RV3D_PT_SPP=16)"
}

$exitNote = "ok"
try {
    $proc = Start-Process -FilePath $exe -WorkingDirectory $repo -PassThru `
        -RedirectStandardOutput $logOut -RedirectStandardError $logErr
    Write-Host "PID $($proc.Id) launched (validation layer on, implicit layers off)"
    Start-Sleep -Seconds $WarmupSec

    $h = [IntPtr]::Zero
    for ($i = 0; $i -lt 40; $i++) {
        $h = [SFResize]::FindWindowW([IntPtr]::Zero, "Steel Front - Vulkan")
        if ($h -ne [IntPtr]::Zero) { break }
        Start-Sleep -Milliseconds 250
    }
    if ($h -eq [IntPtr]::Zero) {
        $exitNote = "NO WINDOW HANDLE"
        Write-Host "!! $exitNote"
    } else {
        foreach ($s in $Sizes.Split(",")) {
            $wh = $s.Trim().Split("x")
            $w = [int]$wh[0]; $ht = [int]$wh[1]
            # SWP_NOMOVE|SWP_NOZORDER|SWP_NOACTIVATE
            [void][SFResize]::SetWindowPos($h, [IntPtr]::Zero, 0, 0, $w, $ht, 0x0002 -bor 0x0004 -bor 0x0010)
            $r = New-Object SFResize+RECT
            [void][SFResize]::GetClientRect($h, [ref]$r)
            Write-Host ("resize -> {0}x{1} ; client {2}x{3}" -f $w, $ht, ($r.R - $r.L), ($r.B - $r.T))
            Start-Sleep -Seconds 2
        }
        if (-not $NoShot) {
            Write-Host "POST VK 123 (F12: engine-side screenshot after resizes)"
            Post-Key $h 123
        }
        Start-Sleep -Seconds $AfterSecs
    }
}
catch {
    $exitNote = "HARNESS ERROR: $($_.Exception.Message)"
    Write-Host "!! $exitNote"
}
finally {
    Kill-Game
}

# ---- verdict: count VUIDs / resize lines straight from the engine log -------------
$txt = ""
if (Test-Path $logErr) { $txt = [System.IO.File]::ReadAllText($logErr) }
$vuid = ([regex]::Matches($txt, "VUID-")).Count
$resize = ([regex]::Matches($txt, "\u7a97\u53e3\u5927\u5c0f\u53d8\u5316")).Count   # window size change
$recreate = ([regex]::Matches($txt, "size mismatch")).Count
$shot = ([regex]::Matches($txt, "\u622a\u56fe\u5df2\u4fdd\u5b58")).Count           # screenshot saved (.NET regex \u escape)
$lost = ([regex]::Matches($txt, "has been lost")).Count
$panic = ([regex]::Matches($txt, "panicked")).Count
# Proves the live PT path actually engaged: without it a "VUID: 0" verdict from -PT
# would mean "the thing I meant to exercise never ran" (lesson 27).
$ptres = ([regex]::Matches($txt, "PT-RESIDENT:")).Count
Write-Host ""
Write-Host "=== resize probe summary ($Tag) ==="
Write-Host "  exit note    : $exitNote"
Write-Host "  resizes seen : $resize (window events) / $recreate (size-mismatch rebuilds)"
Write-Host "  VUID         : $vuid"
Write-Host "  screenshots  : $shot (engine-side saves; 0 with -NoShot)"
Write-Host "  PT resident  : $ptres (needs >=1 when -PT was given)"
Write-Host "  device lost  : $lost ; panics: $panic"
Write-Host "  log          : $logErr"
$ptOk = ((-not $PT) -or ($ptres -ge 1))
# 2026-09-26: must prove the rebuild path actually ran. This probe exists only to drive
# swapchain recreation; zero "window size change" lines means the path was never walked,
# and then VUID=0 only means "nothing happened" -- same criterion as the PT-RESIDENT>=1
# check above, and the same shape as the "scan surface = 0 counts as clean" tools (§21.48).
# (ASCII only at end-of-line: PS 5.1 reads BOM-less .ps1 as ANSI/GBK, and a trailing CJK
#  byte eats the newline itself -- see AGENTS.md 铁律 G.)
$ranOk = ($resize -ge 1)
if (-not (Test-Path $logErr)) {
    Write-Host "  !! engine log is missing: $logErr -- not a pass, the run never happened"
}
Write-Host "  resizes ok   : $ranOk (must be true: at least one window-size change)"
if ($vuid -eq 0 -and $lost -eq 0 -and $panic -eq 0 -and $ptOk -and $ranOk) { Write-Host "RESULT: ALL-OK" } else { Write-Host "RESULT: CHECK" }
