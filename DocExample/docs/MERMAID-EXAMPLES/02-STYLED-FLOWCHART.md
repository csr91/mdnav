# Mermaid Example: Styled Flowchart

## Objetivo

Este ejemplo empuja un poco mas el renderer con labels largos, subgraph y estilos.

```mermaid
graph TB
    sq[Square shape] --> ci((Circle shape))

    subgraph A
        od>Odd shape] -- Two line edge comment --> di{Diamond with line break}
        di -.-> ro(Rounded square shape)
        di ==> ro2(Rounded square shape)
    end

    e --> od3>Really long text with linebreak in an Odd shape]
    e((Inner / circle and some odd special characters)) --> f(,.?!+-*ز)
    cyr[Cyrillic] --> cyr2((Circle shape Начало))

    classDef green fill:#9f6,stroke:#333,stroke-width:2px;
    classDef orange fill:#f96,stroke:#333,stroke-width:4px;
    class sq,e green;
    class di orange;
```

## Notas

- Bueno para probar fallback en preview
- Bueno para probar web link en Mermaid mas complejo
- Volver al [indice Mermaid](00-INDEX.md)

