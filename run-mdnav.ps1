$ErrorActionPreference = "Stop"

$vsDevCmd = "C:\BuildTools\Common7\Tools\VsDevCmd.bat"
if (-not (Test-Path $vsDevCmd)) {
    throw "No se encontro VsDevCmd.bat en $vsDevCmd"
}

$docsPath = if ($args.Count -gt 0) { $args[0] } else { "Documentation/docs" }

cmd /c "call `"$vsDevCmd`" -arch=x64 -host_arch=x64 && set PATH=%USERPROFILE%\.cargo\bin;%PATH% && cargo run -- `"$docsPath`""
