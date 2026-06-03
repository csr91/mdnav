$ErrorActionPreference = "Stop"

$vsDevCmd = if (Test-Path "C:\BuildTools\Common7\Tools\VsDevCmd.bat") {
    "C:\BuildTools\Common7\Tools\VsDevCmd.bat"
} else {
    "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\Tools\VsDevCmd.bat"
}
if (-not (Test-Path $vsDevCmd)) {
    throw "No se encontro VsDevCmd.bat en $vsDevCmd"
}

cmd /c "call `"$vsDevCmd`" -arch=x64 -host_arch=x64 && set PATH=%USERPROFILE%\.cargo\bin;%PATH% && cargo build"
