# Steel Front release packaging
$ErrorActionPreference = "Stop"
Set-Location "D:\Rust\steel-front"

Write-Host "[1/3] building game..."
cargo build --release
if ($LASTEXITCODE -ne 0) { Write-Host "game build FAILED"; exit 1 }

Write-Host "[2/3] building launcher..."
Push-Location launcher
cargo build --release
$LAUNCHER_OK = $LASTEXITCODE
Pop-Location
if ($LAUNCHER_OK -ne 0) { Write-Host "launcher build FAILED"; exit 1 }

Write-Host "[3/3] assembling dist..."
$dist = "release_dist"
Remove-Item -Recurse -Force $dist -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path "$dist\game" | Out-Null

Copy-Item target\release\steel-front.exe "$dist\game\"
Copy-Item -Recurse assets "$dist\game\"
Copy-Item launcher\target\release\steel_front_launcher.exe "$dist\SteelFrontLauncher.exe"

$readmeLines = @()
$readmeLines += 'Steel Front - portable build'
$readmeLines += '================================='
$readmeLines += '1. Run SteelFrontLauncher.exe'
$readmeLines += '2. First run shows install wizard'
$readmeLines += '3. Desktop shortcut created after install'
$readmeLines += '4. Or run game\steel-front.exe directly'
$readmeLines += 'Feedback: https://github.com/Jerry-Hang/steel-front/issues/new'
Set-Content -Path "$dist\README.txt" -Encoding UTF8 -Value $readmeLines

$size = (Get-ChildItem $dist -Recurse -File | Measure-Object Length -Sum).Sum / 1MB
Write-Host ("DONE: $dist ({0:N1} MB)" -f $size)