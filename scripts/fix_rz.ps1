$ErrorActionPreference = 'Stop'
$dir = 'D:\Rust\steel-front\src\engine\guns'
$names = @('antimaterial','assault_blue','assault_red','dmr','hmg','lmg_blue','lmg_red','pistols','shotgun','smg_blue','smg_red','sniper')
$enc = New-Object System.Text.UTF8Encoding $false
foreach ($n in $names) {
  $f = Join-Path $dir ($n + '.rs')
  $lines = [System.IO.File]::ReadAllLines($f, [System.Text.Encoding]::UTF8)
  $changed = 0
  for ($i = 0; $i -lt $lines.Count; $i++) {
    $line = $lines[$i]
    if ($line -match '^use ') { continue }
    $newLine = [regex]::Replace($line, '\brz\b(?!\()', 'rz()')
    if ($newLine -ne $line) { $lines[$i] = $newLine; $changed++ }
  }
  [System.IO.File]::WriteAllLines($f, $lines, $enc)
  Write-Output ($n + ': fixed lines ' + $changed)
}
