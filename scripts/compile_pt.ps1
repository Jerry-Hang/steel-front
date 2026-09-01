# 编译 PT 全景着色器：assets/rt/pt_panorama.glsl -> .spv
# 用法: powershell -ExecutionPolicy Bypass -File scripts/compile_pt.ps1
# 说明: 2026-08-31 起 PT 着色器一律由 glslang 正常编译产出（不再手工拼装 SPIR-V）。
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$glsl = Join-Path $root 'assets\rt\pt_panorama.glsl'
$spv  = Join-Path $root 'assets\rt\pt_panorama.spv'

$candidates = @()
if ($env:VULKAN_SDK) { $candidates += (Join-Path $env:VULKAN_SDK 'Bin\glslangValidator.exe') }
$candidates += Get-ChildItem 'C:\VulkanSDK\*\Bin\glslangValidator.exe' -ErrorAction SilentlyContinue | ForEach-Object { $_.FullName }
$g = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $g) { Write-Error '找不到 glslangValidator.exe（请安装 Vulkan SDK 或设 VULKAN_SDK）'; exit 1 }

& $g -V --target-env vulkan1.3 -S comp -o $spv $glsl
if ($LASTEXITCODE -ne 0) { Write-Error 'glslang 编译失败'; exit 1 }
Write-Host "OK  $spv"

$v = Get-ChildItem 'C:\VulkanSDK\*\Bin\spirv-val.exe' -ErrorAction SilentlyContinue | Select-Object -First 1
if ($v) {
    & $v.FullName --target-env vulkan1.3 $spv
    if ($LASTEXITCODE -ne 0) { Write-Error 'spirv-val 校验失败'; exit 1 }
    Write-Host 'OK  spirv-val (vulkan1.3) 通过'
}
