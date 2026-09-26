# Weapon-switch probe under the validation layer (ASCII only: PS 5.1 reads BOM-less .ps1 as ANSI).
#
# Why this exists (2026-09-26): the gun-mesh upload path is the one that has actually killed the
# device twice in this project's history (2026-09-15: pressing "2" switched to a SMALLER gun,
# the old code recreated the buffer, and destroying an in-flight buffer returned
# VK_ERROR_DEVICE_LOST). The fix (grow only when `need > capacity`, create-new-before-destroy-old)
# has unit tests, but nobody had ever driven the real switch path under the validation layer --
# every validation run so far started once and never touched the weapon rack.
#
# INPUT SAFETY (AGENTS iron rule C): PostMessage only. No SendInput, no SetForegroundWindow,
# no SetCursorPos, no AttachThreadInput. The window is never brought to the foreground, so this
# probe does not disturb whatever the user is doing. Window handle comes from
# FindWindowW(None, "Steel Front - Vulkan") -- `Process.MainWindowHandle` is NOT the window that
# receives the input for winit.
#
# It also counts the engine's weapon-switch log lines, so "the probe ran" and "the game
# actually switched" are separate facts (lesson 27: prove the tool measured what you think).
#
# Usage:
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\run_weapon_probe.ps1
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\run_weapon_probe.ps1 -NoShot
param(
    [string]$Tag = "weapon_probe",
    [int]$WarmupSec = 12,
    [int]$SlotHoldMs = 1400,
    [int]$AfterSecs = 6,
    [switch]$NoShot,
    # -Wheel also posts WM_MOUSEWHEEL (+/- one notch): the second, separate switch code path.
    [switch]$Wheel
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
public class SFWpn {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll", CharSet = CharSet.Unicode, EntryPoint = "FindWindowW")]
  public static extern IntPtr FindWindowW(IntPtr cls, string title);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr wp, IntPtr lp);
  [DllImport("user32.dll")] public static extern uint MapVirtualKey(uint uCode, uint uMapType);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
}
"@
[SFWpn]::SetProcessDPIAware() | Out-Null

function Kill-Game {
    Get-Process -Name steel-front -ErrorAction SilentlyContinue | ForEach-Object {
        try { $_.Kill(); $_.WaitForExit(5000) | Out-Null } catch {}
    }
}

# Live count of the engine's switch lines: lets the probe tell the two switch code paths
# (number keys vs mouse wheel) apart instead of reporting one lump sum.
# The `weapons: ` prefix matters: the wheel tag is a SUPERSET of the digit tag (it has a
# two-character prefix), so a pattern without the prefix would count both paths into one bucket.
# NOTE: the engine process still holds the redirect target open, and `File.ReadAllText` uses
# FileShare.Read -> "file is being used by another process". Open it with FileShare.ReadWrite.
function Read-Shared([string]$path) {
    if (-not (Test-Path $path)) { return "" }
    try {
        $fs = New-Object System.IO.FileStream($path, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::ReadWrite)
        $sr = New-Object System.IO.StreamReader($fs, [System.Text.Encoding]::UTF8)
        $t = $sr.ReadToEnd()
        $sr.Dispose(); $fs.Dispose()
        return $t
    } catch {
        return ""
    }
}

# Number keys: `weapons: <tag> 3 -> 4 (name)`. `\u5207\u67aa` = the direct tag,
# `\u6eda\u8f6e\u5207\u67aa` = the wheel tag. Both are matched with the `weapons: ` prefix.
$RE_DIGIT = "weapons: \u5207\u67aa \d+ -> \d+"
$RE_WHEEL = "weapons: \u6eda\u8f6e\u5207\u67aa \d+ -> \d+"

function Get-Count([string]$path, [string]$pattern) {
    return ([regex]::Matches((Read-Shared $path), $pattern)).Count
}

function Post-Key([IntPtr]$h, [int]$vk) {
    $scan = [int64][SFWpn]::MapVirtualKey([uint32]$vk, 0)      # MAPVK_VK_TO_VSC
    $base = [int64](1 -bor ($scan -shl 16))
    [SFWpn]::PostMessage($h, 0x0100, [IntPtr]$vk, [IntPtr]$base) | Out-Null
    Start-Sleep -Milliseconds 60
    [SFWpn]::PostMessage($h, 0x0101, [IntPtr]$vk, [IntPtr]([int64]($base -bor [int64]0xC0000000))) | Out-Null
}

function Post-Wheel([IntPtr]$h, [int]$delta) {
    # wParam high word = signed notch delta; lParam = screen coords of the cursor.
    # Post the client-area centre: winit derives its wheel position from lParam, and (0,0)
    # (what the 2026-09-13 probe posted) is outside the window.
    $r = New-Object SFWpn+RECT
    [void][SFWpn]::GetClientRect($h, [ref]$r)
    $pt = New-Object SFWpn+POINT
    $pt.X = [int](($r.R - $r.L) / 2); $pt.Y = [int](($r.B - $r.T) / 2)
    [void][SFWpn]::ClientToScreen($h, [ref]$pt)
    $lp = [int64]((($pt.Y -band 0xFFFF) -shl 16) -bor ($pt.X -band 0xFFFF))
    [SFWpn]::PostMessage($h, 0x020A, [IntPtr]([int64](($delta -band 0xFFFF) -shl 16)), [IntPtr]$lp) | Out-Null
}

Remove-Item $logOut, $logErr -ErrorAction SilentlyContinue
Kill-Game
Start-Sleep -Seconds 1

# Autostart goes straight into the city map; RV3D_AUTOSTART does NOT skip the menu, it starts
# the match, which is what puts the game into Playing (the state switch_weapon() requires).
$env:RV3D_AUTOSTART = "1"
$env:RV3D_GPU = "dgpu"
$env:RV3D_PRESENT_MODE = "mailbox"
$env:RV3D_VALIDATION = "1"
$env:DISABLE_RTSS_LAYER = "1"
$env:DISABLE_GAMEPP_LAYER = "1"

$exitNote = "ok"
$shots = 0
try {
    $proc = Start-Process -FilePath $exe -WorkingDirectory $repo -PassThru `
        -RedirectStandardOutput $logOut -RedirectStandardError $logErr
    Write-Host "PID $($proc.Id) launched (validation layer on, implicit layers off)"
    Start-Sleep -Seconds $WarmupSec

    $h = [IntPtr]::Zero
    for ($i = 0; $i -lt 40; $i++) {
        $h = [SFWpn]::FindWindowW([IntPtr]::Zero, "Steel Front - Vulkan")
        if ($h -ne [IntPtr]::Zero) { break }
        Start-Sleep -Milliseconds 250
    }
    if ($h -eq [IntPtr]::Zero) {
        $exitNote = "NO WINDOW HANDLE"
        Write-Host "!! $exitNote"
    } else {
        # VK_1..VK_9 = 0x31..0x39 : the number-key switch (main.rs maps them to rack slots 0..8).
        foreach ($vk in 0x31..0x39) {
            Write-Host ("POST VK {0} (weapon slot {1})" -f $vk, ($vk - 0x30))
            Post-Key $h $vk
            Start-Sleep -Milliseconds $SlotHoldMs
            if (-not $NoShot) {
                # F12 -> engine-side readback of the swapchain (PNG under screenshots\).
                Post-Key $h 123
                Start-Sleep -Milliseconds 600
            }
        }
        $digits = Get-Count $logErr $RE_DIGIT
        Write-Host "switches after 9 digit keys: $digits"
        if ($Wheel) {
            Write-Host "POST WM_MOUSEWHEEL +120 / -120"
            Post-Wheel $h 120;  Start-Sleep -Milliseconds $SlotHoldMs
            Post-Wheel $h -120; Start-Sleep -Milliseconds $SlotHoldMs
            $wheelSwitches = (Get-Count $logErr $RE_WHEEL)
            Write-Host "switches caused by the two wheel notches: $wheelSwitches (0 = the wheel path did not fire)"
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

# ---- verdict: read the engine log, count VUIDs and the switches it actually performed -------
$txt = Read-Shared $logErr
$vuid = ([regex]::Matches($txt, "VUID-")).Count
$lost = ([regex]::Matches($txt, "has been lost")).Count
$panic = ([regex]::Matches($txt, "panicked")).Count
# Distinct saved filenames: one F12 logs the same path twice, so counting log lines would
# report twice as many screenshots as there are files.
$shot = @([regex]::Matches($txt, "steel_front_\d+\.png") | ForEach-Object { $_.Value } | Sort-Object -Unique).Count
$switched = ([regex]::Matches($txt, $RE_DIGIT)).Count                 # number-key switches
$wheelSw = ([regex]::Matches($txt, $RE_WHEEL)).Count                  # wheel switches
$oor = ([regex]::Matches($txt, "\u5207\u67aa\u56de\u9000")).Count                     # slot out of range
$gunGrow = ([regex]::Matches($txt, "\u67aa\u6a21\u7f13\u51b2\u533a|\u67aa\u6a21\u7f13\u51b2")).Count  # gun buffer (re)alloc
Write-Host ""
Write-Host "=== weapon probe summary ($Tag) ==="
Write-Host "  exit note    : $exitNote"
Write-Host "  VUID         : $vuid"
Write-Host "  switches     : $switched by number key, $wheelSw by mouse wheel ; out-of-range: $oor"
Write-Host "  gun buffer   : $gunGrow (alloc lines; 0 = buffers were reused, no destroy/create)"
Write-Host "  screenshots  : $shot (distinct engine-side PNGs; 0 with -NoShot)"
Write-Host "  device lost  : $lost ; panics: $panic"
Write-Host "  log          : $logErr"
# Prove the probe really drove the rack: 9 digit keys, of which at most one is a no-op
# (pressing the already-active slot logs nothing) => at least 7 real switches expected.
$drove = ($switched + $oor) -ge 7
if (-not $drove) { Write-Host "!! only $switched switches: the injected keys did not reach the game" }
$clean = ($exitNote -eq "ok") -and $vuid -eq 0 -and $lost -eq 0 -and $panic -eq 0 -and $drove
if ($clean) { Write-Host "RESULT: ALL-OK" } else { Write-Host "RESULT: CHECK" }
