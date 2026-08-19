
# 钢铁前线 → 千问桥（半自动）：自动激活千问 → 点击输入框 → 粘贴截图 → 输入英文问题 → 发送
# 用法: .ask_qianwen.ps1 -Image <png路径> -Question "英文问题" [-WaitSec 10]
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
if (-not $proc) { Write-Output 'ERR: qianwen 进程未找到'; exit 1 }
[QWB]::SetForegroundWindow($proc.MainWindowHandle) | Out-Null
Start-Sleep -Milliseconds 800
$rect = New-Object RECTB
[QWB]::GetWindowRect($proc.MainWindowHandle, [ref]$rect) | Out-Null
$w = $rect.Right - $rect.Left; $h = $rect.Bottom - $rect.Top
# 输入框：窗口左 25%、底部 -45（千问客户端实测位置）
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
  Write-Output 'OK: 图片已粘贴'
}
[System.Windows.Forms.SendKeys]::SendWait($Question)
Start-Sleep -Milliseconds 500
[System.Windows.Forms.SendKeys]::SendWait('{ENTER}')
Write-Output ('OK: 已发送（等 ' + $WaitSec + ' 秒回答）')
Start-Sleep -Seconds $WaitSec
Write-Output 'OK: 请查看千问窗口中的回答'
