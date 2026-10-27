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

## Validar cambios

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

El mismo conjunto se ejecuta en Windows y Linux mediante GitHub Actions.

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

## Requests HTTP: notas de diseño

Decisiones que no se deducen leyendo un solo archivo:

- **Formato:** OpenCollection YAML, el formato nativo de Bruno. No hay formato propio, así la misma carpeta se abre en Bruno. El YAML se lee a `serde_json::Value` y no a structs, porque la spec acepta varias formas para un mismo campo (`auth: inherit` como texto u objeto, `params` como lista o separados por tipo).
- **Compatibilidad con Bruno:** un request sin `auth` va sin auth y solo `auth: inherit` hereda. Si la URL ya trae query string, los `params` de tipo query no se agregan (Bruno mantiene los dos sincronizados). La precedencia de variables es collection < environment < folders.
- **Ejecución:** `reqwest` blocking en un worker thread (`http::spawn_request`). `App::poll_http` recoge el resultado en cada tick, y el loop baja a 100 ms mientras hay un request en curso.
- **`Shift+E` relanza la `App`:** `main.rs` traslada `HttpSession` (responses y request en curso) y `AppConfig`. Cualquier estado nuevo que tenga que sobrevivir al editor va por ese camino.
- **Feedback:** con un archivo abierto el footer muestra metadata y no `app.status`, salvo en la vista HTTP. Para errores de una acción usá `show_alert`.
- **Teclas del HTTP Client:** `HTTP_KEY_SECTIONS` (`app.rs`) describe todo lo que maneja `handle_http_client_key`. La ayuda `?` y los hints del footer salen de esa tabla: si agregás o cambiás una tecla, actualizala ahí.
- **`serde_json` con `preserve_order`:** no lo saques. Sin eso, las keys de la response se ordenan alfabéticamente.
- **Tests:** `http.rs` levanta un servidor local (`one_shot_server`) para verificar lo que sale por el cable. Hay otro test que valida la collection de `DocExample/api`, así la demo no queda desactualizada.

## Estructura

```
src/
  main.rs       — entrada, loop principal, shell hooks
  app.rs        — estado de la app, handlers de teclado
  ui.rs         — rendering con ratatui
  config.rs     — configuracion de usuario
  docs.rs       — árbol de archivos
  http.rs       — requests HTTP: collections OpenCollection, environments, envío
  http_tree.rs  — árbol de collections, folders y requests del HTTP Client
  postman.rs    — import de collections y environments de Postman
  markdown.rs   — preview y syntax highlighting
  strings.rs    — textos i18n (es/en)
```
