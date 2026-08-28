# Roadmap

## v0.1.11 — actual

- [x] Arbol navegable de carpetas y archivos `.md`
- [x] Preview Markdown con scroll independiente
- [x] Syntax highlighting en code blocks y archivos fuente (sh, py, etc.)
- [x] Deteccion y preview de bloques Mermaid (terminal, HTML, web)
- [x] Layout adaptable con separacion ajustable (`Shift+1..5`)
- [x] Pantalla completa por panel (`Shift+0`)
- [x] Navegacion vim: `hjkl`, foco entre paneles con `Tab`
- [x] Modo seleccion con highlight visual y copia al portapapeles
- [x] Command palette estilo vim (`:`)
- [x] Buscar archivo en el arbol (`/` desde palette)
- [x] Buscar texto en archivo (`find` desde palette)
- [x] Tabla de contenidos (`Shift+T`)
- [x] Navegar links del preview (`[` / `]`)
- [x] Abrir editor externo configurable: nano / vim (`Shift+E`)
- [x] Renombrar archivo o carpeta (`Shift+R`)
- [x] Eliminar con confirmacion (`Shift+X`)
- [x] Crear archivo o carpeta (`:create`)
- [x] Bookmarks de carpetas favoritas (`Shift+B`)
- [x] Subir por encima del root (`←` doble)
- [x] Cd automatico al directorio del item (`Shift+G` + shell hook)
- [x] Shell hook para PowerShell, Bash y Zsh
- [x] Copiar ruta al portapapeles (`Ctrl+Shift+C`)
- [x] Auto-refresh del arbol y preview al detectar cambios externos
- [x] Git integrado: status, log, diff, commit, push, pull (`:git`)
- [x] Settings persistentes: idioma (es/en), editor, only_mds, bookmarks
- [x] Info opcional en el arbol: tamanio de archivos o cantidad de lineas (`:treeinfo`)
- [x] Ordenar arbol por nombre / fecha / tamanio (`:sort`)
- [x] Git status visual en el arbol (modificado, staged, nuevo, ignorado, renombrado, eliminado, conflicto)
- [x] Instalador con un comando para Windows y Linux
- [x] Mover y copiar archivos entre directorios (`:move` / `:copy`, picker de carpetas navegable, copia recursiva de carpetas)
- [x] Configuracion TOML robusta con migracion automatica del formato anterior
- [x] Operaciones move/copy seguras: sin sobrescritura, rollback y fallback entre filesystems
- [x] Tests automatizados y CI en Windows/Linux

## Proximo — v0.2.x

- [ ] Plugin vim: abrir mdnav desde vim y volver al archivo seleccionado
- [ ] Subir de root mostrando el directorio padre completo

## Ideas a futuro

- [ ] Schema browser interactivo para `erDiagram`: grid de entidades navegable con Tab, relaciones numeradas para saltar entre cajas, resaltado de entidades relacionadas — sin necesidad de renderizar líneas
- [ ] Telemetría de documentación para teams: evento silencioso al abrir archivo (token por user/team, path hasheado, timestamp) — dashboard web muestra docs más leídas, docs sin actividad, historial de onboarding. Token ya existe vía `MDNAV_WEB_WRITE_TOKEN`, falta definir granularidad (token por equipo vs por usuario) y política de opt-in
- [ ] Panel de chat integrado (Claude / Codex) — requiere threading
- [ ] Publicacion en winget para instalacion nativa en Windows
- [ ] Temas de color configurables
- [ ] Soporte para archivos `.rst` y `.txt`
- [ ] Preview de imagenes en terminal (protocolo Kitty/iTerm2)
