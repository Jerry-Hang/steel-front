
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public struct RECTD { public int Left, Top, Right, Bottom; }
public class DSH {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECTD rect);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
}
"@
function Cap-Edge($path) {
  $proc = Get-Process -Id 27584
  $rect = New-Object RECTD
  [DSH]::GetWindowRect($proc.MainWindowHandle, [ref]$rect) | Out-Null
  $w = $rect.Right - $rect.Left; $h = $rect.Bottom - $rect.Top
  $bmp = New-Object System.Drawing.Bitmap($w, $h)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($rect.Left, $rect.Top, 0, 0, (New-Object System.Drawing.Size($w, $h)))
  $bmp.Save($path)
  return @{ L = $rect.Left; T = $rect.Top; W = $w; H = $h }
}
# 基准截图
$r = Cap-Edge 'D:\Rust\steel-front\scripts\dsh_before.png'
# 激活 Edge
$proc = Get-Process -Id 27584
[DSH]::SetForegroundWindow($proc.MainWindowHandle) | Out-Null
Start-Sleep -Milliseconds 800
# 点击输入框（窗口相对坐标：x=50% 宽, y=窗口高-45）
$x = $r.L + [int]($r.W * 0.5)
$y = $r.T + $r.H - 45
Write-Output ('click at: ' + $x + ',' + $y)
[DSH]::SetCursorPos($x, $y) | Out-Null
Start-Sleep -Milliseconds 300
[DSH]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
[DSH]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 800
# 输入测试文字（不 Enter）
[System.Windows.Forms.SendKeys]::SendWait('TEST_SCHED')
Start-Sleep -Milliseconds 800
$r2 = Cap-Edge 'D:\Rust\steel-front\scripts\dsh_after.png'
Write-Output 'typed'
