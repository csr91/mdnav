# mdnav

Workspace de terminal para el día a día del desarrollador: explorá y editá tu repo, probá APIs, consultá bases de datos y manejá git, todo en una sola TUI.

- **Explorar y editar**: árbol de cualquier directorio con crear, renombrar, mover, copiar y borrar; búsqueda de archivos y `grep`; bookmarks y recientes; edición con tu editor favorito.
- **Preview**: código de cualquier lenguaje con syntax highlighting, Markdown renderizado con índice, links y diagramas Mermaid.
- **Studio HTTP** (`:@http`): cliente de APIs con collections [OpenCollection](https://www.opencollection.com/) versionables en git, environments, secretos en `.env`.
- **Studio SQL** (`:@sql`): conexiones y queries guardadas para SQLite, PostgreSQL y MySQL, con resultados paginados, TLS y explorador de tablas y columnas.
- **Git**: estado en el árbol y status, diff, commit, push, branches y stash desde la paleta.
- **Compartir** (`mdnav share`): links efímeros para logs, JSON, CSV, diffs o texto.

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
mdnav                # abre el directorio actual
mdnav ruta/proyecto  # abre una carpeta específica
mdnav --version      # versión instalada
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

`:q` `:files` `:find` `:grep` `:marks` `:recent` `:create` `:git` `:@http` `:@sql` `:@jsonviewer` `:select` `:edit` `:rename` `:move` `:copy` `:delete` `:copypath` `:goto` `:toc` `:mermaid` `:fullscreen` `:bookmark` `:bookmarks` `:gitinfo` `:split1..5` `:sort` `:treeinfo` `:undo` `:@commands`

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

## Studio SQL

`:@sql` abre el estudio de bases de datos, esté donde esté el directorio de trabajo. Hay tres motores: **SQLite** (vía `rusqlite`), **PostgreSQL** (vía `postgres`) y **MySQL** (vía `mysql`). El motor se elige al crear la conexión y lo comparten todos sus entornos; queda guardado como `engine` en `connection.toml`. Las conexiones viven en `~/.config/mdnav/sql` (o en `sql_connections_dir` del config), una carpeta por conexión:

```
sql/
└── ejemplo/
    ├── connection.toml    # motor y entornos de la conexión
    ├── .env               # valores de los entornos secretos
    ├── ejemplo.sqlite     # la base de ejemplo
    └── listar-notas.sql   # una query por archivo
```

La primera vez aparece `ejemplo`, una conexión SQLite con una tabla `notas` y la query `listar-notas`. `c` crea otra conexión: pide el nombre, el motor y el valor de su entorno `default`. `n` agrega una query.

El valor de un entorno depende del motor:

| Motor | Se escribe como | Valor del entorno |
|-------|-----------------|-------------------|
| SQLite | `sqlite` o `sqlite3` | ruta del archivo `.sqlite` |
| PostgreSQL | `postgres`, `postgresql` o `pg` | `postgres://usuario:clave@host:5432/base` |
| MySQL | `mysql`, `maria` o `mariadb` | `mysql://usuario:clave@host:3306/base` |

El panel izquierdo es el árbol de conexiones: cada conexión muestra su motor (`[sqlite]`, `[postgres]`, `[mysql]`) y despliega sus entornos (`env:`, con 🔒 si es secreto), la rama `Tablas` y sus queries. A la derecha está la query arriba y el resultado abajo. El borde marca el panel con foco y el pie cambia con ese panel.

Para ejecutar hace falta una conexión activa y un entorno. `Enter` sobre una conexión la despliega y la activa (`(activa)`), sobre un entorno lo elige (`(activo)`) y sobre una query la carga en el editor. Si la conexión tiene un solo entorno, se elige solo. El entorno activo se guarda en el config con la ruta canónica de la conexión.

| Tecla | Acción |
|-------|--------|
| `j` / `k` o `↑` / `↓` | moverse en el árbol; en el resultado, filas |
| `h` / `l` o `←` / `→` | en el resultado, columnas |
| `Tab` / `Shift+Tab` | árbol → query → resultado (`Shift+Tab` al revés) |
| `Enter` | en el árbol, activar conexión, elegir entorno, abrir query o desplegar `Tablas` y una tabla; en la query, ejecutar |
| letra | en la query, escribe; `Backspace` borra |
| `c` | conexión nueva: nombre, motor y valor |
| `Shift+E` | entorno nuevo: nombre, valor (ruta o connection string, según el motor) y si es secreto (`y` / `n`) |
| `n` | query nueva con el texto del editor |
| `r` | renombrar (el nombre no puede llevar `.` ni `/`) |
| `y` | duplicar la conexión, el entorno o la query |
| `x` | borrar, con confirmación; en el resultado, cancelar la consulta en curso |
| `PgDn` / `PgUp` | página siguiente / anterior del resultado |
| `Esc` | salir del estudio |

Cada conexión puede tener varios entornos (dev, staging, prod...), cada uno con su valor. Un entorno secreto no guarda el valor en `connection.toml`: va al `.env` de la conexión, bajo el nombre del entorno en mayúsculas. El entorno `default` que crea `c` no es secreto: si el connection string lleva una clave, creá un entorno secreto con `Shift+E`. Duplicar una conexión copia la carpeta entera, `.env` incluido.

**TLS (PostgreSQL y MySQL):** para el caso común no hay que configurar nada. PostgreSQL respeta el `sslmode` del connection string: por defecto (`prefer`) usa TLS si el servidor lo soporta y si no sigue en texto plano, y el certificado se valida contra los certificados raíz del sistema. MySQL hace lo mismo a mano: intenta con TLS y, si el servidor responde que no lo soporta, reintenta sin cifrar. Para un servidor con certificado autofirmado o de una CA interna, agregá `sslinsecure=true` (o `sslinsecure=1`) a los parámetros de la URL, por ejemplo `postgres://usuario:clave@host:5432/base?sslmode=require&sslinsecure=true`: la conexión sigue cifrada, pero no se valida quién firmó el certificado. mdnav saca ese parámetro antes de pasarle la URL al driver.

**Tablas:** `Enter` sobre la rama `Tablas` de una conexión lista sus tablas usando el entorno activo de esa conexión (o el único que tenga), y `Enter` sobre una tabla muestra sus columnas con el tipo, 🔑 si es primary key y `NOT NULL` si no acepta nulos. Mientras carga se ve `Tablas (cargando...)`, después la cantidad de tablas; si falla se ve el error y otro `Enter` reintenta. En SQLite salen las tablas de `sqlite_master` (sin las internas `sqlite_*`), en PostgreSQL las del schema `public` y en MySQL las de la base del connection string. La lista se carga una vez y queda en memoria hasta que salís del estudio: después `Enter` solo la pliega o la despliega, aunque cambies de entorno. Se carga un esquema por vez.

Ejecutar con `Enter` escribe la query en su `.sql` si vino del árbol, y la escritura es atómica. No hay otro guardado. Si salís con texto que no guardaste ni ejecutaste, pregunta `g` guardar, `d` descartar o `Esc` seguir editando; si ese texto no tiene archivo, `g` pide un nombre y lo guarda como query nueva.

Cada ejecución corre una sola sentencia y abre su propia conexión. Un `SELECT` trae el resultado de a 200 filas: `PgDn` vuelve a correr la última query ejecutada (no la que estés escribiendo) desde la fila siguiente y `PgUp` vuelve a la página anterior. La barra inferior muestra `Filas X-Y` y avisa si hay más. Un `INSERT`, `UPDATE`, `DELETE` o DDL muestra las filas afectadas. Las celdas se truncan al ancho de la columna y `NULL` se ve como `NULL`. En SQLite un blob se ve como `<blob N bytes>`; en PostgreSQL un tipo que mdnav no sabe mostrar (arrays, intervalos, `bytea`...) se ve como `<tipo no soportado>`, por ejemplo `<bytea no soportado>`; en MySQL todo valor se muestra como texto.

Las consultas salen en un thread aparte: la UI no se congela. Mientras una corre, `x` en el resultado la cancela según el motor: SQLite la interrumpe directamente, PostgreSQL manda el pedido de cancelación del protocolo por una conexión aparte y MySQL abre otra conexión y ejecuta `KILL QUERY` sobre la de la consulta, así que el usuario necesita permiso para hacerlo. Si la conexión todavía no terminó de abrirse, `x` no hace nada. `Esc` sale del estudio abandonándola.

**Fuera de alcance por ahora:** otros motores además de SQLite, PostgreSQL y MySQL, cambiar el motor de una conexión desde el estudio, schemas de PostgreSQL fuera de `public` en `Tablas` y recargar `Tablas` sin salir del estudio.

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
