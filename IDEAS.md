# IDEAS

Ideas de producto — sin compromiso de fecha ni prioridad.

---

## 1. Share efímero generalizado — hecho

`mdnav share --file/--stdin/--clipboard` ya comparte cualquier contenido (texto, log, csv, json, diff, ademas de mermaid), reusando la infraestructura existente de mdnav-web (TTL, one-time view, colección `shares` en Mongo). El endpoint `/api/diagrams/{hash}` acepta `{kind, content, title, filename, ttlSeconds}` y el receptor renderiza segun `kind` (tabla para CSV, colores para diff, resaltado de error/warn en logs, etc.).

---

## 2. Share efímero para equipos (Teams)

Extensión del punto anterior orientada a equipos pequeños:

- Token de equipo compartido
- Dashboard efímero: cada miembro ve los shares activos del equipo con su TTL
- `mdnav share --to @team` para compartir directamente al canal del equipo
- Caso de uso: devs compartiendo logs, queries, dumps de datos sin salir de la terminal ni usar Slack/email

---

## 3. Server mode — base de documentación compartida

mdnav deja de leer solo el filesystem local y puede montar un árbol remoto:

```
mdnav connect empresa.mdnav.app
```

Misma UX (hjkl, preview, :find), pero el contenido es del equipo en lugar del disco local.

**MVP — read only:**
- El servidor expone docs, los clientes solo leen
- Sin edición ni conflictos
- Suficiente para el caso KaizenLab (ver abajo)
- mdnav-web como backend natural

**Versión completa — wiki colaborativa:**
- Edición compartida, cambios en tiempo real
- Historial, auth por usuario
- Compite con Notion/Confluence pero desde terminal

### Caso KaizenLab
Cada implementación de CRM/ERP genera documentación técnica enorme (arquitectura, mapeo de campos, runbooks, queries). KaizenLab podría:
1. Entregar cada proyecto como un workspace de mdnav
2. El equipo técnico del cliente lo navega desde terminal
3. KaizenLab lo actualiza remotamente, el cliente siempre tiene la versión más reciente

Diferencial concreto en el pitch: *"te entregamos la implementación y la documentación navegable desde tu terminal"*.

---

## 4. GitHub remoto desde el árbol

`:github` monta el repo remoto como si fuera local:
- Explorar branches, ver último commit por archivo
- Ver PRs abiertos, issues
- Sin salir de mdnav, sin browser

---

## 5. Otros features de navegación

- Visor diff paralelo / combinado para archivos: mostrar versión vieja y nueva alineadas, o una vista de archivo combinado con líneas agregadas/eliminadas/modificadas resaltadas. MVP posible: `:diff` sobre el archivo seleccionado usando Git; versión posterior con dos columnas y scroll sincronizado.
- Preview de notebooks `.ipynb`
- Editar CSV/JSON inline
- Preview de PDFs (texto extraído)
