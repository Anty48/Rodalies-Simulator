# rodalies-sim

Simulador de **tráfico ferroviario e incidencias** para **Rodalies de Barcelona**, escrito
en Rust. Construye la red (grafo de vías/cantones), la asignación de vías por estación y
los trenes **100 % dinámicamente** a partir de archivos **GTFS** históricos ubicados en
`./data/gtfs`.

Su objetivo es analizar la **resiliencia** del sistema: cómo se propagan los retrasos por
señalización y aglomeración de pasajeros, y cuánto tarda la red en volver al equilibrio
tras una incidencia.

---

## ¿Qué modela?

- **Grafo de infraestructura** (`petgraph`): cada parada del GTFS es un nodo; cada par de
  paradas consecutivas de un `trip` es una arista dirigida (**cantón/sección**) con su
  tiempo de marcha nominal (`arrival_next − departure_current`) y su capacidad.
- **Servicios de tren** (`TrainService`): `trip_id`, número de circulación de Renfe
  (`trip_short_name`, con *fallback* a `trip_id`), línea (`R1`, `R2N`, `R4`…) y horario
  ordenado de paradas en segundos desde medianoche.
- **Dwell time dinámico** (`passenger_model.rs`): el tiempo de parada crece de forma
  **exponencial** con el retraso de llegada, porque a más retraso más pasajeros
  acumulados en el andén:

  ```text
  Dwell Real = Dwell Teórico + Δt_pasajeros
  pasajeros_extra = tasa_base · retraso_min
  Δt_pasajeros    = t_pax · (pasajeros_extra / coches) · e^(k · retraso_min)
  ```

  Con retraso 0 el Δt es 0 (tren puntual = conforme a horario), de modo que la métrica de
  *retorno al equilibrio* tiene sentido.
- **Motor de eventos discretos** (`simulation_engine.rs`): cola de prioridades por tiempo.
  - **Señalización**: un cantón admite `max(1, marcha / separación_mínima)` trenes
    (varios bloques por sección → el tronco central de Barcelona absorbe más tráfico).
    Si no queda bloque libre, el tren se retiene acumulando retraso segundo a segundo.
  - **Andenes**: capacidad configurable de vías por estación, con asignación dinámica de
    vía (aparece en el log tipo CTC).
  - **Incidencias**: retraso puntual a un tren en una estación, o bloqueo de un cantón
    durante una ventana temporal.
  - **Métrica de estabilidad**: retraso acumulado global de la red muestreado en el tiempo
    y detección del instante de retorno al equilibrio.
- **Análisis de resiliencia en paralelo** (`rayon` + `rand`): barrido Monte Carlo de
  escenarios (p. ej. duración creciente del bloqueo) ejecutados simultáneamente.

---

## Estructura

```
src/
  main.rs                # Punto de entrada: carga, resumen, simulación CTC, resiliencia
  gtfs_loader.rs         # Parser GTFS -> grafo petgraph + servicios (carga dinámica)
  passenger_model.rs     # Dwell time en función de pasajeros y retraso
  simulation_engine.rs   # Motor de eventos discretos + señalización + incidencias
scripts/
  prep_gtfs.sh           # Filtra un feed GTFS nacional a Rodalies de Catalunya
data/gtfs/               # GTFS de Rodalies que lee el simulador (generado, no versionado)
raw/                     # Feed GTFS nacional y fuentes crudas de terceros (no versionado)
```

## Dependencias

`serde` + `csv` (parseo GTFS), `petgraph` (grafo), `rayon` (paralelismo),
`tokio` (runtime asíncrono del `main`), `rand` (incidencias/pasajeros probabilísticos).

---

## Datos GTFS (`./data/gtfs`)

El simulador lee `stops.txt`, `routes.txt`, `trips.txt` y `stop_times.txt` de
`./data/gtfs`. Estos archivos **no se versionan** (son datos externos). Para generarlos a
partir del feed GTFS público de Cercanías/Rodalies (p. ej. el del Ministerio de
Transportes), coloca el feed nacional en `./raw/fomento_transit/` y ejecuta:

```bash
bash scripts/prep_gtfs.sh
```

El script filtra el feed a **Rodalies de Catalunya** (líneas cuyo `route_short_name`
empieza por `R`) y escribe los cuatro archivos en `./data/gtfs`.

> Notas sobre el feed real de Renfe: los campos vienen rellenados con espacios (ancho
> fijo) — se leen con `Trim::All`. No trae `trip_short_name` ni `parent_station`; el
> `train_number` cae a `trip_id` y `parent_station` queda como `None`.

---

## Ejecución

```bash
cargo run --release
```

Salida (resumida):

1. Comprobación de `./data/gtfs` y **tiempo de carga en ms**.
2. **Resumen de la red**: nodos (vías/andenes), cantones, servicios, líneas y ejemplos de
   número de circulación de Renfe.
3. **Ruta detallada** de un tren de ejemplo (el `25412` si existe; si no, uno equivalente)
   con paradas, tiempos de marcha y vía asignada.
4. **Simulación CTC** de 2 h (07:00–09:00) con un log de entradas/salidas en tramos clave
   (Sants, Passeig de Gràcia, Clot, Pl. Catalunya, Estació de França, Arc de Triomf),
   incidencias inyectadas y métricas de estabilidad.
5. **Tabla de resiliencia** (barrido paralelo): pico de retraso acumulado, trenes
   afectados y tiempo de recuperación según la severidad de la incidencia.

## Tests

```bash
cargo test
```

## Requisitos

Rust estable (edición 2021). Probado con `cargo 1.97`.

---

Datos: GTFS de Cercanías/Rodalies del Ministerio de Transportes (dominio público).
Proyecto con fines de análisis y educativos.
