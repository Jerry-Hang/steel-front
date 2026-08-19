
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System;
using System.Runtime.InteropServices;
public struct RECTF { public int Left, Top, Right, Bottom; }
public class DSHF {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECTF rect);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
}
"@
$proc = Get-Process -Id 27584
[DSHF]::SetForegroundWindow($proc.MainWindowHandle) | Out-Null
Start-Sleep -Milliseconds 800
$rect = New-Object RECTF
[DSHF]::GetWindowRect($proc.MainWindowHandle, [ref]$rect) | Out-Null
$w = $rect.Right - $rect.Left
$h = $rect.Bottom - $rect.Top
Write-Output ('size: ' + $w + 'x' + $h)
$x = $rect.Left + [int]($w * 0.5)
$y = $rect.Top + $h - 45
Write-Output ('click: ' + $x + ',' + $y)
[DSHF]::SetCursorPos($x, $y) | Out-Null
Start-Sleep -Milliseconds 400
[DSHF]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
[DSHF]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 800
[System.Windows.Forms.SendKeys]::SendWait('__scheduler_test__')
Start-Sleep -Milliseconds 800
[System.Windows.Forms.SendKeys]::SendWait('~')
Write-Output 'sent'
