---
name: codemap
description: Regenerar/actualizar docs/CODEMAP.md — el índice función-por-función (con nº de línea) de los hot paths del proyecto (juego web app.js + game.rs + motor de simulación + optimizador). Úsala cuando el CODEMAP esté desfasado tras cambios grandes, cuando el usuario pida "actualiza el mapa de código" / "refresca el codemap", o antes de retomar trabajo en una zona caliente para tener anclas de línea frescas.
---

# Skill: /codemap — refrescar el mapa de código

Mantiene `docs/CODEMAP.md` sincronizado con el código. El mapa existe para **ahorrar tokens de
contexto**: permite saltar directo a `fichero:línea` en vez de releer ficheros enteros. Su valor
depende de que los nº de línea y las funciones estén al día.

## Alcance (SOLO hot paths — no ampliar sin que el usuario lo pida)

- `game/web/app.js` — motor del juego web (bucle por frame)
- `src/game.rs` — backend JSON del juego (`/api/game/*`)
- `src/simulation_engine.rs` — motor de eventos discretos
- `src/signaling.rs` — lógica de tres aspectos
- `src/topology.rs` — datos operativos (vías, vía única, corredores)
- `src/optimizer/*.rs` — annealing + potencial V(H)

`CLAUDE.md` ya cubre el resto de módulos a nivel de arquitectura; NO los dupliques aquí.

## Procedimiento

1. **Commit de referencia.** Ejecuta `git rev-parse --short HEAD` y la fecha, para el encabezado.

2. **Extrae símbolos con nº de línea** de los ficheros del alcance. Usa Grep (output_mode
   `content`, `-n true`):
   - Rust: patrón `^\s*(pub\s+)?(async\s+)?fn\s+\w+|^\s*pub\s+struct\s+\w+|^\s*pub\s+enum\s+\w+|^\s*impl\s+\w+`
     (añade `|^\s*const\s+\w+|^\s*static\s+\w+` en `topology.rs` porque las tablas de datos
     son const relevantes).
   - JS (`app.js`): `^\s*(async\s+)?function\s+\w+` para las funciones con nombre. Además localiza
     a mano el objeto de estado `S` (`const S =`), las constantes de cabecera y `const map`/`const $`.
   - Para bucles muy largos con fases marcadas por comentarios (p. ej. `Simulator::run`), extrae
     las fases con `^\s*//\s*---` para listarlas como sub-anclas dentro de la función.

3. **Reescribe `docs/CODEMAP.md`** conservando su estructura de secciones (1–6 + "Cómo usar").
   Para cada fichero: una línea de propósito + tablas `| Símbolo | Línea | Responsabilidad |`.
   - **Verifica el propósito** de funciones nuevas o renombradas leyendo ±15 líneas alrededor;
     no inventes descripciones. Si una función desapareció, quítala; si cambió de nombre,
     renómbrala.
   - Marca en **negrita** las funciones que son el núcleo caliente (`paso`, `Simulator::run`,
     `network_json`, `schedule_json`, `optimize_system`, `bloqueEntrada`, `aspectoBloque`).
   - Mantén los enlaces `[[memoria]]` existentes (feedback-simulation-engine-design,
     game-web-engine-internals, godot-game-features, renfe-gtfs-source).
   - Actualiza el encabezado con el commit y la fecha nuevos.
   - Comentarios/prosa en catalán/castellano (convención del proyecto).

4. **No** ejecutes `cargo` ni el servidor: esta skill solo lee y regenera documentación.

5. Informa al usuario del diff (funciones añadidas/eliminadas/renombradas) en una frase.

## Invariante

El CODEMAP es un **índice derivado del código**, no una fuente de verdad: si hay discrepancia,
gana el código. Los nº de línea son orientativos (envejecen entre regeneraciones); por eso el
propio mapa recuerda "re-lee ±20 líneas antes de editar".
