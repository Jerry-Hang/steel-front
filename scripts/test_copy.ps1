
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class QW2 {
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
}
"@
$proc = Get-Process | Where-Object { $_.ProcessName -eq 'qianwen' -and $_.MainWindowHandle -ne 0 } | Select-Object -First 1
[QW2]::SetForegroundWindow($proc.MainWindowHandle) | Out-Null
Start-Sleep -Milliseconds 1000
[System.Windows.Forms.SendKeys]::SendWait('^a')
Start-Sleep -Milliseconds 800
[System.Windows.Forms.SendKeys]::SendWait('^c')
Start-Sleep -Milliseconds 800
$t = [System.Windows.Forms.Clipboard]::GetText()
Write-Output ('clipboard len: ' + $t.Length)
if ($t.Length -gt 0) {
  Write-Output ('head: ' + $t.Substring(0, [Math]::Min(300, $t.Length)))
  Write-Output ('tail: ' + $t.Substring([Math]::Max(0, $t.Length - 300)))
} else {
  Write-Output 'EMPTY - 复制失败（可能窗口未激活或无可选文本）'
}
