$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$zigDir = Get-ChildItem -Path $env:LOCALAPPDATA -Recurse -Filter zig.exe -ErrorAction SilentlyContinue |
    Select-Object -First 1 -ExpandProperty DirectoryName

if (-not $zigDir) {
    throw "No se encontro zig.exe. Instala Zig o deja disponible zig en PATH."
}

$env:PATH = "$zigDir;$env:USERPROFILE\.cargo\bin;$env:PATH"

& rustup target add x86_64-unknown-linux-musl | Out-Null
& cargo zigbuild --release --target x86_64-unknown-linux-musl
