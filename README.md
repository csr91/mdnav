# mdnav

Explorador TUI para navegar documentacion Markdown y proyectos MkDocs desde la terminal.

Navega el arbol de archivos, previsualiza Markdown con syntax highlighting, editá con tu editor favorito y gestioná tu repo git — todo sin salir de la terminal.

## Instalacion

**Windows (PowerShell):**
```powershell
irm https://raw.githubusercontent.com/csr91/mdnav/master/install.ps1 | iex
```

**Linux:**
```bash
curl -fsSL https://raw.githubusercontent.com/csr91/mdnav/master/install.sh | bash
```

El instalador pregunta si configurar el shell hook para cd automático con `Shift+G`.

## Desinstalacion

**Windows:**
```powershell
irm https://raw.githubusercontent.com/csr91/mdnav/master/uninstall.ps1 | iex
```

**Linux:** eliminá `~/.local/bin/mdnav` y la línea del hook en `~/.bashrc` o `~/.zshrc`.

## Uso

```bash
mdnav              # abre el directorio actual
mdnav ruta/docs    # abre una carpeta específica
mdnav --version    # versión instalada
```

## Controles

### Navegación

| Tecla | Acción |
|-------|--------|
| `j` / `k` o `↑` / `↓` | mover selección en el árbol |
| `l` / `Enter` o `→` | expandir carpeta o abrir archivo |
| `h` / `Backspace` o `←` | colapsar carpeta o subir al padre |
| `← ←` (dos veces) | subir un nivel por encima del root |
| `Tab` / `Shift+Tab` | cambiar foco entre árbol y preview |
| `Shift+G` | cd pendiente al directorio del item |
| `Shift+E` | abrir archivo en el editor configurado |
| `Shift+R` | renombrar archivo o carpeta |
| `Shift+X` | eliminar con confirmación |
| `Shift+B` | marcar/desmarcar bookmark |
| `Shift+0` | pantalla completa del panel enfocado |
| `Shift+1..5` | ajustar proporción entre paneles |
| `Ctrl+Shift+C` | copiar ruta del archivo seleccionado |
| `q` | salir |

### Preview

| Tecla | Acción |
|-------|--------|
| `j` / `k` o `,` / `.` | scroll línea a línea |
| `PgUp` / `PgDn` | scroll rápido |
| `h` / `l` o `[` / `]` | navegar links |
| `Shift+T` | tabla de contenidos |
| `Shift+M` | acciones Mermaid |

### Selección y copia

| Tecla | Acción |
|-------|--------|
| `Shift+Y` | activar cursor de selección |
| `y` | marcar ancla (inicio de selección) |
| mover cursor | extiende el highlight |
| `y` | copiar al portapapeles y soltar ancla |
| `Esc` | salir del modo selección |

### Command palette (estilo vim)

Presioná `:` para abrir. Comandos disponibles:

`:q` `:files` `:find` `:create` `:git` `:select` `:edit` `:rename` `:delete` `:copypath` `:goto` `:toc` `:mermaid` `:fullscreen` `:bookmark` `:bookmarks` `:gitinfo` `:split1..5` `:sort` `:treeinfo`

`:treeinfo` alterna la informacion mostrada a la derecha de cada archivo en el arbol: tamanio, lineas u oculto.

`:sort` alterna el orden del arbol: nombre, fecha de modificacion o tamanio.

## Settings

Abrí `?` → pestaña **Settings** (o **Ajustes**). Navegá con `j`/`k` y cambiá con `Enter`:

| Setting | Valores | Descripción |
|---------|---------|-------------|
| Solo Mds | ON / OFF | muestra solo `.md` en el árbol |
| Editor | nano / vim | editor que abre `Shift+E` |
| Idioma | es / en | idioma de la interfaz |
| Bookmarks | ON / OFF | muestra/oculta bookmarks en el árbol |

La configuración se guarda en:
- Windows: `%APPDATA%\mdnav\config.toml`
- Linux: `~/.config/mdnav/config.toml`

## Shell hook

Para que `Shift+G` cambie el directorio automáticamente al salir:

**PowerShell** (se configura solo con el instalador):
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

## Bookmarks

Marcá carpetas favoritas con `Shift+B` — aparecen con `★` al tope del árbol. Activar un bookmark cambia el root a esa carpeta. Toglear visibilidad con `:bookmarks` o desde Settings.

## Git visual

Cuando el root está dentro de un repo Git, el árbol muestra indicadores de estado:

| Símbolo | Estado |
|---------|--------|
| `M` | modificado |
| `A` | staged |
| `?` | nuevo sin trackear |
| `!` | ignorado |
| `R` | renombrado |
| `D` | eliminado |
| `U` | conflicto |

Usá `:gitinfo` para mostrar u ocultar estos indicadores.

## Autor

Cesar Mendoza — cesar.mendoza.77@gmail.com
