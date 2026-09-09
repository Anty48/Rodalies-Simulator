# Rodalies-Simulator

Plataforma unificada de **simulación, optimización y visualización** de la red de **Rodalies
de Catalunya**. Reúne, en un único repositorio, dos proyectos antes separados:

- **Simulador y optimizador de horarios** (Rust, raíz del repositorio): construye la red
  (grafo de vías/cantones), la asignación de vías por estación y los trenes **100 %
  dinámicamente** a partir de archivos **GTFS** (feed de Fomento_Transit) en `./data/gtfs`.
  Incluye el panel de análisis web (dashboard + optimizador + calculador de tiempo mínimo).
- **Simulador de red / juego** (`game/`): un simulador 2D data-driven de la red. La versión
  actual es **web-nativa** (`game/web/`, HTML5 Canvas + JS puro) servida por el propio servidor
  en `/game` y alimentada por el GTFS y los horarios optimizados; el proyecto **Godot original**
  se conserva intacto en `game/godot-original/`. Véase `game/README.md`.

El objetivo analítico del simulador es la **resiliencia** del sistema: cómo se propagan los
retrasos por señalización y aglomeración de pasajeros, y cuánto tarda la red en volver al
equilibrio tras una incidencia.

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
  signaling.rs           # Sistema de señalización por cantones (verde/amarillo/rojo, cap. 1)
  simulation_engine.rs   # Motor de eventos discretos + señalización + incidencias
  scenario.rs            # Constructores de "vistas" parametrizados (consola y web)
  optimizer/potential.rs # Función de potencial V(H)
  optimizer/search.rs    # Recocido simulado + Monte Carlo paralelo (rayon)
  exporter.rs            # Exporta horarios optimizados a CSV + comparativa
  map.rs                 # Mapa SVG de la red (lat/lon) con trenes animados
  report.rs              # Renderiza el dashboard HTML (estático e interactivo)
  server.rs              # Servidor web local (tokio) para la UI interactiva
run.bat                  # Lanzador de doble clic (Windows): compila, ejecuta y abre la UI
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

La forma más fácil en Windows: **doble clic en `run.bat`**. Compila, ejecuta la simulación
y **abre un dashboard web interactivo** en el navegador.

Por línea de comandos:

```bash
cargo run --release                  # imprime resumen, escribe report/dashboard.html y arranca el servidor web
cargo run --release -- --static      # solo genera y abre el dashboard HTML (offline, sin servidor)
cargo run --release -- --no-open     # no abre el navegador automáticamente
cargo run --release -- optimize R1 R4  # OPTIMIZA los horarios de esas líneas (ver abajo)
```

### Motor de optimización de horarios (del SISTEMA)

`cargo run --release -- optimize` optimiza el **sistema entero** (todas las líneas a la
vez) para un **día laborable completo (05:00–00:00)**. El problema real no es una línea
aislada, sino los **conflictos entre líneas** en los cantones compartidos (el tronco
Sants–Passeig lo usan 8–9 líneas) y la **respuesta a las incidencias**: un buen juego de
horarios hace que, ante una incidencia, el sistema **tienda a la estabilidad** en vez de al
caos. Se optimiza el **desfase de fase de cada línea** (±5 min); cada candidato se evalúa
con **muchas simulaciones Monte Carlo del sistema completo, cada una con incidencias
aleatorias repartidas por el día**, en paralelo (`rayon`).

Resultado real (una ejecución, ~9 s): V **643 → 196 (−70 %)**, conflictos entre líneas
**81 → 12**, y **tiempo medio de recuperación tras incidencia 329 min → 100 min**. Genera
un **CSV y un PDF por línea** (`report/optimized/`) con el horario coordinado.

`cargo run --release -- optimize-line R1 R4` es un modo avanzado que optimiza líneas por
separado (con la misma función de potencial):

- **Física estricta** (`signaling.rs`): señalización por cantones con los tres aspectos
  (verde = velocidad nominal; amarillo = anden de destino ocupado → ralentiza; rojo =
  cantón ocupado → parada y retraso), **capacidad 1** por cantón y por andén.
- **Potencial** `V(H)` (`optimizer/potential.rs`): regularidad de frecuencias (con
  penalización **triple en hora punta** 07:00–09:30), retraso ponderado por pasajeros, y
  penalización severa por conflicto de vía.
- **Búsqueda** (`optimizer/search.rs`): **recocido simulado** que prueba variaciones de
  ±1..±5 min en la salida de origen; cada candidato se evalúa con **N simulaciones Monte
  Carlo en paralelo (`rayon`)** inyectando incidencias aleatorias en puntos críticos
  (Clot, Arc de Triomf…).
- **Exportación** (`exporter.rs`): escribe `report/optimized/R*_optimized.csv` **y un PDF por
  línea** (`R*_horari.pdf`, tabla estación×tren estilo horario oficial) y una comparativa en
  consola: **V base vs optimizado, recuperación y reducción de retraso por pasajero**.

Además (`topology.rs`, datos operativos derivados de fuentes públicas / del simulador de
referencia):

- **Vía única** (token/bastón piloto): tramos como R3 al norte de Montcada Bifurcació solo
  admiten un tren a la vez en cualquier sentido.
- **Vías reales por estación** (Sants 14, Clot 4… en vez de 1) → los andenes no son el
  cuello de botella; los cantones sí (capacidad 1).
- **Autobuses de sustitución por obras** (`route_type=3`, p.ej. 46 servicios en R3): se
  **distinguen** (marcador propio en el mapa) y se **excluyen** de la física ferroviaria y
  de la optimización.

### Optimización del sistema desde la web (en vivo)

El dashboard incluye un panel **Optimitzador del sistema**: pulsas *Optimitzar el sistema*
y el servidor lanza el recocido simulado del sistema completo en segundo plano; la web
muestra **en vivo cómo baja V(H)** a lo largo de las simulaciones (gráfico + contador de
iteraciones) y, al terminar, un aviso claro con el **valor final, el % de reducción, el pico
de retraso y el tiempo de recuperación**, más una tabla con el **desfase óptimo por línea**
y enlaces de descarga a cada **CSV** y **PDF**.

El PDF de cada línea es una tabla de horarios con **las estaciones en columnas y los trenes
(horarios) en filas**.

### UI interactiva (servidor web)

Por defecto arranca un pequeño **servidor web local** (tokio) en
`http://127.0.0.1:8080` con **controles interactivos**: franja horaria (hora de inicio y
duración), línea, duración del bloqueo de cantón, retraso inyectado, vías por andén,
separación mínima de bloque y pasajeros estocásticos. Al pulsar **Simular** vuelve a
correr la simulación en el servidor y actualiza el dashboard (gráfico SVG del retraso, log
CTC coloreado, métricas de estabilidad y tabla de resiliencia) sin recompilar. Ctrl+C para
parar el servidor.

El dashboard incluye además un **mapa de la red** (SVG) con las estaciones y cantones
proyectados desde lat/lon del GTFS y **trenes moviéndose** por sus rutas reales (animación
SMIL, sin JavaScript).

### Dashboard estático (offline)

`--static` (o cada ejecución, como copia) genera un **`report/dashboard.html`
autocontenido** —sin dependencias externas ni conexión— con los mismos paneles. Se puede
abrir con doble clic.

Salida por consola (resumida):

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
