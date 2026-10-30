# Requests HTTP

mdnav envía requests HTTP guardados como archivos YAML, al estilo Postman o Bruno, sin salir de la terminal. Cada request es un archivo del repo: se versiona con git y se revisa en un PR como cualquier otro cambio.

## Probalo en un minuto

`:@http` no mira la carpeta desde la que abriste mdnav. Podés estar en cualquier lado:

```powershell
mdnav .                       # o mdnav, o mdnav DocExample
```

**1. Abrí el estudio**

- `:` abre la palette. Escribí `@http` y `Enter`.
- A la izquierda está tu biblioteca (`~/.config/mdnav/collections`). La primera vez trae `ejemplo`, un GET a httpbin. ★ marca el request abierto.
- La línea `→` ya muestra `https://httpbin.org/get`. No hace falta elegir environment: si la collection tiene uno solo, queda activo.

**2. Enviá**

- `S`: la response llega abajo con el JSON. El borde grueso pasa a la response.
- `j`/`k` hacen scroll, `]` cambia a Headers e Info, `y` copia el body.

**3. Cambiá algo sin tocar YAML**

- `Tab` vuelve al editor. Con la URL enfocada, las letras escriben (no disparan atajos). `m`, fuera de un campo de texto, cambia el método. El título dice `guardado`.
- `Esc` deja el campo. Otro `Esc` pide confirmación; el segundo sale. Al volver a entrar, la response sigue ahí.

`c` crea otra collection en esa carpeta, `/` busca, `n` agrega un request. `DocExample/api` sigue siendo una collection de ejemplo en el repo: copiala dentro de la biblioteca si querés esos requests. `Shift+E` abre el YAML crudo.

Para secretos, el `.env` vive al lado de la collection (en la biblioteca, no en el repo del proyecto).

## Ejemplos para aprender

`api/ejemplos/` tiene un request por concepto. Todos van contra httpbin, que te devuelve lo que mandaste: cambialo en el estudio (`:@http`), enviá con `S` y mirá el efecto en la response. Params, Headers, Body y Auth son pestañas del editor. `Shift+E` abre el YAML si querés ver el archivo.

| Archivo | Qué muestra |
|---------|-------------|
| `01-get-y-cambiar-metodo.yml` | el request mínimo y cómo pasar de GET a POST, DELETE, etc. |
| `02-query-y-path-params.yml` | `:id` en la URL, query params encodeados y apagados |
| `03-post-json.yml` | body JSON en varias líneas, con variables adentro |
| `04-post-form.yml` | body de formulario (`form-urlencoded`) |
| `05-headers.yml` | headers propios y los heredados de la collection |
| `06-token-bearer.yml` | **dónde poner tokens**: en el request, en el environment o en `.env` |
| `07-api-key-secreta.yml` | variables `secret: true` y API key en header o query |
| `08-auth-heredada/` | `folder.yml` con auth, headers y variables que heredan sus requests |
| `09-status-timeout-redirects.yml` | colores de status, timeout y redirects |

Para que los ejemplos 06 a 08 usen tus valores, creá `DocExample/api/.env` con:

```
DEMO_TOKEN=lo-que-quieras
demo_api_key=mi-clave
```

## Cómo funciona

```mermaid
flowchart LR
    A[request yml] --> R[Resolver]
    C[defaults yml] --> R
    E[environment] --> R
    S[archivo env] --> R
    R --> W[Worker thread]
    W --> P[Panel Response]
```

- El resolver junta los defaults heredados (`opencollection.yml` y los `folder.yml`: variables, headers y auth), el environment activo y los secretos del `.env`.
- El request sale en un thread aparte: la UI no se congela y `Esc` lo abandona.
- La response queda en memoria por request y sobrevive a `Shift+E` (el YAML crudo, y al volver se reabre el estudio).

## El formato: OpenCollection

Es el formato abierto que Bruno usa de forma nativa (spec en opencollection.com). Que mdnav no tenga un formato propio es a propósito: la misma carpeta se abre en Bruno, y quien no usa mdnav no queda afuera.

```
api/
├── opencollection.yml     # defaults de toda la collection
├── .env                   # secretos (gitignored)
├── environments/
│   └── public.yml
├── github/
│   ├── folder.yml         # defaults del folder
│   ├── get-user.yml
│   └── list-repos.yml
└── echo/
    ├── folder.yml         # auth bearer heredada por sus requests
    └── post-json.yml
```

## Variables y secretos

| Sintaxis | De dónde sale |
|----------|---------------|
| `{{nombre}}` | collection < environment < folders (gana el más específico) |
| `{{process.env.NOMBRE}}` | `.env` de la collection y después el entorno del proceso |
| `secret: true` en un environment | se busca por nombre en `.env` |

Si una variable de la URL no se resuelve, el request no se envía y mdnav te pide elegir un environment (`e` o `:@http env`).

## Teclas del estudio

El pie cambia según el panel. En el editor dice `h/l columnas · [ ] pestañas · letra edita · m método · S enviar`.

| Tecla | Acción |
|-------|--------|
| `:@http` | abrir el estudio, desde cualquier carpeta |
| `:@http env` | abrirlo y elegir environment |
| `j` `k` | moverse |
| `Tab` | árbol → editor → response |
| `h` `l` | árbol: entrar o volver. Tabla: cambiar de columna |
| `[` `]` | pestañas del editor o de la response |
| `m` | cambiar el método, si no estás escribiendo |
| letra | con un campo enfocado, editar y escribir |
| `Enter` | editar el campo |
| `+` `-` | agregar o quitar una fila |
| `n` | request nuevo |
| `c` | collection nueva en la biblioteca |
| `/` | buscar |
| `r` | renombrar (sin `.` ni `/`) |
| `y` | duplicar; en la response, copiar el body |
| `x` | borrar, con confirmación |
| `S` | enviar |
| `e` | elegir environment |
| `Shift+E` | editar el YAML crudo |
| `Esc` `Esc` | salir del campo y después confirmar la salida |

En 80×24 el árbol, el editor y la response se apilan. Más ancha, el árbol queda a la izquierda.

## Arrancar una collection propia

- La biblioteca está en `~/.config/mdnav/collections`. `http_collections_dir` en el config apunta a otra carpeta.
- `c` crea una collection ahí, con environment `local` en `https://httpbin.org` y un GET de ejemplo.
- `n` crea un request. `r` lo renombra, `y` lo duplica (el archivo usa guiones, no espacios) y `x` lo borra.
- Al dejar un campo, o al enviar, el estudio reescribe el YAML OpenCollection.

Importar un export de Postman desde el estudio queda para más adelante.

## Límites actuales

- Los scripts y assertions no se ejecutan; se avisa cuántos hay. Los scripts de Postman quedan como documentación del request.
- Sin bodies multipart ni file, y sin auth OAuth, AWS, digest ni NTLM.

## Siguiente

- [Volver a navegación](02-NAVIGATION.md)
- [Volver al inicio](../00-INTRO.md)
