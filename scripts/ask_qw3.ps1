
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public struct RECT8 { public int Left, Top, Right, Bottom; }
public class QW7 {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT8 rect);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
}
"@
function Cap-QW($path) {
  $proc = Get-Process | Where-Object { $_.ProcessName -eq 'qianwen' -and $_.MainWindowHandle -ne 0 } | Select-Object -First 1
  $rect = New-Object RECT8
  [QW7]::GetWindowRect($proc.MainWindowHandle, [ref]$rect) | Out-Null
  $w = $rect.Right - $rect.Left; $h = $rect.Bottom - $rect.Top
  $bmp = New-Object System.Drawing.Bitmap($w, $h)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($rect.Left, $rect.Top, 0, 0, (New-Object System.Drawing.Size($w, $h)))
  $bmp.Save($path)
  return @{ L = $rect.Left; T = $rect.Top; W = $w; H = $h }
}
$proc = Get-Process | Where-Object { $_.ProcessName -eq 'qianwen' -and $_.MainWindowHandle -ne 0 } | Select-Object -First 1
[QW7]::SetForegroundWindow($proc.MainWindowHandle) | Out-Null
Start-Sleep -Milliseconds 800
$r = Cap-QW 'D:\Rust\steel-front\scripts\qw_b3.png'
$ix = $r.L + [int]($r.W * 0.25)
$iy = $r.T + $r.H - 45
[QW7]::SetCursorPos($ix, $iy) | Out-Null
Start-Sleep -Milliseconds 300
[QW7]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
[QW7]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 800
# 粘贴图片（剪贴板已放图）
$img = [System.Drawing.Image]::FromFile('D:\Rust\steel-front\scripts\qianwen_cap.png')
[System.Windows.Forms.Clipboard]::SetImage($img)
Start-Sleep -Milliseconds 300
[System.Windows.Forms.SendKeys]::SendWait('^v')
Start-Sleep -Milliseconds 2500
[System.Windows.Forms.SendKeys]::SendWait('Describe this screenshot in one short sentence.')
Start-Sleep -Milliseconds 500
[System.Windows.Forms.SendKeys]::SendWait('{ENTER}')
Write-Output 'sent (simplified)'
Start-Sleep -Seconds 4
$r2 = Cap-QW 'D:\Rust\steel-front\scripts\qw_a3.png'
Write-Output 'captured after'
