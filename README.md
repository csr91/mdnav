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

## Compartir contenido (`mdnav share`)

Genera un link web efímero (un solo acceso) para cualquier contenido, sin abrir el explorador:

```bash
mdnav share --file error.log        # comparte un archivo, detecta el tipo por extension (.log, .csv, .json, .diff/.patch, o texto)
mdnav share --stdin < salida.txt    # comparte lo que llega por stdin
mdnav share --clipboard             # comparte el texto del portapapeles
```

Opciones: `--title <texto>`, `--ttl <segundos>` (por defecto 3600), `--open` (abre el link en el navegador). El comando copia el link al portapapeles e imprime la URL por stdout.

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
| `!` `@` `#` `$` `%` | proporción entre paneles, del 1 al 5 |
| `Ctrl+Shift+C` | copiar ruta del archivo seleccionado |
| `:q` | salir |

### Preview

| Tecla | Acción |
|-------|--------|
| `j` / `k`, `↑` / `↓` o `,` / `.` | scroll línea a línea |
| `PgUp` / `PgDn` | scroll rápido |
| `h` / `l` o `[` / `]` | navegar links |
| `Shift+T` | tabla de contenidos |
| `Shift+M` | acciones Mermaid |

El preview muestra la ruta del archivo actual en el titulo y metadata breve en la barra inferior.

### Selección y copia

| Tecla | Acción |
|-------|--------|
| `Shift+Y` | activar cursor de selección |
| `y` | marcar ancla (inicio de selección) |
| mover cursor | extiende el highlight |
| `y` | copiar al portapapeles y soltar ancla |
| `Esc` | salir del modo selección |

### Command palette (estilo vim)

Presioná `:` para abrir y escribí el comando. La paleta chica prioriza tus comandos recientes y muestra sugerencias por prefijo mientras tipeás; tambien entiende algunas intenciones (`buscar`, `copiar`, `indice`, `recientes`). Usá `:@commands` para abrir el catálogo grande agrupado por secciones, con todos los comandos y su descripción.

`:q` `:files` `:find` `:grep` `:marks` `:recent` `:create` `:git` `:@http` `:@jsonviewer` `:select` `:edit` `:rename` `:move` `:copy` `:delete` `:copypath` `:goto` `:toc` `:mermaid` `:fullscreen` `:bookmark` `:bookmarks` `:gitinfo` `:split1..5` `:sort` `:treeinfo` `:@commands`

`:recent` abre los ultimos archivos visitados. `:marks` abre un popup filtrable con los bookmarks guardados. `:find` mantiene resaltada la ultima busqueda en el preview hasta abrir otro archivo o limpiar la busqueda. `:grep` busca texto en los archivos visibles del arbol y salta al resultado elegido.

`:move` y `:copy` abren un selector navegable de carpetas. Las operaciones no sobrescriben destinos existentes y la copia de carpetas se revierte si ocurre un error.

La barra inferior muestra siempre el modo activo (`TREE`, `PREVIEW`, `COMMAND`, `VISUAL`, `GREP`, etc.) para que sea mas claro donde estas parado. Cuando hay un archivo abierto tambien muestra posicion del preview (`line X/Y`, porcentaje), cantidad de lineas, tamanio y estado Git si corresponde.

`:treeinfo` alterna la informacion mostrada a la derecha de cada archivo en el arbol: tamanio, lineas u oculto.

`:sort` alterna el orden del arbol: nombre, fecha de modificacion o tamanio.

## Requests HTTP

mdnav envía requests HTTP guardados como archivos [OpenCollection](https://www.opencollection.com/) YAML, el formato abierto que Bruno usa de forma nativa. Una collection es una carpeta más del repo: se versiona con git y la misma carpeta se abre en Bruno.

```
api/
├── opencollection.yml     # config de la collection (auth, headers y variables heredables)
├── .env                   # secretos, ignorado por git
├── .env.sample            # nombres de los secretos, sin valores
├── environments/
│   ├── local.yml
│   └── prod.yml
└── users/
    ├── folder.yml         # defaults del folder
    └── get-user.yml       # un request por archivo
```

`:@http` abre el estudio esté donde esté el directorio de trabajo. Las collections viven en `~/.config/mdnav/collections` (o en `http_collections_dir` del config). La primera vez aparece `ejemplo`, un GET contra `https://httpbin.org/get`: `S` funciona sin elegir carpeta ni environment. `c` crea otra collection ahí, `/` filtra el árbol, `n` agrega un request.

El panel izquierdo es el árbol de collections (carpetas y requests; ★ es el abierto). A la derecha está el editor y abajo la response. En 80×24 se apilan. El borde grueso y el `●` marcan el panel con foco, y el pie cambia con ese panel. El editor muestra la URL escrita y, debajo en gris, la URL resuelta. Params y Headers son tablas de clave, valor y on/off.

| Tecla | Acción |
|-------|--------|
| `j` / `k` | moverse. Si hay cambios sin guardar, pregunta antes |
| `Tab` / `Shift+Tab` | árbol → editor → response |
| `h` / `l` | en el árbol, entrar o volver; en la tabla, cambiar de columna |
| `[` / `]` | pestañas del editor o de la response |
| `m` | cambiar el método (si no estás escribiendo un campo) |
| letra | si el campo está enfocado, entra en edición y escribe |
| `Enter` | editar el campo; `Esc` lo deja y guarda |
| `+` / `-` | agregar o quitar una fila |
| `n` | request nuevo |
| `c` / `C` | collection vacía / collection con el GET de ejemplo |
| `/` | buscar en el árbol |
| `r` | renombrar (el nombre no puede llevar `.` ni `/`) |
| `y` | duplicar (el archivo no lleva espacios; en la response, copiar el body) |
| `x` | borrar, con confirmación |
| `Shift+S` | enviar |
| `e` | editor de environments (`:@http env` también) |
| `Shift+E` | abrir el YAML en el editor |
| `Esc` `Esc` | salir del estudio; la response queda para la próxima vez |

El environment activo se guarda con la ruta canónica de la collection, así `mdnav .` y `mdnav /ruta` usan la misma clave.

Salir de un campo con `Esc` o `Enter`, y enviar con `Shift+S`, escriben el `.yml` OpenCollection. El título muestra `guardado` un momento. Si cambiás de panel, te movés en el árbol o salís del estudio con cambios sin guardar, pregunta `g` guardar, `d` descartar o `Esc` cancelar. No hay un guardado automático por tiempo. Si el disco falla, avisa y no tira lo que escribiste. `n`, `r`, `y` y `x` crean, renombran, copian y borran, tanto un request como la collection bajo el cursor.

Los requests salen en un thread aparte: la UI no se congela y `Esc` en la response abandona el que está en curso.

**Variables:** `{{nombre}}` se resuelve con esta precedencia: variables de la collection < environment < folders. `{{process.env.NOMBRE}}` lee el `.env` de la collection y después el entorno del proceso. Las variables `secret: true` de un environment no guardan valor en el archivo: mdnav las busca por nombre en `.env`. `e` edita esos environments (crear, renombrar, duplicar, borrar, y las variables) sin tocar el YAML a mano. Un secreto se escribe en `.env` y en pantalla se ve enmascarado.

**Auth:** en la pestaña Auth, `none`, `inherit`, `bearer`, `basic` y `apikey` (header o query). `inherit` toma la del folder más cercano o la de la collection. Los perfiles con nombre vienen después; `a` queda reservada para eso.

**Fuera de alcance por ahora:** perfiles de auth con nombre, pegar un curl, importar Postman desde el estudio, scripts y assertions (se avisa cuántos tiene el request), bodies multipart, file y GraphQL nativo, y auth OAuth, AWS, digest y NTLM.

Con **Solo Mds** activado, el árbol igual muestra los `.yml` de las collections y los exports de Postman.

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

Marcá carpetas favoritas con `Shift+B` — aparecen con `★` al tope del árbol. Activar un bookmark cambia el root a esa carpeta. `:marks` abre todos los bookmarks en un popup filtrable; `:bookmarks` o Settings alternan si se ven en el arbol.

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
