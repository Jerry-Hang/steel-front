
param([string]$Question = 'Say hello in one word.')
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public struct RECT5 { public int Left, Top, Right, Bottom; }
public class QW4 {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT5 rect);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
}
"@
function Cap-QW($path) {
  $proc = Get-Process | Where-Object { $_.ProcessName -eq 'qianwen' -and $_.MainWindowHandle -ne 0 } | Select-Object -First 1
  $rect = New-Object RECT5
  [QW4]::GetWindowRect($proc.MainWindowHandle, [ref]$rect) | Out-Null
  $w = $rect.Right - $rect.Left; $h = $rect.Bottom - $rect.Top
  $bmp = New-Object System.Drawing.Bitmap($w, $h)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($rect.Left, $rect.Top, 0, 0, (New-Object System.Drawing.Size($w, $h)))
  $bmp.Save($path)
  return $rect
}
$proc = Get-Process | Where-Object { $_.ProcessName -eq 'qianwen' -and $_.MainWindowHandle -ne 0 } | Select-Object -First 1
[QW4]::SetForegroundWindow($proc.MainWindowHandle) | Out-Null
Start-Sleep -Milliseconds 800
# 点击输入框（底部中央）并输入问题（纯文字，无图——先验证文字发送）
$rect = Cap-QW 'D:\Rust\steel-front\scripts\qw_before.png'
$cx = $rect.Left + [int](($rect.Right - $rect.Left) / 2)
$cy = $rect.Bottom - 45
[QW4]::SetCursorPos($cx, $cy) | Out-Null
Start-Sleep -Milliseconds 300
[QW4]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
[QW4]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 800
[System.Windows.Forms.SendKeys]::SendWait($Question)
Start-Sleep -Milliseconds 400
[System.Windows.Forms.SendKeys]::SendWait('{ENTER}')
Write-Output 'sent'
Start-Sleep -Seconds 4
# 截发送后窗口
$rect2 = Cap-QW 'D:\Rust\steel-front\scripts\qw_after.png'
Write-Output ('captured before/after: ' + $rect.Left + ',' + $rect.Top + ' ' + ($rect2.Right - $rect2.Left) + 'x' + ($rect2.Bottom - $rect2.Top))
