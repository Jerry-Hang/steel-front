param(
    [int]$WarmupSec = 10,
    [int]$HoldSec = 4,
    [string]$Tag = "cap",
    # ⚠ -Keys 收的是 **Windows 虚拟键码（VK）**，不是 winit 的 KeyCode 枚举序号。
    # 两套毫无关系：winit 的 KeyR=36，而 Windows 的 VK_R=82(0x52)，36 在 Windows 里是 VK_HOME。
    # 2026-09-12 之前混用过，症状是"POST VK 打印了、游戏零响应"。
    # 常用：R=82 C=67 Z=90 W=87 A=65 S=83 D=68 Space=32 Tab=9 Escape=27 Shift=16。
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
    Write-Host "SAVED $out (${w}x${ht})"
}

function Post-Key([IntPtr]$h, [int]$vk) {
    # lParam 的 bit16-23 必须是**扫描码**：winit 的 Windows 后端正是靠它把 WM_KEYDOWN
    # 解析成 KeyCode 的。2026-09-12 之前这里只发 `1 | bit30`（无扫描码），于是 winit 拿不到
    # 键位、事件被丢弃 —— cap_safe 的 -Keys 从来没有生效过，而同期 pm_play /
    # gameplay_smoke_pm.py 的 `lp = 1 | (scan << 16)` 一直是对的。别再退回不带扫描码的写法。
    $scan = [int64][W32S]::MapVirtualKey([uint32]$vk, 0)   # MAPVK_VK_TO_VSC
    $base = [int64](1 -bor ($scan -shl 16))
    $downL = [IntPtr]$base
    $upL = [IntPtr]([int64]($base -bor [int64]0xC0000000))  # bit31 前次状态 + bit30 转换
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
        # 按**窗口标题**找 HWND，不要用 Process.MainWindowHandle：
        # 2026-09-12 实测后者对 winit 程序拿到的不是接收输入的那个窗口 —— cap_safe 的
        # -Keys 因此从来没生效过（投了 VK 36/R 换弹，游戏零响应；而同一个按键走
        # pm_play / gameplay_smoke_pm 的 FindWindowW 路径就能确定性改变游戏状态）。
        # 必须轮询 + 先把 $h 归一化成 [IntPtr]::Zero：返回 null 时 `-eq Zero` 是假的，
        # 会一路把 null 传进 Screenshot 报"cannot convert null to IntPtr"。
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
