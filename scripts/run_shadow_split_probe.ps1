# run_shadow_split_probe.ps1 - validate the two-shadow-map split (static + dynamic).
#
# What is being validated (2026-09-26)
# -----------------------------------
# renderer.rs now keeps TWO shadow maps: a static one (terrain / ground field / markers /
# props) that is redrawn only every RV3D_SHADOW_STATIC_EVERY frames, and a dynamic one (NPC
# boxes/cylinders/spheres + soldier GLB) redrawn on the RV3D_SHADOW_EVERY cadence. The
# fragment shader samples both and takes the max occlusion (build.rs binding 10).
#
# The A/B control arm is -Extra RV3D_NO_SHADOW_SPLIT=1: one map, both groups, every frame
# count unchanged -- i.e. exactly the code path this change replaced.
#
# Usage:
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\run_shadow_split_probe.ps1
#   ... -Rounds 3 -Secs 20 -Screens -Validate
#
# Exit code 0 when every measurement produced a number; 1 otherwise.
param(
    [int]$Rounds = 3,
    [int]$Secs = 20,
    [int]$Stress = 128,
    [switch]$Screens,
    [switch]$Validate
)

$ErrorActionPreference = "Continue"
$repo = "D:\Rust\steel-front"
$out = Join-Path $repo "logs\shadow_split_probe.txt"

function Clear-Extra([string[]]$keys) {
    foreach ($k in $keys) { Remove-Item ("Env:" + $k) -ErrorAction SilentlyContinue }
}

"=== shadow split probe $(Get-Date -Format 'HH:mm:ss') (secs=$Secs stress=$Stress rounds=$Rounds) ===" |
    Set-Content -Encoding utf8 $out

# ---- 1. A/B: split (default) vs single map (control) -------------------------------------
$splitMeans = @()
$singleMeans = @()
for ($i = 1; $i -le $Rounds; $i++) {
    $a = & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $repo "scripts\perf_run.ps1") -Secs $Secs -Stress $Stress 2>&1
    $la = ($a | Select-String -Pattern '^\s+fps\s').Line
    $dtA = ($a | Select-String -Pattern '^\s+dt_us\s').Line
    "split   R$i | $($la.Trim()) | $($dtA.Trim())" | Add-Content -Encoding utf8 $out
    $mA = [regex]::Match([string]$la, 'mean\s+([\d.]+)')
    if ($mA.Success) { $splitMeans += [double]$mA.Groups[1].Value }

    $b = & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $repo "scripts\perf_run.ps1") -Secs $Secs -Stress $Stress -Extra "RV3D_NO_SHADOW_SPLIT=1" 2>&1
    $lb = ($b | Select-String -Pattern '^\s+fps\s').Line
    $dtB = ($b | Select-String -Pattern '^\s+dt_us\s').Line
    "single  R$i | $($lb.Trim()) | $($dtB.Trim())" | Add-Content -Encoding utf8 $out
    $mB = [regex]::Match([string]$lb, 'mean\s+([\d.]+)')
    if ($mB.Success) { $singleMeans += [double]$mB.Groups[1].Value }
}
Clear-Extra @("RV3D_NO_SHADOW_SPLIT")

function Describe($vals, $name) {
    if ($vals.Count -eq 0) { return "$name : no samples" }
    $min = ($vals | Measure-Object -Minimum).Minimum
    $max = ($vals | Measure-Object -Maximum).Maximum
    $avg = ($vals | Measure-Object -Average).Average
    $pct = if ($min -gt 0) { 100.0 * ($max - $min) / $min } else { 0.0 }
    return ("{0,-8} mean {1,7:N2}  min {2,7:N2}  max {3,7:N2}  spread {4:N1}%" -f $name, $avg, $min, $max, $pct)
}
$summary = @("", (Describe $splitMeans "split"), (Describe $singleMeans "single"))
if ($splitMeans.Count -gt 0 -and $singleMeans.Count -gt 0) {
    $d = 100.0 * (($splitMeans | Measure-Object -Average).Average - ($singleMeans | Measure-Object -Average).Average) / ($singleMeans | Measure-Object -Average).Average
    $summary += ("delta    {0:+0.0;-0.0}% (split vs single map)" -f $d)
}
$summary | ForEach-Object { $_ | Add-Content -Encoding utf8 $out; Write-Host $_ }

# ---- 2. Visual evidence: shadows exist, and the split looks like the single map -----------
if ($Screens) {
    $shots = @(
        @{ tag = "split_default"; env = @{} },
        @{ tag = "split_off";     env = @{ RV3D_NO_SHADOW_SPLIT = "1" } },
        @{ tag = "split_noshadow"; env = @{ RV3D_NO_SHADOW = "1" } }
    )
    foreach ($s in $shots) {
        foreach ($k in $s.env.Keys) { Set-Item -Path ("Env:" + $k) -Value $s.env[$k] }
        $env:RV3D_NPC_CAM = "1"
        & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $repo "scripts\cap_safe.ps1") `
            -Tag $s.tag -WarmupSec 12 -HoldSec 2 -Keys 123 -AfterKeysSec 3 2>&1 |
            Select-Object -Last 2 | ForEach-Object { "  $_" | Add-Content -Encoding utf8 $out }
        Clear-Extra @("RV3D_NO_SHADOW_SPLIT", "RV3D_NO_SHADOW", "RV3D_NPC_CAM")
    }
    "screens: split_default / split_off / split_noshadow -> screenshots\" | Add-Content -Encoding utf8 $out
}

# ---- 3. Whole-frame validation under the layer -------------------------------------------
if ($Validate) {
    $env:RV3D_VALIDATION = "1"
    $env:DISABLE_RTSS_LAYER = "1"
    $env:DISABLE_GAMEPP_LAYER = "1"
    $v = & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $repo "scripts\run_smoke_pm.ps1") 2>&1
    ($v | Select-Object -Last 4) | ForEach-Object { $_ | Add-Content -Encoding utf8 $out; Write-Host $_ }
    Clear-Extra @("RV3D_VALIDATION", "DISABLE_RTSS_LAYER", "DISABLE_GAMEPP_LAYER")
}

"=== done $(Get-Date -Format 'HH:mm:ss') ===" | Add-Content -Encoding utf8 $out
Write-Host ("full log: {0}" -f $out)
if ($splitMeans.Count -eq 0 -or $singleMeans.Count -eq 0) { exit 1 }
exit 0
