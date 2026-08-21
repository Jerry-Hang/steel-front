$ErrorActionPreference = "Stop"
$log = 'D:\Rust\steel-front\vision_result.txt'
$key = 'sk-54811b5099304920963578b4755d884b'
$b64 = [Convert]::ToBase64String([IO.File]::ReadAllBytes('D:\Rust\steel-front\screenshots\steel_front_1787309259.png'))
$body = @{
    model = 'deepseek-v4-flash-vision-exp'
    messages = @(@{ role = 'user'; content = @(
        @{ type = 'text'; text = 'Describe this game screenshot: HUD elements, is the Chinese text at bottom-left (weapon name) clear? garbled? distorted?' },
        @{ type = 'image_url'; image_url = @{ url = ('data:image/png;base64,' + $b64) } }
    ) })
    max_tokens = 800
} | ConvertTo-Json -Depth 10
try {
    $r = Invoke-RestMethod -Uri 'https://api.deepseek.com/chat/completions' -Method Post -Headers @{ Authorization = ('Bearer ' + $key) } -ContentType 'application/json' -Body $body -TimeoutSec 150
    Set-Content -Path $log -Encoding UTF8 -Value ('OK' + [Environment]::NewLine + $r.choices[0].message.content)
} catch {
    Set-Content -Path $log -Encoding UTF8 -Value ('FAIL ' + $_.Exception.Message)
    if ($_.ErrorDetails.Message) { Add-Content -Path $log -Encoding UTF8 -Value $_.ErrorDetails.Message }
}