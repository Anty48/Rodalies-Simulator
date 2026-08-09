# ROADMAP · rodalies-sim

## Estado actual

Motor de **optimización y estudio de horarios** para Rodalies, construido dinámicamente
desde GTFS. Resumen de lo implementado:

- **Carga GTFS dinámica** → grafo `petgraph` (estaciones = nodos, cantones = aristas),
  servicios con nº de circulación de Renfe, lat/lon, y distinción de **autobuses de
  sustitución por obras** (`route_type=3`; se excluyen de la física ferroviaria).
- **Física de señalización** (`signaling.rs`): tres aspectos verde/amarillo/rojo; capacidad
  de cantón por **modelo de bloques**; **vía única** con testigo (R3 al N de Montcada);
  **vías reales por estación** (Sants 14, Clot 4…).
- **Función de potencial** `V(H)`: regularidad de frecuencias (×3 en punta), retraso
  ponderado por pasajeros y penalización por conflicto entre líneas.
- **Optimización del SISTEMA entero** (`optimizer/system.rs`): recocido simulado sobre el
  **desfase de fase por línea** (±5 min) para un **día laborable 05:00–00:00**, evaluado con
  **muchas simulaciones Monte Carlo del sistema completo** con incidencias aleatorias
  (rayon). Resultado típico: V −70 %, conflictos 81→12, **recuperación media 329→100 min**.
- **Exportación**: CSV + **PDF por línea** (estaciones en columnas, trenes/horarios en
  filas).
- **Dashboard web** (tokio): controles interactivos, **mapa de la red** (lat/lon) con trenes
  animados (buses diferenciados), y panel **"Optimitzar el sistema"** con **progreso en vivo**
  de V(H) y descarga de resultados.

## Siguiente paso natural

**Optimización en dos niveles (per-línea + ajuste fino por tren).**

Hoy la decisión es un único desfase de fase *por línea*. El siguiente paso es añadir un
segundo nivel de **ajuste fino por tren** (±1–2 min por circulación individual, encima del
desfase de su línea) para suavizar los intervalos (headways) y exprimir más resiliencia:

1. Fase gruesa: optimizar el desfase por línea (ya hecho) para cuadrar líneas entre sí.
2. Fase fina: para cada línea, optimizar pequeños offsets por tren manteniendo fija la fase
   de línea, minimizando el mismo `V(H)` del sistema.

## Ideas secundarias

- **Continuidad de material rodante**: usar `block_id` del GTFS para que un tren que llega
  tarde propague el retraso a su siguiente servicio (turnaround), capturando un mecanismo
  real de propagación de incidencias.
- Subir iteraciones / escenarios Monte Carlo para afinar la recuperación.
- PDF: cabecera con nombres de estación completos **rotados 90°**.
- Ampliar los tramos de **vía única** en `topology.rs` (R2 Sant Vicenç, ramal de R4…).
