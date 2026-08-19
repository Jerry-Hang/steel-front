
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public struct RECTC { public int Left, Top, Right, Bottom; }
public class CapEdge {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECTC rect);
}
"@
$proc = Get-Process -Id 27584
$rect = New-Object RECTC
[CapEdge]::GetWindowRect($proc.MainWindowHandle, [ref]$rect) | Out-Null
$w = $rect.Right - $rect.Left; $h = $rect.Bottom - $rect.Top
Write-Output ('edge window: ' + $w + 'x' + $h + ' at ' + $rect.Left + ',' + $rect.Top)
if ($w -gt 200 -and $h -gt 200) {
  $bmp = New-Object System.Drawing.Bitmap($w, $h)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($rect.Left, $rect.Top, 0, 0, (New-Object System.Drawing.Size($w, $h)))
  $bmp.Save('D:\Rust\steel-front\scripts\dsh_cap.png')
  Write-Output ('saved: ' + (Get-Item 'D:\Rust\steel-front\scripts\dsh_cap.png').Length)
} else { Write-Output 'window too small' }
