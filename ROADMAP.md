# Roadmap

## v0.1.8 — actual

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
- [x] Instalador con un comando para Windows y Linux

## Proximo — v0.2.x

- [ ] Git status visual en el arbol (colores por archivo modificado/nuevo/ignorado)
- [ ] Mostrar tamaño de archivos en el arbol
- [ ] Ordenar arbol por nombre / fecha / tamaño
- [ ] Mover y copiar archivos entre directorios
- [ ] Plugin vim: abrir mdnav desde vim y volver al archivo seleccionado
- [ ] Subir de root mostrando el directorio padre completo

## Ideas a futuro

- [ ] Panel de chat integrado (Claude / Codex) — requiere threading
- [ ] Publicacion en winget para instalacion nativa en Windows
- [ ] Temas de color configurables
- [ ] Soporte para archivos `.rst` y `.txt`
- [ ] Preview de imagenes en terminal (protocolo Kitty/iTerm2)
