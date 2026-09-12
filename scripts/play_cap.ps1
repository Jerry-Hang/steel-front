# -*- coding: utf-8 -*-
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class WIN {
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint f);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, UIntPtr e);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr e);
  public struct RECT { public int L, T, R, B; }
}
"@
Add-Type -AssemblyName System.Drawing
$p = Get-Process steel-front -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $p) { Write-Host 'no game'; exit }
$h = $p.MainWindowHandle
[WIN]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 400
$r = New-Object WIN+RECT
[WIN]::GetWindowRect($h, [ref]$r) | Out-Null
$cx = [int](($r.L + $r.R) / 2); $cy = [int](($r.T + $r.B) / 2)
# 鼠标左键点击窗口中心（拿焦点！）+ 移开
[WIN]::SetCursorPos($cx, $cy) | Out-Null
Start-Sleep -Milliseconds 150
[WIN]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)   # LEFTDOWN
[WIN]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)   # LEFTUP
Start-Sleep -Milliseconds 700
# 死亡/菜单 UI：按 R（重开进入 Playing！）
[WIN]::keybd_event(0x52, 0, 0, [UIntPtr]::Zero)
[WIN]::keybd_event(0x52, 0, 2, [UIntPtr]::Zero)
Start-Sleep -Seconds 2
# PrintWindow 截图
$bmp = New-Object System.Drawing.Bitmap ($r.R - $r.L), ($r.B - $r.T)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $g.GetHdc()
[WIN]::PrintWindow($h, $hdc, 2) | Out-Null
$g.ReleaseHdc($hdc)
$bmp.Save('D:\Rust\steel-front\screenshots\play.png', [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
# 鼠标放回右上角安全位（还原！）
[WIN]::SetCursorPos(20, 20) | Out-Null
Write-Host 'done: focused + R restarted + captured play.png + mouse restored'
