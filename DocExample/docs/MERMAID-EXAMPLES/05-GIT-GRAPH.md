# Mermaid Example: Git Graph

## Objetivo

Este ejemplo sirve para probar una visual mas estructurada.

```mermaid
gitGraph:
    commit "Ashish"
    branch newbranch
    checkout newbranch
    commit id:"1111"
    commit tag:"test"
    checkout main
    commit type: HIGHLIGHT
    merge newbranch
    commit
    branch b2
    commit
```

## Notas

- Ideal para salida HTML o web link
- Bueno para ver como responde la deteccion de Mermaid con otra sintaxis
- Volver al [indice Mermaid](00-INDEX.md)

