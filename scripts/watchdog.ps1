# 采集看门狗：若 steel-front 退出则 15 秒后重启（直到 22:10）
$until = (Get-Date).Date.AddHours(22).AddMinutes(10)
while ((Get-Date) -lt $until) {
    $p = Get-Process -Name steel-front -ErrorAction SilentlyContinue
    if (-not $p) {
        Write-Host ("[{0}] game down -> restart" -f (Get-Date -Format 'HH:mm:ss'))
        Start-Sleep -Seconds 15
        $env:RV3D_AUTOSTART='1'
        Remove-Item Env:RV3D_STRESS_AI -ErrorAction SilentlyContinue
        $env:RV3D_LLM='1'
        $env:RV3D_LLM_INTERVAL='150'
        Start-Process -FilePath 'D:\Rust\steel-front\target\release\steel-front.exe' -WorkingDirectory 'D:\Rust\steel-front' -RedirectStandardError 'D:\Rust\steel-front\data\battle_log.txt'
    }
    Start-Sleep -Seconds 10
}
Write-Host 'watchdog done (22:10)'
