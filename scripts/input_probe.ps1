# input_probe.ps1 - minimal injection bisection for steel-front. Pure ASCII on purpose.
#
# Why this exists
# ---------------
# Both ai_play.ps1 and the repo's own run_gameplay_smoke.ps1 fail to move the
# camera on this machine: 28 consecutive `cam: yaw=0.0` readings, kills=0, while
# the game reports `input: cursor captured (grab=confined, look=absolute)`.
# SendInput returns 1 for every event (nothing is refused), so the events enter
# the system but the game never observes them. This probe answers ONE question:
# does the game's own thread consider its window focused and able to take input?
#
# It reports, for the game's UI thread:
#   hwndActive   - the thread's active window
#   hwndFocus    - the window that actually holds keyboard focus
#   hwndCapture  - the window holding mouse capture
# and then delivers the same key twice, by two different routes:
#   SendInput   - the normal route, requires the foreground window to be ours
#   PostMessage - posts WM_KEYDOWN/UP straight into the window's message queue
# If PostMessage works and SendInput does not, the problem is foreground routing,
# not the game.
#
# Safety: same contract as ai_play.ps1 - try/finally kills the game, then
# release_input.ps1 VERIFIES the handback.
param(
    [int]$WarmupSec = 10,
    [string]$Tag    = "probe"
)

$ErrorActionPreference = "Continue"
$repo = "D:\Rust\steel-front"
$exe  = Join-Path $repo "target\release\steel-front.exe"
$logs = Join-Path $repo "logs"
New-Item -ItemType Directory -Force -Path $logs | Out-Null
$logOut = Join-Path $logs "$Tag.log"
$logErr = Join-Path $logs "$Tag.log.err"

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Probe {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll", CharSet = CharSet.Unicode, EntryPoint = "FindWindowW")]
  public static extern IntPtr FindWindowW(IntPtr cls, string title);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern uint SendInput(uint n, INPUT[] inputs, int size);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, IntPtr pid);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
  [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool f);
  [DllImport("user32.dll")] public static extern bool GetGUIThreadInfo(uint tid, ref GUITHREADINFO g);
  [DllImport("user32.dll", CharSet = CharSet.Unicode, EntryPoint = "PostMessageW")]
  public static extern bool PostMessageW(IntPtr h, uint msg, IntPtr w, IntPtr l);

  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L,T,R,B; }
  [StructLayout(LayoutKind.Sequential)] public struct GUITHREADINFO {
    public int cbSize; public int flags;
    public IntPtr hwndActive, hwndFocus, hwndCapture;
    public IntPtr hwndMenuOwner, hwndMoveSize, hwndCaret;
    public RECT rcCaret;
  }
  [StructLayout(LayoutKind.Sequential)] public struct MOUSEINPUT { public int dx, dy; public uint mouseData, dwFlags, time; public IntPtr extra; }
  [StructLayout(LayoutKind.Sequential)] public struct KEYBDINPUT { public ushort vk, scan; public uint flags, time; public IntPtr extra; }
  [StructLayout(LayoutKind.Explicit)] public struct INPUTUNION {
    [FieldOffset(0)] public MOUSEINPUT mi;
    [FieldOffset(0)] public KEYBDINPUT ki;
  }
  [StructLayout(LayoutKind.Sequential)] public struct INPUT { public uint type; public INPUTUNION u; }

  public const uint INPUT_MOUSE = 0, INPUT_KEYBOARD = 1;
  public const uint MOUSEEVENTF_MOVE = 0x0001;
  public const uint KEYEVENTF_KEYUP = 0x0002, KEYEVENTF_SCANCODE = 0x0008;
  public const uint WM_KEYDOWN = 0x0100, WM_KEYUP = 0x0101;
}
"@
[Probe]::SetProcessDPIAware() | Out-Null

function Kill-Game {
    Get-Process -Name steel-front -ErrorAction SilentlyContinue | ForEach-Object {
        try { $_.Kill(); $_.WaitForExit(5000) | Out-Null } catch {}
    }
}

function Count-Pat($pat) {
    try { return (Select-String -Path $logErr -Pattern $pat -ErrorAction Stop | Measure-Object).Count }
    catch { return 0 }
}

function Read-Cam {
    try { $m = Select-String -Path $logErr -Pattern 'cam: yaw=([-\d.]+) pitch=([-\d.]+)' -ErrorAction Stop }
    catch { return $null }
    if (-not $m) { return $null }
    $g = $m[-1].Matches[0].Groups
    return [pscustomobject]@{ Yaw = [double]$g[1].Value; Samples = $m.Count }
}

function Send-KeyInput([int]$scan, [bool]$up) {
    $i = New-Object Probe+INPUT
    $i.type = [Probe]::INPUT_KEYBOARD
    $i.u.ki.vk = 0
    $i.u.ki.scan = [uint16]$scan
    $f = [Probe]::KEYEVENTF_SCANCODE
    if ($up) { $f = $f -bor [Probe]::KEYEVENTF_KEYUP }
    $i.u.ki.flags = [uint32]$f
    return [Probe]::SendInput(1, @($i), [System.Runtime.InteropServices.Marshal]::SizeOf($i))
}

function Send-Move([int]$dx, [int]$dy) {
    $i = New-Object Probe+INPUT
    $i.type = [Probe]::INPUT_MOUSE
    $i.u.mi.dx = $dx; $i.u.mi.dy = $dy
    $i.u.mi.dwFlags = [Probe]::MOUSEEVENTF_MOVE
    return [Probe]::SendInput(1, @($i), [System.Runtime.InteropServices.Marshal]::SizeOf($i))
}

# Post a key straight into the window's queue. lParam carries the scan code in
# bits 16-23 because that is what winit turns into PhysicalKey::Code.
function Post-Key([IntPtr]$h, [int]$vk, [int]$scan, [bool]$up) {
    $lp = [int64](1 -bor ($scan -shl 16))
    if ($up) { $lp = $lp -bor (1 -shl 30) -bor (1 -shl 31) }
    $msg = if ($up) { [Probe]::WM_KEYUP } else { [Probe]::WM_KEYDOWN }
    return [Probe]::PostMessageW($h, $msg, [IntPtr]$vk, [IntPtr]$lp)
}

function Show-ThreadInfo([string]$label, [IntPtr]$h) {
    $tid = [Probe]::GetWindowThreadProcessId($h, [IntPtr]::Zero)
    $g = New-Object Probe+GUITHREADINFO
    $g.cbSize = [System.Runtime.InteropServices.Marshal]::SizeOf($g)
    [Probe]::GetGUIThreadInfo($tid, [ref]$g) | Out-Null
    Write-Host ("  [{0}] game thread id={1}" -f $label, $tid)
    Write-Host ("      hwndActive =0x{0:X} {1}" -f ([int64]$g.hwndActive), $(if ($g.hwndActive -eq $h) { "<== the game window" } else { "" }))
    Write-Host ("      hwndFocus  =0x{0:X} {1}" -f ([int64]$g.hwndFocus),  $(if ($g.hwndFocus  -eq $h) { "<== the game window" } else { "(NOT the game window)" }))
    Write-Host ("      hwndCapture=0x{0:X}" -f ([int64]$g.hwndCapture))
}

Kill-Game
Start-Sleep -Seconds 1
Remove-Item $logOut, $logErr -ErrorAction SilentlyContinue

$env:RV3D_AUTOSTART = "1"
$env:RV3D_STRESS_AI = "0"

try {
    Start-Process -FilePath $exe -WorkingDirectory $repo -WindowStyle Hidden `
        -RedirectStandardOutput $logOut -RedirectStandardError $logErr | Out-Null
    Write-Host "launched, waiting ${WarmupSec}s"
    Start-Sleep -Seconds $WarmupSec

    $h = [IntPtr]::Zero
    for ($i = 0; $i -lt 40; $i++) {
        $h = [Probe]::FindWindowW([IntPtr]::Zero, "Steel Front - Vulkan")
        if ($h -ne [IntPtr]::Zero) { break }
        Start-Sleep -Milliseconds 250
    }
    if ($h -eq [IntPtr]::Zero) { throw "window not found" }
    Write-Host ("hwnd=0x{0:X}  enabled={1} visible={2}" -f ([int64]$h), [Probe]::IsWindowEnabled($h), [Probe]::IsWindowVisible($h))

    # same focus routine as ai_play.ps1, so the game reaches the captured state
    for ($try = 0; $try -lt 15; $try++) {
        if ((Count-Pat 'input: cursor captured') -gt (Count-Pat 'input: cursor released')) { break }
        $fg = [Probe]::GetForegroundWindow()
        if ($fg -ne $h) {
            $ft = [Probe]::GetWindowThreadProcessId($fg, [IntPtr]::Zero)
            $me = [Probe]::GetCurrentThreadId()
            $at = $false
            if ($ft -ne 0 -and $ft -ne $me) { $at = [Probe]::AttachThreadInput($me, $ft, $true) }
            [Probe]::SetForegroundWindow($h) | Out-Null
            if ($at) { [Probe]::AttachThreadInput($me, $ft, $false) | Out-Null }
        }
        Start-Sleep -Milliseconds 400
    }
    Write-Host ("captured by game ? {0}" -f ((Count-Pat 'input: cursor captured') -gt (Count-Pat 'input: cursor released')))

    [Probe]::ShowWindow($h, 9) | Out-Null
    [Probe]::SetForegroundWindow($h) | Out-Null
    Start-Sleep -Milliseconds 300
    Write-Host ("foreground is game window ? {0}" -f ([Probe]::GetForegroundWindow() -eq $h))
    Show-ThreadInfo "before injection" $h

    $r = New-Object Probe+RECT
    [Probe]::GetWindowRect($h, [ref]$r) | Out-Null
    $cx = [int](($r.L + $r.R) / 2); $cy = [int](($r.T + $r.B) / 2)
    [Probe]::SetCursorPos($cx, $cy) | Out-Null
    Start-Sleep -Milliseconds 200
    Write-Host ("warped cursor to window center {0},{1}" -f $cx, $cy)

    $c0 = Read-Cam
    Write-Host ("cam before: yaw={0} samples={1}" -f $(if ($c0) { $c0.Yaw } else { "?" }), $(if ($c0) { $c0.Samples } else { 0 }))

    # ---- route 1: SendInput ----
    $acc = 0
    for ($i = 0; $i -lt 4; $i++) { $acc += Send-Move 100 0; Start-Sleep -Milliseconds 60 }
    $kacc = 0
    $sw0 = Count-Pat 'weapons: '
    $kacc += Send-KeyInput 0x03 $false; Start-Sleep -Milliseconds 120; $kacc += Send-KeyInput 0x03 $true
    Start-Sleep -Milliseconds 1200
    $c1 = Read-Cam
    $sw1 = Count-Pat 'weapons: '
    Write-Host ("ROUTE SendInput : moves accepted {0}/4  keys accepted {1}/2" -f $acc, $kacc)
    Write-Host ("                  yaw {0} -> {1}   weapon-switch log {2} -> {3}" -f $(if ($c0) { $c0.Yaw } else { "?" }), $(if ($c1) { $c1.Yaw } else { "?" }), $sw0, $sw1)

    # ---- route 2: PostMessage straight into the queue ----
    $p0 = Count-Pat 'weapons: '
    $p1 = Post-Key $h 0x32 0x03 $false
    Start-Sleep -Milliseconds 120
    $p2 = Post-Key $h 0x32 0x03 $true
    Start-Sleep -Milliseconds 1200
    $p3 = Count-Pat 'weapons: '
    Write-Host ("ROUTE PostMessage: posted {0}/{1}  weapon-switch log {2} -> {3}" -f $(if ($p1) { 1 } else { 0 }), $(if ($p2) { 1 } else { 0 }), $p0, $p3)

    Write-Host "=== VERDICT ==="
    Write-Host ("  SendInput  mouse : {0}" -f $(if ($c1 -and $c0 -and [Math]::Abs($c1.Yaw - $c0.Yaw) -gt 0.5) { "OK" } else { "FAIL" }))
    Write-Host ("  SendInput  keys  : {0}" -f $(if ($sw1 -gt $sw0) { "OK" } else { "FAIL" }))
    Write-Host ("  PostMessage keys : {0}" -f $(if ($p3 -gt $p0) { "OK" } else { "FAIL" }))
}
catch {
    Write-Host "!! $_"
}
finally {
    Kill-Game
    Remove-Item Env:RV3D_AUTOSTART -ErrorAction SilentlyContinue
    Remove-Item Env:RV3D_STRESS_AI -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 400
    & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $repo "scripts\release_input.ps1")
}
