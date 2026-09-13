# Resolution scaling test: vertex-bound or fill-rate-bound? (ASCII only)
#
# WHY: ablation showed props cost 37.6% of the frame. That alone does not say WHICH
# resource is short. If halving the resolution nearly doubles fps, the frame is
# FRAGMENT/fill-rate bound (fix = less overdraw / fewer pixels). If fps barely moves,
# it is VERTEX/geometry/submit bound (fix = fewer vertices or fewer draws).
#
# Resolution lives only in %USERPROFILE%\.steel_front.cfg (no env override), so this
# script backs the file up, swaps in each resolution, runs, and ALWAYS restores the
# original -- including on error.
#
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File scripts\test_resolution.ps1

param([int]$WarmupSec = 14)

$ErrorActionPreference = "Continue"
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo
$exe = Join-Path $repo "target\release\steel-front.exe"
$cfg = Join-Path $env:USERPROFILE ".steel_front.cfg"
$bak = "$cfg.res_test_backup"

if (-not (Test-Path $cfg)) { Write-Host "config not found: $cfg"; exit 1 }
Copy-Item $cfg $bak -Force
Write-Host ("backed up config -> " + $bak)

# resolutions to try (all must exist in ui.rs RESOLUTIONS or the loader rejects them)
$res = @("2560x1600", "1920x1080", "1280x800")

$results = @()
try {
    Write-Host "==== resolution scaling (fixed camera, RV3D_STRESS_AI=0) ===="
    Write-Host ("{0,-14} {1,12} {2,10} {3,8}" -f "resolution", "megapixels", "med fps", "samples")

    foreach ($r in $res) {
        # rewrite only the resolution= line
        $lines = [System.IO.File]::ReadAllLines($cfg, [System.Text.Encoding]::UTF8)
        $outLines = @()
        foreach ($l in $lines) {
            if ($l -match "^resolution=") { $outLines += ("resolution=" + $r) }
            else { $outLines += $l }
        }
        [System.IO.File]::WriteAllLines($cfg, $outLines, (New-Object System.Text.UTF8Encoding $false))

        $tag = "res_" + ($r -replace "[^0-9]", "_")
        $log = Join-Path $repo "logs\$tag.log.err"
        $out = Join-Path $repo "logs\$tag.log.out"
        if (Test-Path $log) { Remove-Item $log -Force }
        if (Test-Path $out) { Remove-Item $out -Force }

        $env:RV3D_STRESS_AI = "0"
        $env:RV3D_CAM = "0,3,0:0,5"

        $p = Start-Process -FilePath $exe -WorkingDirectory $repo -PassThru `
            -RedirectStandardError $log -RedirectStandardOutput $out
        Start-Sleep -Seconds $WarmupSec
        Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
        Start-Sleep -Seconds 2

        Remove-Item Env:RV3D_STRESS_AI, Env:RV3D_CAM -ErrorAction SilentlyContinue

        $fps = @()
        if (Test-Path $log) {
            foreach ($line in [System.IO.File]::ReadAllLines($log, [System.Text.Encoding]::UTF8)) {
                if ($line -match "cyc=(\d+)") {
                    $v = [double]$Matches[1]
                    if ($v -gt 0) { $fps += (1000000.0 / $v) }
                }
            }
        }
        $wh = $r.Split("x")
        $mp = ([double]$wh[0] * [double]$wh[1]) / 1000000.0
        if ($fps.Count -ge 3) {
            $s = $fps | Sort-Object
            $med = $s[[int]($s.Count / 2)]
            Write-Host ("{0,-14} {1,12:N2} {2,10:N1} {3,8}" -f $r, $mp, $med, $fps.Count)
            $results += [pscustomobject]@{ Res = $r; MP = $mp; Median = $med }
        } else {
            Write-Host ("{0,-14} {1,12:N2} {2,10} {3,8}" -f $r, $mp, "-", $fps.Count)
        }
    }

    Write-Host ""
    Write-Host "==== verdict ===="
    if ($results.Count -ge 2) {
        $hi = $results[0]
        $lo = $results[$results.Count - 1]
        $ratio = $hi.MP / $lo.MP
        $gain = ($lo.Median - $hi.Median) / $hi.Median * 100.0
        Write-Host ("  pixels {0:N2}x fewer -> fps +{1:N1}%" -f $ratio, $gain)
        if ($gain -gt 40.0) {
            Write-Host "  => FILL-RATE bound: the fix is fewer pixels/overdraw, not fewer vertices."
        } elseif ($gain -lt 12.0) {
            Write-Host "  => VERTEX/SUBMIT bound: the fix is fewer vertices or fewer draw calls."
        } else {
            Write-Host "  => mixed: both matter."
        }
    }
} finally {
    Copy-Item $bak $cfg -Force
    Remove-Item $bak -Force -ErrorAction SilentlyContinue
    Write-Host "config restored from backup"
}
