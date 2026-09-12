# pm_play.ps1 - PostMessage-based takeover for steel-front. Pure ASCII on purpose.
#
# Why PostMessage and not SendInput
# ---------------------------------
# Measured on this machine (2026-09-12, scripts/input_probe.ps1):
#   * the game's UI thread reports hwndActive == hwndFocus == its own window
#   * but GetForegroundWindow() returns a DIFFERENT process (the browser)
#   * SendInput accepted 6/6 events and the game observed none of them
#   * PostMessage delivered the same key and the game logged a weapon switch
# SendInput feeds the foreground window's input queue; PostMessage goes straight
# into the target window's queue. So this script never touches the foreground at
# all -- which also means it never takes the cursor grab, never calls ClipCursor
# and never hides the pointer. The user keeps their mouse for the whole run.
#
# How the look works without the cursor grab
# ------------------------------------------
# main.rs::window_event has an uncaptured drag path (the source comment there
# calls it the smoke test's focus-free aiming path): while `dragging` is set --
# which WM_LBUTTONDOWN sets -- a CursorMoved event drives camera.look(). That path
# also warps the real cursor back to the window centre after every event and sets
# last_cursor to that centre, so posting position (centre + d) yields a look delta
# of exactly d. Steps stay under MAX_LOOK_DELTA_PX (512) so they are not treated
# as teleports.
#
# Safety: try/finally always kills the game and then runs release_input.ps1,
# which VERIFIES the handback rather than inferring it.
param(
    [int]$WarmupSec  = 10,
    [int]$TurnPx     = 400,
    [int]$WalkMs     = 1500,
    [int]$Shots      = 2,
    [int]$HardSec    = 60,
    [string]$Tag     = "pmplay"
)

$ErrorActionPreference = "Continue"
$repo = "D:\Rust\steel-front"
$exe  = Join-Path $repo "target\release\steel-front.exe"
$shotDir = Join-Path $repo "screenshots"
$logs = Join-Path $repo "logs"
New-Item -ItemType Directory -Force -Path $shotDir, $logs | Out-Null
$logOut = Join-Path $logs "$Tag.log"
$logErr = Join-Path $logs "$Tag.log.err"

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Pm {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll", CharSet = CharSet.Unicode, EntryPoint = "FindWindowW")]
  public static extern IntPtr FindWindowW(IntPtr cls, string title);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
  [DllImport("user32.dll", CharSet = CharSet.Unicode, EntryPoint = "PostMessageW")]
  public static extern bool PostMessageW(IntPtr h, uint msg, IntPtr w, IntPtr l);

  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L,T,R,B; }

  public const uint WM_MOUSEMOVE = 0x0200, WM_LBUTTONDOWN = 0x0201, WM_LBUTTONUP = 0x0202;
  public const uint WM_KEYDOWN = 0x0100, WM_KEYUP = 0x0101;
  public const int MK_LBUTTON = 0x0001;
}
"@
[Pm]::SetProcessDPIAware() | Out-Null

function Kill-Game {
    Get-Process -Name steel-front -ErrorAction SilentlyContinue | ForEach-Object {
        try { $_.Kill(); $_.WaitForExit(5000) | Out-Null } catch {}
    }
}

function Count-Pat($pat) {
    try { return (Select-String -Path $logErr -Pattern $pat -ErrorAction Stop | Measure-Object).Count }
    catch { return 0 }
}

# The game logs "cam: yaw=.. pitch=.." about once a second, so the game itself
# reports whether the camera moved. This is the pass/fail signal, not a screenshot.
function Read-Cam {
    try { $m = Select-String -Path $logErr -Pattern 'cam: yaw=([-\d.]+) pitch=([-\d.]+)' -ErrorAction Stop }
    catch { return $null }
    if (-not $m) { return $null }
    $g = $m[-1].Matches[0].Groups
    return [pscustomobject]@{ Yaw = [double]$g[1].Value; Samples = $m.Count }
}

function Shot([IntPtr]$h, [string]$out) {
    $r = New-Object Pm+RECT
    [Pm]::GetWindowRect($h, [ref]$r) | Out-Null
    $w = $r.R - $r.L; $ht = $r.B - $r.T
    if ($w -lt 16 -or $ht -lt 16) { return $false }
    $bmp = New-Object System.Drawing.Bitmap($w, $ht)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $dc = $g.GetHdc(); [Pm]::PrintWindow($h, $dc, 2) | Out-Null; $g.ReleaseHdc($dc); $g.Dispose()
    $bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
    return $true
}

function Pack-XY([int]$x, [int]$y) {
    return [IntPtr]([int64](($y -band 0xFFFF) -shl 16) -bor ($x -band 0xFFFF))
}

function Post-Move([IntPtr]$h, [int]$x, [int]$y) {
    return [Pm]::PostMessageW($h, [Pm]::WM_MOUSEMOVE, [IntPtr][Pm]::MK_LBUTTON, (Pack-XY $x $y))
}

function Post-LButton([IntPtr]$h, [bool]$down, [int]$x, [int]$y) {
    if ($down) { return [Pm]::PostMessageW($h, [Pm]::WM_LBUTTONDOWN, [IntPtr][Pm]::MK_LBUTTON, (Pack-XY $x $y)) }
    return [Pm]::PostMessageW($h, [Pm]::WM_LBUTTONUP, [IntPtr]0, (Pack-XY $x $y))
}

# lParam carries the scan code in bits 16-23 because that is what winit turns
# into PhysicalKey::Code. Bits 30/31 mark the previous-state and transition flags
# on key up.
function Post-Key([IntPtr]$h, [int]$vk, [int]$scan, [bool]$up) {
    $lp = [int64](1 -bor ($scan -shl 16))
    if ($up) { $lp = $lp -bor (1 -shl 30) -bor (1 -shl 31) }
    $msg = if ($up) { [Pm]::WM_KEYUP } else { [Pm]::WM_KEYDOWN }
    return [Pm]::PostMessageW($h, $msg, [IntPtr]$vk, [IntPtr]$lp)
}

# Set 1 scan codes / VK pairs
$SC_W = 0x11; $VK_W = 0x57
$SC_R = 0x13; $VK_R = 0x52
$SC_2 = 0x03; $VK_2 = 0x32

Kill-Game
Start-Sleep -Seconds 1
Remove-Item $logOut, $logErr -ErrorAction SilentlyContinue

$env:RV3D_AUTOSTART = "1"
# main.rs defaults RV3D_STRESS_AI to "128", and stress mode makes the player an
# invincible spectator -- not a playable state. The repo's smoke pins it to "0".
$env:RV3D_STRESS_AI = "0"

$note = "ok"
$started = Get-Date
try {
    # -WindowStyle Hidden keeps the console-subsystem binary from spawning a
    # console window that competes for the foreground.
    Start-Process -FilePath $exe -WorkingDirectory $repo -PassThru -WindowStyle Hidden `
        -RedirectStandardOutput $logOut -RedirectStandardError $logErr | Out-Null
    Write-Host "launched; HARD CAP ${HardSec}s from now"
    Start-Sleep -Seconds $WarmupSec
    if ((Get-Date) - $started -gt [TimeSpan]::FromSeconds($HardSec)) { throw "hard cap hit during warmup" }

    $h = [IntPtr]::Zero
    for ($i = 0; $i -lt 40; $i++) {
        $h = [Pm]::FindWindowW([IntPtr]::Zero, "Steel Front - Vulkan")
        if ($h -ne [IntPtr]::Zero) { break }
        Start-Sleep -Milliseconds 250
    }
    if ($h -eq [IntPtr]::Zero) { throw "window 'Steel Front - Vulkan' never appeared" }
    Write-Host ("hwnd=0x{0:X} enabled={1} visible={2}" -f ([int64]$h), [Pm]::IsWindowEnabled($h), [Pm]::IsWindowVisible($h))

    # Deliberately NOT focusing. PostMessage does not need the foreground, and
    # leaving the game unfocused means it never grabs or clips the pointer.
    Write-Host ("foreground is the game ? {0}  (irrelevant for PostMessage)" -f ([Pm]::GetForegroundWindow() -eq $h))
    Write-Host ("cursor captured by game ? {0}  (expect False -> user keeps the pointer)" -f ((Count-Pat 'input: cursor captured') -gt (Count-Pat 'input: cursor released')))

    $cr = New-Object Pm+RECT
    [Pm]::GetClientRect($h, [ref]$cr) | Out-Null
    $cw = $cr.R - $cr.L; $ch = $cr.B - $cr.T
    $cx = [int]($cw / 2); $cy = [int]($ch / 2)
    Write-Host ("client {0}x{1}  centre {2},{3}" -f $cw, $ch, $cx, $cy)

    $cam0 = Read-Cam
    $yaw0 = if ($cam0) { $cam0.Yaw } else { [double]::NaN }
    $a = Join-Path $shotDir "${Tag}_before.png"
    Shot $h $a | Out-Null
    Write-Host "shot A: $a"

    # --- keyboard probe first (no pointer involved at all) ----------------
    $sw0 = Count-Pat 'weapons: '
    Post-Key $h $VK_2 $SC_2 $false | Out-Null
    Start-Sleep -Milliseconds 120
    Post-Key $h $VK_2 $SC_2 $true | Out-Null
    Start-Sleep -Milliseconds 900
    $sw1 = Count-Pat 'weapons: '
    Write-Host ("keyboard Digit2 (PostMessage): weapon-switch log {0} -> {1}" -f $sw0, $sw1)

    # --- look via the uncaptured drag path --------------------------------
    # The drag path computes its first delta against a STALE last_cursor (whatever
    # the last real CursorMoved left there), and only then warps the pointer to the
    # window centre and rebases last_cursor to that centre. So the first posted
    # move is a PRIMER whose delta is arbitrary; every move after it yields exactly
    # the requested delta. The baseline is therefore sampled after the primer.
    Write-Host "look: LMB down, prime, then ${TurnPx}px right in <=400px steps, LMB up"
    Post-LButton $h $true $cx $cy | Out-Null
    Start-Sleep -Milliseconds 150
    Post-Move $h $cx $cy | Out-Null
    Start-Sleep -Milliseconds 250
    $camP = Read-Cam
    $yawP = if ($camP) { $camP.Yaw } else { [double]::NaN }
    Write-Host ("  primer applied; baseline yaw={0}" -f $yawP)

    # winit dedups WM_MOUSEMOVE by position (event_loop.rs:1692,
    # `cursor_moved = last_position != Some(position)`), so consecutive steps MUST
    # post distinct coordinates or they are silently dropped. Because the drag path
    # rebases last_cursor to the window centre after every event, posting
    # (centre + cumulative) yields a look delta of exactly `step` each time.
    # The cumulative offset has to stay inside the client rect, which caps a single
    # sweep at about half the client width.
    $maxOff = [int]($cw / 2) - 8
    $want = [Math]::Min($TurnPx, $maxOff)
    if ($want -ne $TurnPx) { Write-Host ("  (turn clamped to {0}px so the posted position stays inside the client rect)" -f $want) }
    $off = 0
    $rx = $want
    while ($rx -ne 0) {
        $st = [Math]::Max(-400, [Math]::Min(400, $rx))
        $off += $st
        Post-Move $h ($cx + $off) $cy | Out-Null
        Start-Sleep -Milliseconds 60
        $rx -= $st
    }
    Start-Sleep -Milliseconds 200
    Post-LButton $h $false ($cx + $off) $cy | Out-Null
    Start-Sleep -Milliseconds 1400
    $cam1 = Read-Cam
    $yaw1 = if ($cam1) { $cam1.Yaw } else { [double]::NaN }
    $dYaw = $yaw1 - $yawP
    # yaw -= dx * mouse_sens, mouse_sens = 0.0005 + sensitivity*0.002 (game.rs::sensitivity_rads)
    $sens = 0.0005 + 0.992 * 0.002
    $expect = -1.0 * $want * $sens
    Write-Host ("cam after look: yaw={0}  (measured delta {1:+0.00;-0.00;0.00}, expected {2:+0.00;-0.00;0.00} rad)" -f $yaw1, $dYaw, $expect)

    # --- walk forward -----------------------------------------------------
    Write-Host "walk forward ${WalkMs}ms (W)"
    Post-Key $h $VK_W $SC_W $false | Out-Null
    Start-Sleep -Milliseconds $WalkMs
    Post-Key $h $VK_W $SC_W $true | Out-Null
    Start-Sleep -Milliseconds 300

    # --- fire (LMB tap; a held LMB is the drag path, so tap instead) ------
    Write-Host "fire $Shots tap(s) (LMB)"
    for ($i = 0; $i -lt $Shots; $i++) {
        Post-LButton $h $true $cx $cy | Out-Null
        Start-Sleep -Milliseconds 90
        Post-LButton $h $false $cx $cy | Out-Null
        Start-Sleep -Milliseconds 260
    }

    Start-Sleep -Milliseconds 1200
    $b = Join-Path $shotDir "${Tag}_after.png"
    Shot $h $b | Out-Null
    Write-Host "shot B: $b"

    $mouseOk = ($dYaw -eq $dYaw) -and ([Math]::Abs($dYaw) -gt 0.5)
    $calib = ($dYaw -eq $dYaw) -and ([Math]::Abs($dYaw - $expect) -lt [Math]::Abs($expect) * 0.25)
    $keysOk = ($sw1 -gt $sw0)
    $mTxt = if ($mouseOk) { "OK" } else { "FAIL" }
    $kTxt = if ($keysOk) { "OK" } else { "FAIL" }
    Write-Host ""
    Write-Host "=== channel evidence ==="
    Write-Host ("  keyboard (weapon switch) : {0}   log {1} -> {2}" -f $kTxt, $sw0, $sw1)
    Write-Host ("  look     (cam yaw)       : {0}   yaw {1} -> {2}  (delta {3:+0.00;-0.00;0.00}, expected {4:+0.00;-0.00;0.00})" -f $mTxt, $yawP, $yaw1, $dYaw, $expect)
    Write-Host ("  look calibration         : {0}  (within +/-25% of the predicted turn)" -f $(if ($calib) { "MATCHES" } else { "OFF" }))
    $note = "keys=$kTxt look=$mTxt calib=$calib"
}
catch {
    $note = "ABORTED: $($_.Exception.Message)"
    Write-Host "!! $note"
}
finally {
    Kill-Game
    Remove-Item Env:RV3D_AUTOSTART -ErrorAction SilentlyContinue
    Remove-Item Env:RV3D_STRESS_AI -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 400
    Write-Host "--- handing input back ---"
    & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $repo "scripts\release_input.ps1")
    Write-Host ("elapsed {0:N1}s" -f ((Get-Date) - $started).TotalSeconds)
}

Write-Host "=== RESULT: $note ==="
