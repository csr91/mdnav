# mdnav

Explorador TUI para navegar documentacion Markdown y proyectos MkDocs desde la terminal.

## Estado actual

Este repositorio ya tiene un MVP tecnico usable:

- Arbol navegable de carpetas y archivos `.md`
- Preview Markdown con foco independiente
- Scroll de preview
- Deteccion de links Markdown
- Deteccion y preview amigable de bloques Mermaid
- Layout adaptable con separacion ajustable
- Build release para distribuir como CLI
- Carpeta `DocExample` lista para demo

## Instalacion rapida

Windows desde PowerShell:

```powershell
irm https://raw.githubusercontent.com/csr91/mdnav/main/install.ps1 | iex
```

Linux desde shell:

```bash
curl -fsSL https://raw.githubusercontent.com/csr91/mdnav/main/install.sh | bash
```

Despues deberias poder ejecutar:

```powershell
mdnav
```

o en Linux:

```bash
mdnav
```

## Uso rapido

Ejecutar con la carpeta demo incluida:

```powershell
.\run-mdnav.ps1
```

O explicitamente:

```powershell
.\run-mdnav.ps1 DocExample/docs
```

Tambien podes apuntar a cualquier otra carpeta con Markdown:

```powershell
.\run-mdnav.ps1 C:\ruta\a\docs
```

## Demo incluida

La carpeta `DocExample` trae:

- Markdown simple
- Navegacion entre archivos
- Subcarpetas
- Un Mermaid basico

La carpeta `Documentation` queda ignorada por Git para que no subas documentacion privada o interna por accidente.

## Requisitos para compilar

- Rust
- En Windows, Visual Studio Build Tools con C++
- Para cross-build Linux desde Windows, Zig

## Releases

Generar release Windows:

```powershell
.\build-release.ps1
.\package-release.ps1
```

Generar release Linux desde Windows:

```powershell
.\build-release-linux.ps1
.\package-release-linux.ps1
```

Generar ambos paquetes:

```powershell
.\package-all-releases.ps1
```

Artefactos resultantes:

```text
target/release/mdnav.exe
target/x86_64-unknown-linux-musl/release/mdnav
dist/mdnav-windows-x86_64.zip
dist/mdnav-linux-x86_64.tar.gz
```

## Controles

- `Up` / `Down`: mover seleccion en el arbol
- `Right` / `Enter`: expandir carpeta o abrir archivo
- `Left` / `Backspace`: colapsar carpeta o subir al padre
- `Tab` / `Shift+Tab`: cambiar foco entre arbol y preview
- `,` / `.`: scroll de preview
- `Ctrl+Enter`: alternar preview completo
- `Shift+1..5`: ajustar separacion entre navegacion y preview
- `q`: salir
