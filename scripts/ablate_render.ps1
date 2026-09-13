# Render ablation: find where the GPU frame time actually goes (ASCII only).
#
# WHY: the CPU waits ~4ms per frame on the GPU fence (wait=3992us of a 4647us frame),
# and the GPU draws only ~60W of an available 111.92W. So the GPU is the bottleneck but
# is not power-limited -- something in the frame is expensive without being power-dense.
# The perf log has no GPU sub-item breakdown, so measure by ABLATION: switch one thing
# off per run and compare fps.
#
# fps comes from the `cam:` line: fps = 1e6 / cyc. MEDIAN of the samples, because the
# first frames and scene transitions are outliers.
#
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File scripts\ablate_render.ps1

param([int]$WarmupSec = 14)

$ErrorActionPreference = "Continue"
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo
$exe = Join-Path $repo "target\release\steel-front.exe"

# name + environment overrides (semicolon separated KEY=VALUE)
$cases = @(
    @{ Name = "baseline";            Env = "" },
    @{ Name = "no props";            Env = "RV3D_NO_PROPS=1" },
    @{ Name = "no shadow";           Env = "RV3D_NO_SHADOW=1" },
    @{ Name = "no proc-tex";         Env = "RV3D_PROC_TEX=0" },
    @{ Name = "no props+shadow";     Env = "RV3D_NO_PROPS=1;RV3D_NO_SHADOW=1" }
)

Write-Host "==== render ablation (RV3D_STRESS_AI=0, fixed camera) ===="
Write-Host ("{0,-22} {1,10} {2,10} {3,8}" -f "case", "med fps", "min fps", "samples")

$results = @()
foreach ($c in $cases) {
    $tag = "abl_" + ($c.Name -replace '[^0-9A-Za-z]', '_')
    $log = Join-Path $repo "logs\$tag.log.err"
    $out = Join-Path $repo "logs\$tag.log.out"
    if (Test-Path $log) { Remove-Item $log -Force }
    if (Test-Path $out) { Remove-Item $out -Force }

    # A fixed camera makes every run frame the SAME scene, otherwise the comparison is
    # between different views (AGENTS.md lesson 28).
    $env:RV3D_STRESS_AI = "0"
    $env:RV3D_CAM = "0,3,0:0,5"
    if ($c.Env -ne "") {
        foreach ($kv in $c.Env.Split(";")) {
            $parts = $kv.Split("=")
            Set-Item -Path ("Env:" + $parts[0]) -Value $parts[1]
        }
    }

    $p = Start-Process -FilePath $exe -WorkingDirectory $repo -PassThru `
        -RedirectStandardError $log -RedirectStandardOutput $out
    Start-Sleep -Seconds $WarmupSec
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 2

    # clear env
    $env:RV3D_CAM = ""
    Remove-Item Env:RV3D_STRESS_AI, Env:RV3D_CAM, Env:RV3D_NO_PROPS, Env:RV3D_NO_SHADOW, Env:RV3D_PROC_TEX, Env:RV3D_RES -ErrorAction SilentlyContinue

    $fps = @()
    if (Test-Path $log) {
        foreach ($line in [System.IO.File]::ReadAllLines($log, [System.Text.Encoding]::UTF8)) {
            if ($line -match "cyc=(\d+)") {
                $v = [double]$Matches[1]
                if ($v -gt 0) { $fps += (1000000.0 / $v) }
            }
        }
    }
    if ($fps.Count -ge 3) {
        $s = $fps | Sort-Object
        $med = $s[[int]($s.Count / 2)]
        Write-Host ("{0,-22} {1,10:N1} {2,10:N1} {3,8}" -f $c.Name, $med, $s[0], $fps.Count)
        $results += [pscustomobject]@{ Name = $c.Name; Median = $med }
    } else {
        Write-Host ("{0,-22} {1,10} {2,10} {3,8}" -f $c.Name, "-", "-", $fps.Count)
    }
}

Write-Host ""
Write-Host "==== delta vs baseline ===="
if ($results.Count -ge 2) {
    $base = $results[0].Median
    foreach ($r in $results) {
        $d = 0.0
        if ($base -gt 0) { $d = ($r.Median - $base) / $base * 100.0 }
        Write-Host ("  {0,-22} {1,7:N1}%" -f $r.Name, $d)
    }
}
