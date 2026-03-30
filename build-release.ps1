$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$vsDevCmd = "C:\BuildTools\Common7\Tools\VsDevCmd.bat"
if (-not (Test-Path $vsDevCmd)) {
    throw "No se encontro VsDevCmd.bat en $vsDevCmd"
}

$cargoCmd = "cargo build --release"

cmd /c "call `"$vsDevCmd`" -arch=x64 -host_arch=x64 && set PATH=%USERPROFILE%\.cargo\bin;%PATH% && $cargoCmd"
