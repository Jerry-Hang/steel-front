
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System;
using System.Runtime.InteropServices;
public struct RECT9 { public int Left, Top, Right, Bottom; }
public class QW8 {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT9 rect);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
}
"@
$proc = Get-Process | Where-Object { $_.ProcessName -eq 'qianwen' -and $_.MainWindowHandle -ne 0 } | Select-Object -First 1
[QW8]::SetForegroundWindow($proc.MainWindowHandle) | Out-Null
Start-Sleep -Milliseconds 800
$rect = New-Object RECT9
[QW8]::GetWindowRect($proc.MainWindowHandle, [ref]$rect) | Out-Null
$w = $rect.Right - $rect.Left; $h = $rect.Bottom - $rect.Top
# 点击对话区中部（右 60%，高 35%）——消息列表聚焦
$x = $rect.Left + [int]($w * 0.6)
$y = $rect.Top + [int]($h * 0.35)
[QW8]::SetCursorPos($x, $y) | Out-Null
Start-Sleep -Milliseconds 300
[QW8]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
[QW8]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 800
# 全选 + 复制
[System.Windows.Forms.SendKeys]::SendWait('^a')
Start-Sleep -Milliseconds 1000
[System.Windows.Forms.SendKeys]::SendWait('^c')
Start-Sleep -Milliseconds 1000
$t = [System.Windows.Forms.Clipboard]::GetText()
Write-Output ('clipboard len: ' + $t.Length)
if ($t.Length -gt 0) {
  Write-Output '=== content (first 400) ==='
  Write-Output $t.Substring(0, [Math]::Min(400, $t.Length))
} else {
  Write-Output 'EMPTY'
}
