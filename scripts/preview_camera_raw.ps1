param(
    [string]$Image,
    [switch]$Build
)
$ErrorActionPreference = 'Stop'
$previewRoot = Split-Path -Parent $PSScriptRoot
$previewExe = Join-Path $previewRoot 'target/acr18/debug/photocraft.exe'
if ($Build) {
    Push-Location $previewRoot
    try {
        $env:CARGO_TARGET_DIR = Join-Path $previewRoot 'target/acr18'
        $env:CARGO_PROFILE_DEV_DEBUG = '0'
        $env:CARGO_PROFILE_DEV_INCREMENTAL = 'false'
        cargo build -p photocraft -j 4
        if ($LASTEXITCODE -ne 0) { throw 'The native preview build failed.' }
    } finally { Pop-Location }
}
if (-not (Test-Path -LiteralPath $previewExe)) {
    # Reuse the normal development build when the isolated preview was built on
    # another machine. Ordinary design launches must never start Cargo implicitly.
    $existingDebugExe = Join-Path $previewRoot 'target/debug/photocraft.exe'
    if (Test-Path -LiteralPath $existingDebugExe) {
        $previewExe = $existingDebugExe
    } else {
        throw 'No native preview executable is available. Build it once with ./scripts/preview_camera_raw.ps1 -Build.'
    }
}
$env:PHOTOCRAFT_CAMERA_RAW_LAYOUT_FILE = Join-Path $previewRoot 'design/camera-raw/layout.json'
$env:PHOTOCRAFT_THEME_FILE = Join-Path $previewRoot 'design/camera-raw/tokens.json'
$env:PHOTOCRAFT_CONFIG_DIR = Join-Path $previewRoot 'target/acr18/preview-profile'
$env:PHOTOCRAFT_LOCALE = 'en'
if (-not $Image) {
    $Image = Join-Path $previewRoot 'target/acr18/design-fixture.png'
    $fixtureScript = @'
from pathlib import Path
from PIL import Image
import colorsys, sys
p=Path(sys.argv[1]);p.parent.mkdir(parents=True,exist_ok=True)
w,h=1200,900
im=Image.new('RGB',(w,h))
im.putdata([tuple(round(v*255) for v in colorsys.hsv_to_rgb(x/(w-1), 0.25+0.65*y/(h-1), 0.95-0.65*y/(h-1))) for y in range(h) for x in range(w)])
im.save(p)
'@
    $fixtureScript | python -X utf8 - $Image
    if ($LASTEXITCODE -ne 0) { throw 'Could not create the synthetic preview fixture (requires Pillow).' }
}
if (-not (Test-Path -LiteralPath $Image)) { throw "Image not found: $Image" }
# An explicitly requested visible preview; keep its settings separate from the owner's editor.
Start-Process -FilePath $previewExe -WorkingDirectory $previewRoot -ArgumentList @('--raw-workspace', ('"' + (Resolve-Path -LiteralPath $Image).Path + '"'))
Write-Output "Using existing executable: $previewExe"
Write-Output "Live layout: $env:PHOTOCRAFT_CAMERA_RAW_LAYOUT_FILE"
Write-Output "Live colors: $env:PHOTOCRAFT_THEME_FILE"
Write-Output 'Save either JSON file to update the running preview. New processing behavior still requires Rust changes.'
