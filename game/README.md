# Juego · Simulador de red de Rodalies

Este directorio contiene el simulador/juego de la red, unificado dentro de **Rodalies-Simulator**
junto con el simulador y optimizador de horarios en Rust (raíz del repositorio).

## `web/` — simulador de red web-nativo (actual)

Reimplementación web-nativa (HTML5 Canvas + JavaScript puro, sin frameworks) servida por el
propio servidor Rust en `/game`. Se alimenta dinámicamente del GTFS de Fomento_Transit a través
de la API:

- `GET /api/game/network` — topología: estaciones (con vías reales por estación y color oficial
  de línea tomado de `route_color` del GTFS) y la **secuencia real** de cada línea y sentido.
- `GET /api/game/schedule?source=gtfs|optimized[&line=Rn]` — horarios del día laborable dominante;
  `optimized` aplica los desfases por línea que produce el optimizador (`report/optimized/*.csv`).

Mecánicas: mapa geográfico data-driven, trenes circulando por su ruta real siguiendo el horario,
reloj de jornada 05:00–00:00 con control de velocidad, desplazamiento/zoom de cámara, información
por estación, e incidencias con KPI de retraso (minutos·pasajero).

Se lanza desde el panel de análisis (pestaña «Simulador de red (juego)») o directamente en `/game`.

## `godot-original/` — proyecto Godot original (copia intacta, referencia y respaldo)

Copia **intacta** del proyecto original en Godot 4 / GDScript (repositorio `rodalies-game`), que
motivó y sirve de referencia funcional a la versión web. Se conserva completo (escenas, scripts,
datos y documentación) como respaldo histórico. No forma parte de la compilación del crate Rust
ni del servidor; se abre con el editor de Godot. Su documentación propia está en
`godot-original/CLAUDE.md`.
