$ErrorActionPreference = 'Stop'
$dir = 'D:\Rust\steel-front\src\engine\guns'
$names = @('antimaterial','assault_blue','assault_red','dmr','hmg','lmg_blue','lmg_red','pistols','shotgun','smg_blue','smg_red','sniper')
$enc = New-Object System.Text.UTF8Encoding $false
$bt = [string][char]96
foreach ($n in $names) {
  $f = Join-Path $dir ($n + '.rs')
  $c = [System.IO.File]::ReadAllText($f, [System.Text.Encoding]::UTF8)
  $pat = '(?s)' + $bt + $bt + $bt + '[a-zA-Z]*\s*(.*?)' + $bt + $bt + $bt
  $m = [regex]::Match($c, $pat)
  $code = if ($m.Success) { $m.Groups[1].Value } else { $c }
  $code = $code.Trim()
  if ($code -match 'let rzm = ') {
    $code = [regex]::Replace($code, 'rz\((-?[0-9])', 'rzm($1')
  }
  [System.IO.File]::WriteAllText($f, $code + [Environment]::NewLine, $enc)
  $left = [regex]::Matches($code, 'rz\((-?[0-9])').Count
  Write-Output ($n + ': ' + $code.Length + ' bytes, closure calls left: ' + $left)
}
