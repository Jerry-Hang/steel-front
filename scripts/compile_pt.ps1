# Compile the PT panorama shaders: assets/rt/pt_panorama.glsl -> .spv
# Usage: powershell -ExecutionPolicy Bypass -File scripts/compile_pt.ps1
# Note: since 2026-08-31 the PT shaders are always produced by glslang (no hand-built SPIR-V).
#
# ASCII ONLY. Windows PowerShell 5.1 reads a BOM-less script as ANSI, so a trailing Chinese
# character in a comment can be decoded as the lead byte of a double-byte pair whose trail byte
# is the line ending itself -- the next line is then glued onto the comment and silently
# commented out. That is exactly what used to happen to `$ErrorActionPreference = 'Stop'` on
# line 4 of this file (see docs/PROGRESS.md, 2026-09-26): the script ran WITHOUT Stop.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$glsl = Join-Path $root 'assets\rt\pt_panorama.glsl'
$spv  = Join-Path $root 'assets\rt\pt_panorama.spv'

$candidates = @()
if ($env:VULKAN_SDK) { $candidates += (Join-Path $env:VULKAN_SDK 'Bin\glslangValidator.exe') }
$candidates += Get-ChildItem 'C:\VulkanSDK\*\Bin\glslangValidator.exe' -ErrorAction SilentlyContinue | ForEach-Object { $_.FullName }
$g = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $g) { Write-Error 'glslangValidator.exe not found (install the Vulkan SDK or set VULKAN_SDK)'; exit 1 }

& $g -V --target-env vulkan1.3 -S comp -o $spv $glsl
if ($LASTEXITCODE -ne 0) { Write-Error 'glslang failed'; exit 1 }
Write-Host "OK  $spv"

$v = Get-ChildItem 'C:\VulkanSDK\*\Bin\spirv-val.exe' -ErrorAction SilentlyContinue | Select-Object -First 1
if ($v) {
    & $v.FullName --target-env vulkan1.3 $spv
    if ($LASTEXITCODE -ne 0) { Write-Error 'spirv-val rejected the module'; exit 1 }
    Write-Host 'OK  spirv-val (vulkan1.3) passed'
}
