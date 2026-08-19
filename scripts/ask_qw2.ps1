
param([string]$Image, [string]$Question, [int]$WaitSec = 15)
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public struct RECT7 { public int Left, Top, Right, Bottom; }
public class QW6 {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT7 rect);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
}
"@
function Cap-QW($path) {
  $proc = Get-Process | Where-Object { $_.ProcessName -eq 'qianwen' -and $_.MainWindowHandle -ne 0 } | Select-Object -First 1
  $rect = New-Object RECT7
  [QW6]::GetWindowRect($proc.MainWindowHandle, [ref]$rect) | Out-Null
  $w = $rect.Right - $rect.Left; $h = $rect.Bottom - $rect.Top
  $bmp = New-Object System.Drawing.Bitmap($w, $h)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($rect.Left, $rect.Top, 0, 0, (New-Object System.Drawing.Size($w, $h)))
  $bmp.Save($path)
  return @{ L = $rect.Left; T = $rect.Top; W = $w; H = $h }
}
$proc = Get-Process | Where-Object { $_.ProcessName -eq 'qianwen' -and $_.MainWindowHandle -ne 0 } | Select-Object -First 1
[QW6]::SetForegroundWindow($proc.MainWindowHandle) | Out-Null
Start-Sleep -Milliseconds 800
$r = Cap-QW 'D:\Rust\steel-front\scripts\qw_before2.png'
# 输入框：窗口左 25%、底部 -45
$ix = $r.L + [int]($r.W * 0.25)
$iy = $r.T + $r.H - 45
[QW6]::SetCursorPos($ix, $iy) | Out-Null
Start-Sleep -Milliseconds 300
[QW6]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
[QW6]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 600
# 清空输入框残留
[System.Windows.Forms.SendKeys]::SendWait('^a')
Start-Sleep -Milliseconds 300
[System.Windows.Forms.SendKeys]::SendWait('{DELETE}')
Start-Sleep -Milliseconds 400
if ($Image -ne '') {
  $img = [System.Drawing.Image]::FromFile($Image)
  [System.Windows.Forms.Clipboard]::SetImage($img)
  Start-Sleep -Milliseconds 300
  [System.Windows.Forms.SendKeys]::SendWait('^v')
  Start-Sleep -Milliseconds 2000
  Write-Output 'image pasted'
}
[System.Windows.Forms.SendKeys]::SendWait($Question)
Start-Sleep -Milliseconds 500
[System.Windows.Forms.SendKeys]::SendWait('{ENTER}')
Write-Output 'sent'
Start-Sleep -Seconds 3
$r2 = Cap-QW 'D:\Rust\steel-front\scripts\qw_after2.png'
Write-Output ('captured, waiting ' + $WaitSec + 's for answer...')
Start-Sleep -Seconds $WaitSec
