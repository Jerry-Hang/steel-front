$ErrorActionPreference = "Continue"
Set-Location "D:\Rust\steel-front"
$EXE = "D:\Rust\steel-front\target\release\steel-front.exe"
foreach ($w in 35, 14, 32) {
    Get-Process -Name steel-front -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Milliseconds 800
    $log = "D:\Rust\steel-front\switch_smoke_$w.log"
    Remove-Item -Force $log, "$log.err" -ErrorAction SilentlyContinue
    $env:RV3D_SWITCH_WEAPON = "$w"
    $env:RV3D_AUTOSTART = "1"
    Start-Process -FilePath $EXE -WorkingDirectory "D:\Rust\steel-front" -RedirectStandardOutput $log -RedirectStandardError "$log.err" | Out-Null
    Start-Sleep -Seconds 9
    Get-Process -Name steel-front -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Milliseconds 500
    Remove-Item Env:\RV3D_SWITCH_WEAPON -ErrorAction SilentlyContinue
    Remove-Item Env:\RV3D_AUTOSTART -ErrorAction SilentlyContinue
    $err = Get-Content "$log.err" -Raw -Encoding UTF8 -ErrorAction SilentlyContinue
    $panic = $err -match "panicked"
    $vuid = $err -match "VUID"
    $devlost = $err -match "device lost|DEVICE_LOST"
    $switch = $err -match [char]0x5207 + [char]0x67AA
    $fps = [regex]::Matches($err, "fps=([\d\.]+)") | ForEach-Object { $_.Groups[1].Value } | Select-Object -Last 1
    Write-Host ("W={0} panic={1} vuid={2} devlost={3} switchlog={4} fps={5}" -f $w, $panic, $vuid, $devlost, $switch, $fps)
    if ($panic -or $vuid -or $devlost) { Get-Content "$log.err" -Tail 15 -Encoding UTF8 }
}
