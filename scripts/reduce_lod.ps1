$ErrorActionPreference = 'Stop'
$dir = 'D:\Rust\steel-front\src\engine\guns'
$names = @('antimaterial','assault_blue','assault_red','dmr','hmg','lmg_blue','lmg_red','pistols','shotgun','smg_blue','smg_red','sniper')
$enc = New-Object System.Text.UTF8Encoding $false
foreach ($n in $names) {
  $f = Join-Path $dir ($n + '.rs')
  $c = [System.IO.File]::ReadAllText($f, [System.Text.Encoding]::UTF8)
  $orig = $c
  # cylinder(r, h, seg)  seg>16 -> 12
  $c = [regex]::Replace($c, 'cylinder\(([0-9.]+),\s*([0-9.]+),\s*([0-9]+)\)', {
    param($m)
    $seg = [int]$m.Groups[3].Value
    if ($seg -gt 16) { return ('cylinder(' + $m.Groups[1].Value + ', ' + $m.Groups[2].Value + ', 12)') }
    return $m.Value
  })
  # frustum(r0, r1, h, seg, caps)  seg>16 -> 12
  $c = [regex]::Replace($c, 'frustum\(([0-9.]+),\s*([0-9.]+),\s*([0-9.]+),\s*([0-9]+)', {
    param($m)
    $seg = [int]$m.Groups[4].Value
    if ($seg -gt 16) { return ('frustum(' + $m.Groups[1].Value + ', ' + $m.Groups[2].Value + ', ' + $m.Groups[3].Value + ', 12') }
    return $m.Value
  })
  # sphere(seg, rings)  seg>14 -> 10, rings>8 -> 6
  $c = [regex]::Replace($c, 'sphere\(([0-9]+),\s*([0-9]+)\)', {
    param($m)
    $seg = [int]$m.Groups[1].Value
    $rings = [int]$m.Groups[2].Value
    $ns = if ($seg -gt 14) { 10 } else { $seg }
    $nr = if ($rings -gt 8) { 6 } else { $rings }
    return ('sphere(' + $ns + ', ' + $nr + ')')
  })
  # torus_arc(ring_r, tube_r, t0, t1, seg_ring, seg_tube)  ring>12 -> 8, tube>6 -> 6
  $c = [regex]::Replace($c, 'torus_arc\(([^)]+),\s*([0-9]+),\s*([0-9]+)\)', {
    param($m)
    $sr = [int]$m.Groups[2].Value
    $st = [int]$m.Groups[3].Value
    $nsr = if ($sr -gt 12) { 8 } else { $sr }
    $nst = if ($st -gt 6) { 6 } else { $st }
    return ('torus_arc(' + $m.Groups[1].Value + ', ' + $nsr + ', ' + $nst + ')')
  })
  # beveled_box(w,h,d,r,seg)  seg>4 -> 3
  $c = [regex]::Replace($c, 'beveled_box\(([^)]+),\s*([0-9]+)\)', {
    param($m)
    $seg = [int]$m.Groups[2].Value
    if ($seg -gt 4) { return ('beveled_box(' + $m.Groups[1].Value + ', 3)') }
    return $m.Value
  })
  if ($c -ne $orig) {
    [System.IO.File]::WriteAllText($f, $c, $enc)
    Write-Output ($n + ': reduced')
  } else {
    Write-Output ($n + ': unchanged')
  }
}
