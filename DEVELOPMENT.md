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

## Atajos de archivos

Crear, renombrar, duplicar y borrar usan las mismas teclas en el árbol de docs y en el estudio `:@http`:

| Tecla | Acción |
|-------|--------|
| `n` | crear |
| `r` | renombrar |
| `y` | duplicar |
| `x` | borrar, con confirmación |

En el árbol de `:@http` actúan sobre el request si el cursor está en un request, y sobre la collection si el cursor está en ella. En el editor de environments (`e`) actúan sobre el environment si el foco está en la lista, y sobre la variable si el foco está en la tabla. `s` marca esa variable como secreta. `c` crea una collection vacía y `C` una con el GET de ejemplo. `Shift+R` y `Shift+X` siguen abriendo renombrar y borrar en el árbol de docs, como alias de `r` y `x`.

En el JSON Viewer no hay un archivo que renombrar o duplicar. `n` y `x` vacían el texto (con confirmación si había algo) y `y` copia el JSON formateado: es la excepción, porque duplicar no aplica. `Shift+S` es enviar en el estudio HTTP; no es la `S` de un atajo de una sola tecla.

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
  postman.rs    — import de collections y environments de Postman
  markdown.rs   — preview y syntax highlighting
  strings.rs    — textos i18n (es/en)
```
