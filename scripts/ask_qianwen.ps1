
# Steel Front -> Qwen bridge (semi-automatic): focus Qwen, click the input box, paste the
# screenshot, type an English question, send it.
# Usage: .ask_qianwen.ps1 -Image <png path> -Question "english question" [-WaitSec 10]
#
# ASCII ONLY (2026-09-26): with Chinese comments this file was read as ANSI by Windows
# PowerShell 5.1, and a trailing multi-byte character ate the following line -- here
# `$ix = $rect.Left + [int]($w * 0.25)` was silently commented out, so the click landed at
# x=0. See docs/PROGRESS.md 2026-09-26.
param(
  [string]$Image = '',
  [string]$Question = 'Describe this screenshot briefly.',
  [int]$WaitSec = 10
)
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public struct RECTB { public int Left, Top, Right, Bottom; }
public class QWB {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECTB rect);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
}
"@
$proc = Get-Process | Where-Object { $_.ProcessName -eq 'qianwen' -and $_.MainWindowHandle -ne 0 } | Select-Object -First 1
if (-not $proc) { Write-Output 'ERR: qianwen process not found'; exit 1 }
[QWB]::SetForegroundWindow($proc.MainWindowHandle) | Out-Null
Start-Sleep -Milliseconds 800
$rect = New-Object RECTB
[QWB]::GetWindowRect($proc.MainWindowHandle, [ref]$rect) | Out-Null
$w = $rect.Right - $rect.Left; $h = $rect.Bottom - $rect.Top
# Input box: 25% from the window's left edge, 45 px above its bottom (measured on the client).
$ix = $rect.Left + [int]($w * 0.25)
$iy = $rect.Top + $h - 45
[QWB]::SetCursorPos($ix, $iy) | Out-Null
Start-Sleep -Milliseconds 300
[QWB]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
[QWB]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 800
if ($Image -ne '') {
  $img = [System.Drawing.Image]::FromFile($Image)
  [System.Windows.Forms.Clipboard]::SetImage($img)
  Start-Sleep -Milliseconds 300
  [System.Windows.Forms.SendKeys]::SendWait('^v')
  Start-Sleep -Milliseconds 2500
  Write-Output 'OK: image pasted'
}
[System.Windows.Forms.SendKeys]::SendWait($Question)
Start-Sleep -Milliseconds 500
[System.Windows.Forms.SendKeys]::SendWait('{ENTER}')
Write-Output ('OK: sent (waiting ' + $WaitSec + ' s for the answer)')
Start-Sleep -Seconds $WaitSec
Write-Output 'OK: read the answer in the Qwen window'
