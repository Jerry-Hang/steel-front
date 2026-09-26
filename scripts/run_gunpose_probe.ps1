# First-person gun POSE probe: capture one engine-side screenshot per pose (ASCII only).
#
# Why this exists (2026-09-26): the gun had recoil / walk sway / switch animation / ADS
# interpolation, but NO sprint pose, NO reload action and NO idle breathing -- so the three
# poses added that day need a visual before/after ruler. Static screenshots can only show one
# phase of an animation, so this probe pins the state (hold the keys, then F12) and produces
# one PNG per pose; compare them with tools/diff_gun_region.py (it crops the lower-right gun
# region and excludes the HUD strips, so a HUD change cannot fake a difference).
#
# INPUT SAFETY (AGENTS iron rule C): PostMessage only, no SendInput / SetForegroundWindow /
# SetCursorPos. Movement keys are HELD (keydown, no keyup) and released explicitly, so the
# player really walks / sprints while the frame is captured.
#
# Usage:
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\run_gunpose_probe.ps1
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\run_gunpose_probe.ps1 -Tag gunpose_before
param(
    [string]$Tag = "gunpose",
    [int]$WarmupSec = 12,
    [int]$PoseSecs = 2,
    [switch]$NoShot
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
public class SFPose {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll", CharSet = CharSet.Unicode, EntryPoint = "FindWindowW")]
  public static extern IntPtr FindWindowW(IntPtr cls, string title);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr wp, IntPtr lp);
  [DllImport("user32.dll")] public static extern uint MapVirtualKey(uint uCode, uint uMapType);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
}
"@
[SFPose]::SetProcessDPIAware() | Out-Null

$VK_W = 0x57; $VK_SHIFT = 0x10; $VK_R = 0x52; $VK_F12 = 123

function Kill-Game {
    Get-Process -Name steel-front -ErrorAction SilentlyContinue | ForEach-Object {
        try { $_.Kill(); $_.WaitForExit(5000) | Out-Null } catch {}
    }
}

# Held keys: keydown without a matching keyup (winit keeps the key pressed; the game keeps
# moving / sprinting until we release it).
function Key-Down([IntPtr]$h, [int]$vk) {
    $scan = [int64][SFPose]::MapVirtualKey([uint32]$vk, 0)
    [SFPose]::PostMessage($h, 0x0100, [IntPtr]$vk, [IntPtr]([int64](1 -bor ($scan -shl 16)))) | Out-Null
}
function Key-Up([IntPtr]$h, [int]$vk) {
    $scan = [int64][SFPose]::MapVirtualKey([uint32]$vk, 0)
    $base = [int64](1 -bor ($scan -shl 16) -bor [int64]0xC0000000)
    [SFPose]::PostMessage($h, 0x0101, [IntPtr]$vk, [IntPtr]$base) | Out-Null
}
function Tap-Key([IntPtr]$h, [int]$vk) {
    Key-Down $h $vk
    Start-Sleep -Milliseconds 60
    Key-Up $h $vk
}
# Left click at the client centre: fires one shot (the reload pose needs a non-full magazine --
# `start_reload()` is a no-op on a full magazine, which is exactly why the first version of this
# probe measured `reload=0.000` the whole run).
function Click-Fire([IntPtr]$h) {
    $r = New-Object SFPose+RECT
    [void][SFPose]::GetClientRect($h, [ref]$r)
    $pt = New-Object SFPose+POINT
    $pt.X = [int](($r.R - $r.L) / 2); $pt.Y = [int](($r.B - $r.T) / 2)
    [void][SFPose]::ClientToScreen($h, [ref]$pt)
    $lp = [IntPtr]([int64]((($pt.Y -band 0xFFFF) -shl 16) -bor ($pt.X -band 0xFFFF)))
    [SFPose]::PostMessage($h, 0x0201, [IntPtr]1, $lp) | Out-Null
    Start-Sleep -Milliseconds 40
    [SFPose]::PostMessage($h, 0x0202, [IntPtr]0, $lp) | Out-Null
}

Remove-Item $logOut, $logErr -ErrorAction SilentlyContinue
Kill-Game
Start-Sleep -Seconds 1

$env:RV3D_AUTOSTART = "1"
$env:RV3D_GPU = "dgpu"
$env:RV3D_PRESENT_MODE = "mailbox"
# Pose diagnostics: one `gundiag:` line per second with sprint/walk/reload/idle/ads numbers.
# This is the PRIMARY evidence (static screenshots can only show "the picture changed").
$env:RV3D_GUN_DIAG = "1"

$exitNote = "ok"
$poses = @()
try {
    $proc = Start-Process -FilePath $exe -WorkingDirectory $repo -PassThru `
        -RedirectStandardOutput $logOut -RedirectStandardError $logErr
    Write-Host "PID $($proc.Id) launched"
    Start-Sleep -Seconds $WarmupSec

    $h = [IntPtr]::Zero
    for ($i = 0; $i -lt 40; $i++) {
        $h = [SFPose]::FindWindowW([IntPtr]::Zero, "Steel Front - Vulkan")
        if ($h -ne [IntPtr]::Zero) { break }
        Start-Sleep -Milliseconds 250
    }
    if ($h -eq [IntPtr]::Zero) {
        $exitNote = "NO WINDOW HANDLE"
        Write-Host "!! $exitNote"
    } else {
        # pose name -> keys held while the shot is taken
        $poses = @(
            @{ name = "idle";   hold = @() },
            @{ name = "walk";   hold = @($VK_W) },
            @{ name = "sprint"; hold = @($VK_W, $VK_SHIFT) }
        )
        foreach ($p in $poses) {
            foreach ($vk in $p.hold) { Key-Down $h $vk }
            Start-Sleep -Seconds $PoseSecs
            if (-not $NoShot) {
                Tap-Key $h $VK_F12
                Start-Sleep -Milliseconds 700
            }
            foreach ($vk in $p.hold) { Key-Up $h $vk }
            Start-Sleep -Milliseconds 400
            Write-Host ("pose {0}: shot taken (held {1} key(s))" -f $p.name, $p.hold.Count)
        }
        # Reload: fire three shots first, then R starts a 2.3s AK-12M reload (full magazine
        # makes start_reload a no-op). Shoot the middle of it.
        for ($i = 0; $i -lt 3; $i++) {
            Click-Fire $h
            Start-Sleep -Milliseconds 180
        }
        Start-Sleep -Milliseconds 300
        Tap-Key $h $VK_R
        Start-Sleep -Milliseconds 900
        if (-not $NoShot) {
            Tap-Key $h $VK_F12
            Start-Sleep -Milliseconds 700
        }
        Write-Host "pose reload: fired 3, then R; shot taken ~0.9s into the reload"
        Start-Sleep -Seconds 2
    }
}
catch {
    $exitNote = "HARNESS ERROR: $($_.Exception.Message)"
    Write-Host "!! $exitNote"
}
finally {
    Kill-Game
}

$txt = ""
if (Test-Path $logErr) { $txt = [System.IO.File]::ReadAllText($logErr) }
$vuid = ([regex]::Matches($txt, "VUID-")).Count
$lost = ([regex]::Matches($txt, "has been lost")).Count
$panic = ([regex]::Matches($txt, "panicked")).Count
$shots = @([regex]::Matches($txt, "steel_front_\d+\.png") | ForEach-Object { $_.Value } | Sort-Object -Unique).Count
# Evidence that the poses were really driven: the engine logs the sprint state indirectly via
# `cam:` only, so instead we prove the frames exist and are distinct (the caller diffs them).
Write-Host ""
Write-Host "=== gun-pose probe summary ($Tag) ==="
Write-Host "  exit note    : $exitNote"
Write-Host "  poses        : idle / walk / sprint / reload"
Write-Host "  screenshots  : $shots distinct engine-side PNGs (expect 4 with -NoShot off)"
Write-Host "  VUID         : $vuid ; device lost: $lost ; panics: $panic"
Write-Host "  log          : $logErr"
# Primary ruler: the engine's own pose numbers, printed per second. `sprint` must reach ~1
# while W+Shift are held (and ~0 before), `walk_env` ~1 while W is held, `reload` ~1 in the
# middle of a reload. A pose that never moves here is not a working pose.
$gun = @([regex]::Matches($txt, "gundiag: sprint=[^\r\n]*") | ForEach-Object { $_.Value })
Write-Host ("  gundiag lines: {0}" -f $gun.Count)
foreach ($g in ($gun | Select-Object -Last 8)) { Write-Host ("    " + $g) }
$sprintSeen = @($gun | Where-Object { $_ -match "sprint=(0\.[89]\d*|1\.0)" }).Count
$reloadSeen = @($gun | Where-Object { $_ -match "reload=(0\.[5-9]\d*|1\.0)" }).Count
Write-Host ("  pose evidence : sprint>=0.8 seen {0}x ; reload>=0.5 seen {1}x" -f $sprintSeen, $reloadSeen)
Write-Host "  next step    : python tools\diff_gun_region.py <idle.png> <sprint.png> <reload.png>"
$ok = ($exitNote -eq "ok") -and $vuid -eq 0 -and $lost -eq 0 -and $panic -eq 0 -and $sprintSeen -ge 1 -and $reloadSeen -ge 1
if ($NoShot) { $ok = $ok -and ($shots -eq 0) } else { $ok = $ok -and ($shots -ge 4) }
if ($ok) { Write-Host "RESULT: ALL-OK" } else { Write-Host "RESULT: CHECK" }
