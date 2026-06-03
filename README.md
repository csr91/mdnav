# mdnav

Explorador TUI para navegar documentacion Markdown y proyectos MkDocs desde la terminal.

## Instalacion rapida

Windows desde PowerShell:

```powershell
irm https://raw.githubusercontent.com/csr91/mdnav/main/install.ps1 | iex
```

Linux desde shell:

```bash
curl -fsSL https://raw.githubusercontent.com/csr91/mdnav/main/install.sh | bash
```

El instalador pregunta si instalar el shell hook para `Shift+G` (cd automatico al salir).

## Desinstalacion

Windows:

```powershell
irm https://raw.githubusercontent.com/csr91/mdnav/main/uninstall.ps1 | iex
```

Linux: elimina el binario de `~/.local/bin/mdnav` y la linea del hook en `~/.bashrc` o `~/.zshrc`.

## Uso

```bash
mdnav              # abre el directorio actual
mdnav ruta/docs    # abre una carpeta especifica
mdnav --version    # muestra la version instalada
```

## Controles

### Navegacion

| Tecla | Accion |
|-------|--------|
| `j` / `k` o `↑` / `↓` | mover seleccion en el arbol |
| `l` / `Enter` o `→` | expandir carpeta o abrir archivo |
| `h` / `Backspace` o `←` | colapsar carpeta o subir al padre |
| `Tab` / `Shift+Tab` | cambiar foco entre arbol y preview |
| `Shift+G` | dejar pendiente cd al directorio del item |
| `Shift+E` | abrir editor sobre el archivo actual |
| `Shift+0` | pantalla completa del panel enfocado |
| `Shift+1..5` | ajustar separacion entre paneles |
| `Ctrl+Shift+C` | copiar ruta del archivo seleccionado |
| `q` | salir |

### Preview

| Tecla | Accion |
|-------|--------|
| `j` / `k` o `,` / `.` | scroll linea a linea |
| `PgUp` / `PgDn` | scroll rapido |
| `h` / `l` o `[` / `]` | navegar links |
| `Shift+T` | tabla de contenidos |
| `Shift+M` | acciones Mermaid |
| `:` | command palette |
| `?` | ayuda |

### Seleccion y copia

| Tecla | Accion |
|-------|--------|
| `Shift+Y` | activar cursor de seleccion |
| `y` | marcar ancla (inicio de seleccion) |
| mover cursor | extiende el highlight |
| `y` | copiar seleccion al portapapeles |
| `Esc` | salir del modo seleccion |

## Integracion de shell

Para cd automatico al salir con `Shift+G`:

**PowerShell** (se configura automaticamente con el instalador):
```powershell
mdnav --shell-hook powershell >> $PROFILE
```

**Bash:**
```bash
echo 'source <(mdnav --shell-hook bash)' >> ~/.bashrc
```

**Zsh:**
```zsh
echo 'source <(mdnav --shell-hook zsh)' >> ~/.zshrc
```

## Settings

Abre `?` y ve a la pestana **Settings** (o **Ajustes**). Usa `↑↓` para seleccionar y `Enter` para cambiar:

- **Solo Mds / Only Mds**: muestra solo archivos `.md` en el arbol
- **Editor**: `nano` o `vim`, usado con `Shift+E`
- **Idioma / Language**: `es` (Espanol) o `en` (English)

La configuracion se guarda en:
- Windows: `%APPDATA%\mdnav\config.toml`
- Linux: `~/.config/mdnav/config.toml`

## Requisitos para compilar

- Rust
- Windows: Visual Studio Build Tools con workload C++
- Linux: `cargo build --release`
- Cross-build Linux desde Windows: Zig

## Generar release

Windows:
```powershell
.\package-release.ps1
```

Linux:
```bash
cargo build --release
tar -czf mdnav-linux-x86_64.tar.gz -C target/release mdnav
```
