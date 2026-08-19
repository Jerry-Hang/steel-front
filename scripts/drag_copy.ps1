
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System;
using System.Runtime.InteropServices;
public struct RECTA { public int Left, Top, Right, Bottom; }
public class QW9 {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECTA rect);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
}
"@
# 清剪贴板
[System.Windows.Forms.Clipboard]::Clear()
$proc = Get-Process | Where-Object { $_.ProcessName -eq 'qianwen' -and $_.MainWindowHandle -ne 0 } | Select-Object -First 1
[QW9]::SetForegroundWindow($proc.MainWindowHandle) | Out-Null
Start-Sleep -Milliseconds 800
$rect = New-Object RECTA
[QW9]::GetWindowRect($proc.MainWindowHandle, [ref]$rect) | Out-Null
$w = $rect.Right - $rect.Left; $h = $rect.Bottom - $rect.Top
# 拖选：对话区从 (0.45W, 0.25H) 到 (0.85W, 0.6H)——覆盖回答文本
$x1 = $rect.Left + [int]($w * 0.45); $y1 = $rect.Top + [int]($h * 0.25)
$x2 = $rect.Left + [int]($w * 0.85); $y2 = $rect.Top + [int]($h * 0.60)
[QW9]::SetCursorPos($x1, $y1) | Out-Null
Start-Sleep -Milliseconds 300
[QW9]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)  # LEFTDOWN
Start-Sleep -Milliseconds 200
for ($i = 1; $i -le 20; $i++) {
  $px = $x1 + [int](($x2 - $x1) * $i / 20)
  $py = $y1 + [int](($y2 - $y1) * $i / 20)
  [QW9]::SetCursorPos($px, $py) | Out-Null
  Start-Sleep -Milliseconds 40
}
[QW9]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)  # LEFTUP
Start-Sleep -Milliseconds 600
[System.Windows.Forms.SendKeys]::SendWait('^c')
Start-Sleep -Milliseconds 800
$t = [System.Windows.Forms.Clipboard]::GetText()
Write-Output ('clipboard len: ' + $t.Length)
if ($t.Length -gt 0) {
  Write-Output '=== tail 600 ==='
  Write-Output $t.Substring([Math]::Max(0, $t.Length - 600))
}
