
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class QW3 {
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
}
"@
$proc = Get-Process | Where-Object { $_.ProcessName -eq 'qianwen' -and $_.MainWindowHandle -ne 0 } | Select-Object -First 1
$fgBefore = [QW3]::GetForegroundWindow()
Write-Output ('fg before: ' + $fgBefore + ' qianwen: ' + $proc.MainWindowHandle)
[QW3]::SetForegroundWindow($proc.MainWindowHandle) | Out-Null
Start-Sleep -Milliseconds 1200
$fgAfter = [QW3]::GetForegroundWindow()
Write-Output ('fg after: ' + $fgAfter + ' match=' + ($fgAfter -eq $proc.MainWindowHandle))
# 重试复制
[System.Windows.Forms.SendKeys]::SendWait('^a')
Start-Sleep -Milliseconds 800
[System.Windows.Forms.SendKeys]::SendWait('^c')
Start-Sleep -Milliseconds 800
$t = [System.Windows.Forms.Clipboard]::GetText()
Write-Output ('clipboard len: ' + $t.Length)
if ($t.Length -gt 0) { Write-Output ('tail: ' + $t.Substring([Math]::Max(0, $t.Length - 200))) }
