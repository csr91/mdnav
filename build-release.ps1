$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$vsDevCmd = if (Test-Path "C:\BuildTools\Common7\Tools\VsDevCmd.bat") {
    "C:\BuildTools\Common7\Tools\VsDevCmd.bat"
} else {
    "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\Tools\VsDevCmd.bat"
}
if (-not (Test-Path $vsDevCmd)) {
    throw "No se encontro VsDevCmd.bat en $vsDevCmd"
}

$cargoCmd = "cargo build --release"

$binaryPath = Join-Path $repoRoot "target\release\mdnav.exe"
if (Test-Path $binaryPath) {
    Remove-Item -LiteralPath $binaryPath -Force
}

cmd /v:on /c "call `"$vsDevCmd`" -arch=x64 -host_arch=x64 && set `"PATH=C:\Program Files\Rust stable MSVC 1.95\bin;%USERPROFILE%\.cargo\bin;!PATH!`" && $cargoCmd"
if ($LASTEXITCODE -ne 0) {
    throw "cargo build --release fallo con codigo $LASTEXITCODE"
}
