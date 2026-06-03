# Development

## Requisitos

- Rust (MSVC toolchain en Windows)
- Windows: Visual Studio Build Tools con workload C++
- Linux: `cargo build --release` directo

## Compilar

**Windows:**
```powershell
.\build-release.ps1
```

**Linux:**
```bash
cargo build --release
```

## Generar release

**Windows:**
```powershell
.\package-release.ps1
```

Genera `dist/mdnav-windows-x86_64.zip`.

**Linux:**
```bash
cargo build --release
tar -czf mdnav-linux-x86_64.tar.gz -C target/release mdnav
```

## Publicar en GitHub

```powershell
gh release create vX.X.X dist\mdnav-windows-x86_64.zip --title "vX.X.X" --notes "..."
gh release upload vX.X.X mdnav-linux-x86_64.tar.gz --repo csr91/mdnav
```

## Estructura

```
src/
  main.rs       — entrada, loop principal, shell hooks
  app.rs        — estado de la app, handlers de teclado
  ui.rs         — rendering con ratatui
  config.rs     — configuracion de usuario
  docs.rs       — árbol de archivos
  markdown.rs   — preview y syntax highlighting
  strings.rs    — textos i18n (es/en)
```
