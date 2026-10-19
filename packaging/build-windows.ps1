# Builds the Windows package: dist\rekordbox-rs-<version>-windows-x64.zip
#
#   powershell -ExecutionPolicy Bypass -File packaging\build-windows.ps1 [-SkipTests]
#
# Release build with the C runtime linked statically (no Visual C++ Redistributable needed), in its
# own target folder so that normal builds are not invalidated. The tests run first unless -SkipTests.
param([switch]$SkipTests)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$version = (Select-String -Path "$root\ui\Cargo.toml" -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$name = "rekordbox-rs-$version-windows-x64"
$dist = Join-Path $root 'dist'
$stage = Join-Path $dist $name
Write-Host "== $name =="

if (-not $SkipTests) {
    Write-Host '-- Tests'
    cargo test --workspace --release
    if ($LASTEXITCODE -ne 0) { throw 'tests failed' }
}

Write-Host '-- Release-Build (statische C-Laufzeit)'
$env:RUSTFLAGS = '-C target-feature=+crt-static'
cargo build --release --target-dir target\dist -p rbx-ui -p rbx-engine -p rekordbox-formats -p rbx-link --bins
$build = $LASTEXITCODE
Remove-Item Env:RUSTFLAGS
if ($build -ne 0) { throw 'build failed' }

if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force $stage | Out-Null
foreach ($exe in 'rbx-ui', 'rbx-play', 'rbx-analyze', 'rbx-selftest', 'rbf', 'link-announce', 'link-sniff') {
    Copy-Item "target\dist\release\$exe.exe" $stage
}
foreach ($f in 'install.ps1', 'uninstall.ps1', 'Installieren.cmd', 'Selbsttest.cmd', 'Controller-Test.cmd', 'Link-Diagnose.cmd', 'Link-Anmeldung.cmd', 'Link-Mitschnitt.cmd', 'link-capture.ps1', 'LIESMICH.txt') {
    Copy-Item "packaging\windows\$f" $stage
}
Copy-Item 'ui\assets\icon.ico' $stage

# the executables must not need the Visual C++ runtime DLLs
foreach ($exe in Get-ChildItem "$stage\*.exe") {
    $bytes = [System.IO.File]::ReadAllBytes($exe.FullName)
    $text = [System.Text.Encoding]::ASCII.GetString($bytes)
    if ($text -match 'VCRUNTIME140\.dll' -or $text -match 'MSVCP140\.dll') { throw "$($exe.Name) still imports the Visual C++ runtime" }
}

$zip = Join-Path $dist "$name.zip"
if (Test-Path $zip) { Remove-Item $zip }
Compress-Archive -Path "$stage\*" -DestinationPath $zip
$size = [math]::Round((Get-Item $zip).Length / 1MB, 1)
Write-Host "-- fertig: $zip ($size MB)"
