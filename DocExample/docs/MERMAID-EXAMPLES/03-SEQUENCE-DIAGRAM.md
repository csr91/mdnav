# Mermaid Example: Sequence Diagram

## Objetivo

Este ejemplo sirve para validar otro tipo de diagrama distinto de flowchart.

```mermaid
sequenceDiagram
    Alice ->> Bob: Hello Bob, how are you?
    Bob-->>John: How about you John?
    Bob--x Alice: I am good thanks!
    Bob-x John: I am good thanks!
    Note right of John: Bob thinks a long time, so long that the text does not fit on a row.
    Bob-->Alice: Checking with John...
    Alice->John: Yes... John, how are you?
```

## Notas

- Bueno para validar como cae el preview cuando no es un flowchart
- Bueno para probar salida HTML o web
- Volver al [indice Mermaid](00-INDEX.md)

