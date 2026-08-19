
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public struct RECT6 { public int Left, Top, Right, Bottom; }
public class QW5 {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT6 rect);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
}
"@
function Cap-QW($path) {
  $proc = Get-Process | Where-Object { $_.ProcessName -eq 'qianwen' -and $_.MainWindowHandle -ne 0 } | Select-Object -First 1
  $rect = New-Object RECT6
  [QW5]::GetWindowRect($proc.MainWindowHandle, [ref]$rect) | Out-Null
  $w = $rect.Right - $rect.Left; $h = $rect.Bottom - $rect.Top
  $bmp = New-Object System.Drawing.Bitmap($w, $h)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($rect.Left, $rect.Top, 0, 0, (New-Object System.Drawing.Size($w, $h)))
  $bmp.Save($path)
  return @{ L = $rect.Left; T = $rect.Top; W = $w; H = $h }
}
$proc = Get-Process | Where-Object { $_.ProcessName -eq 'qianwen' -and $_.MainWindowHandle -ne 0 } | Select-Object -First 1
[QW5]::SetForegroundWindow($proc.MainWindowHandle) | Out-Null
Start-Sleep -Milliseconds 600
$r = Cap-QW 'D:\Rust\steel-front\scripts\qw_base.png'
# 候选点（窗口相对坐标）：右 75%/50%/85% x 底部 45/80/120
$cands = @(
  @(0.75, -45), @(0.50, -45), @(0.85, -45), @(0.75, -80),
  @(0.50, -80), @(0.85, -80), @(0.75, -120), @(0.25, -45)
)
$idx = 0
foreach ($c in $cands) {
  $idx++
  $x = $r.L + [int]($r.W * $c[0])
  $y = $r.T + $r.H + $c[1]
  [QW5]::SetCursorPos($x, $y) | Out-Null
  Start-Sleep -Milliseconds 250
  [QW5]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
  [QW5]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
  Start-Sleep -Milliseconds 500
  [System.Windows.Forms.SendKeys]::SendWait('probe' + $idx)
  Start-Sleep -Milliseconds 300
  [System.Windows.Forms.SendKeys]::SendWait('{ENTER}')
  Start-Sleep -Milliseconds 2500
  $r2 = Cap-QW ('D:\Rust\steel-front\scripts\qw_probe_' + $idx + '.png')
  Write-Output ('probe ' + $idx + ' at (' + $x + ',' + $y + ')')
}
Write-Output 'done'
