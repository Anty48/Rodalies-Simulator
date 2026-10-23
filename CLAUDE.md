# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

> ## 🗺️ LEE ESTO PRIMERO — mapa de código
> Antes de explorar o editar una zona caliente, abre **[`docs/CODEMAP.md`](docs/CODEMAP.md)**:
> un índice **función-por-función con nº de línea** de los *hot paths* (juego web `app.js`,
> `game.rs`, `simulation_engine.rs`, `signaling.rs`, `topology.rs`, `optimizer/`). Salta directo
> a `fichero:línea` en vez de releer ficheros enteros (ahorra contexto). Los nº de línea
> envejecen → re-lee ±20 líneas antes de editar. Regénéralo con la skill **`/codemap`** cuando
> se desfase. Esta sección (arquitectura por módulos) queda como referencia de segundo nivel.

## Toolchain (Windows quirk)

Rust was installed via rustup but `cargo`/`rustc` are **not on the shell PATH by default**.
Prepend the cargo bin dir before every invocation (PowerShell):

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"; & cargo <cmd>
```

## Commands

- Build / typecheck: `cargo check` (must stay warning-free) · `cargo build --release`
- Run: `cargo run --release` prints a console summary, writes `report/dashboard.html`, and
  starts an interactive web server on http://127.0.0.1:8080 (blocks until Ctrl+C).
  `-- --static` = write+open the offline HTML and exit; `-- --no-open` = don't open a browser
  (use for CI/tests). `run.bat` is the double-click launcher.
- Optimize timetables: `cargo run --release -- optimize [R1 R4 …]` runs simulated annealing
  and writes `report/optimized/R*_optimized.csv` + a console comparison. No line args = top-4.
- Compare with Renfe: `cargo run --release -- compare [seed]` runs a day "before" (realism off)
  and "now" (saved realism) and compares per line with the GTFS and with REAL data from the
  GTFS-RT (`processed/gtfsrt/observed.json`) → console + `report/compare.json`.
- Tests: `cargo test` · single test: `cargo test mas_tiempo_desde_el_tren_anterior`
  (unit tests live in `#[cfg(test)]` modules inside each `src/*.rs`)
- Manual server check: run with `--no-open`, then `curl http://127.0.0.1:8080/api/render?block=12&delay=8`

## Data prerequisite

The binary reads `stops.txt`, `routes.txt`, `trips.txt`, `stop_times.txt` from `./data/gtfs`
and **exits if the folder or any file is missing**. `data/` and `fomento_transit/` are
gitignored (external data; the national feed and other raw sources live under `./raw/`).
To (re)generate `./data/gtfs`, place the national GTFS feed in `./raw/fomento_transit/` and
run `bash scripts/prep_gtfs.sh` — it filters the feed to Rodalies de Catalunya
(`route_short_name` starting with `R`).

Optional but recommended: `python scripts/fetch_osm_track.py` (re)builds
`processed/adif/track_geometry.json` — the REAL track polyline + length of every canton, routed on
the OSM rail graph with a turn-angle constraint (`src/track_geometry.rs`). Without it the engine
measures cantons in a straight line (e.g. França→Passeig de Gràcia 1,9 km instead of 4,9 km).
Re-run it after a GTFS update that adds new station pairs.

Realism data (all optional; the engine falls back to estimates if missing):
- `python scripts/fetch_renfe_demand.py` → `raw/renfe/barcelona_viajeros_por_franja.csv`: REAL
  boardings/alightings per station and 30-min band (Renfe 2018, stop_id = GTFS). Feeds `demand.rs`.
- Page `/datos` (menu «Datos reales de Renfe»): one button installs a Windows Task Scheduler task
  (`scripts/gtfsrt_autostart.ps1`, at logon + daily 04:30, pythonw, no admin) that runs
  `scripts/collect_gtfsrt.py` → Renfe GTFS-RT (positions, delays, cancellations, skipped stops,
  incident notices of Rodalies every ~20 s) into `raw/gtfsrt/*.jsonl` (~17 MB/day). The collector is
  single-instance with a control port 127.0.0.1:47231 (`ping`/`stop`). Manual: `collect_gtfsrt.bat`.
  **Cloud** (same page, `src/cloud.rs`): the collector 24/7 on **Cloudflare Workers FREE plan** (no card
  ⇒ cannot be billed). `scripts/cloudflare/` = a Durable Object (SQLite) with an alarm every 20 s, ONE
  poll per alarm (free plan = 10 ms CPU per invocation; the feed is chunked and only Rodalies
  entities are parsed, ~2 ms), gzip rows, 45-day retention (<1 GB DO storage), cron every 10 min re-arms
  the alarm; free SQLite = 5 M rows read/day — exceeding it blocks the DB (and recording) until 00:00 UTC,
  so `/status` reads per-day totals from the small `days` table, never a scan of `polls`; API with a Bearer token (`/status`, `/data?since=`, `/alerts`, `/incidents?since=`, `/start`, `/stop`). The
  simulator deploys it with `npx wrangler@4` (login via browser, `deploy`, `secret put TOKEN`) from a
  work copy in `report/cloudflare-worker/`; config/token in `report/cloudflare.json` (gitignored); HTTP via
  `curl`. Test locally without an account: copy `scripts/cloudflare` somewhere, `.dev.vars` with
  `TOKEN=…`, `npx wrangler@4 dev --test-scheduled`. Data syncs incrementally to `raw/gtfsrt/cloud/`;
  `analyze_gtfsrt.py` merges PC + cloud, `.jsonl` and `.jsonl.gz`, deduplicating polls by timestamp.
  **Only one collector records at a time:** the PC collector checks the cloud's `/status` every 2 min
  and stays on STANDBY while the cloud is enabled and alive (`alive_at` < 150 s); otherwise it records
  (automatic failover); deploy/start/stop from the simulator send `recheck` to its control port so the
  switch is immediate. The server auto-syncs the cloud data at startup and every 15 min
  (`start_cloud_autosync`), and `/datos` also syncs on open; `cloud::sync` is single-flight
  (Mutex) and only appends polls newer than the last line of each day file (no duplicates).
  `python scripts/analyze_gtfsrt.py` → `processed/gtfsrt/observed.json` (real segment/dwell
  times, delay distribution per line) used by `compare`; `--day D` → `observed_<D>.json` (cached while
  the day's files don't change), used by «Realidad por línea» (`POST /api/gtfsrt/analyze?day=D`).
- `/datos` → «Días grabados»: `scripts/gtfsrt_days.py` (PC + nube, dedupe por `t`, caché por fichero) →
  `processed/gtfsrt/days.json`: cobertura del horario 05–24 h (hora de Madrid sin tzdata), puntualidad,
  ≥15 min, cancelaciones, avisos y valoración normal/alterado/excepcional/incompleto (umbrales fijos +
  mediana/MAD de tus días con ≥5 completos). Ruta `/api/gtfsrt/days`.
- `/datos` order: PC collector + «¿Qué datos da Renfe?» → cloud → «Días grabados» → «Incidencias del día»
  → «Realidad por línea» (day selectors coloured by the day's verdict). The compare UI was removed for
  now (`/api/compare` and the CLI remain).
- Incidencias: `scripts/incidents.py` (`capture` / `list --day D`) → `raw/incidents/<día de publicación>.jsonl`,
  key = sha1("source|pub|text")[:16] (same in the Worker, so PC/cloud/button never duplicate). Sources:
  Gencat RSS `incidencies_rodalies_rss_ca_ES.xml` (publication time, lines, text; ONLY current ones →
  capture often: Worker every 15th alarm ≈5 min, PC collector every 5 min while it records, button
  `POST /api/incidents/capture`) + Renfe GTFS-RT alerts. `list` also merges the Renfe alerts recorded
  that day (`before_day` = carried over; Renfe leaves stale alerts, e.g. 6-7/10/2026, active for weeks).
  X/Twitter and WhatsApp are NOT readable for free (X API is paid; the public syndication endpoint
  returns random old tweets; WhatsApp channels have no API). `cloud::sync` pulls `/incidents` (`last_inc`).
- Día real con las herramientas del simulador (`/datos`): `scripts/gtfsrt_track.py --day D` reduce el GTFS-RT
  grabado a series por minuto (retraso vivo de la red y por línea, huecos sin datos), llegadas por tren y parada
  y avisos del día → `processed/gtfsrt/track_<D>.json` (caché). `src/realday.rs` lo convierte en una run con el
  MISMO formato que el motor (`track`+`lines`, `analysis` de `recovery.rs`) y calcula estadísticas comunes
  (puntualidad < 3 min, por línea y por hora, tramos donde se genera el retraso, tren a tren por nº de tren+línea).
  `POST /api/gtfsrt/track?day=D`; `GET /api/compare/day?day=D&seed=N&sel=…&vvar=…&frt=…&cad=…&free=1` simula con
  esa configuración y compara. El historial del visor guarda la consulta exacta (`q`, `cad_local`) para repetirla.
- Día real en el visor (pantalla previa → «Día real grabado»): `gtfsrt_track.py` escribe también
  `processed/gtfsrt/pos_<D>.json` (posiciones GPS); `src/realtrace.rs` las proyecta sobre el recorrido GTFS de cada
  tren (nº de tren + línea) y genera un trace con el formato del motor → `GET /api/game/realtrace?day=D`. Los
  semáforos son solo visuales; un tren parado sin tren delante se marca «causa externa».
- Prioridad de regionales (`RealismCfg` commuter_*/fast_*): cercanías esperan en cabecera a un regional rápido
  (`seguidores_rapidos`, `regional_rapido_llega`) y se apartan si les sigue uno frenado (`decidir_apartado_cercanias`).
- LTV: `/configuracion` → "Buscar y actualizar LTV" runs `scripts/fetch_adif_ltv.py` and reloads.

UI languages (es/ca/en): `game/web/i18n.js` translates the rendered DOM using
`game/web/i18n_dict.js` (Spanish source text → [ca, en], numbers as `{n}`). New UI text = add an
entry; find missing ones with `I18N.missing` in the browser console with English selected.

## Architecture

Discrete-event railway simulator whose entire network is built **dynamically from GTFS** at
runtime — there is no hardcoded map. Data flows in one direction: GTFS → `Network` → `Simulator`.

- **`gtfs_loader.rs`** — parses the four GTFS files into a `Network`:
  - `petgraph::DiGraph` where each stop is a node (`StopNode`) and each pair of consecutive
    stops in a trip is a directed edge (`TrackEdge` = a *cantón*/block) weighted with the
    nominal run time (`arrival_next − departure_current`). Edges are deduped, keeping the min run time.
  - `TrainService` per trip with an ordered `schedule`. `train_number` comes from
    `trip_short_name` **with fallback to `trip_id`** (the real Renfe feed lacks that column).
  - Real feed quirks handled here: fixed-width space padding (read with `csv::Trim::All`) and
    optional/missing columns (`parent_station`, `trip_short_name`) via `#[serde(default)]`.
  - Helpers used by the sim/reports: `dominant_service()`, `busiest_segment()`,
    `find_stop_by_name()`, `edge_between()`, `parse_gtfs_time()`/`fmt_hms()` (seconds-since-midnight).

- **`passenger_model.rs`** — dwell time. Key invariant: **extra dwell is 0 when arrival
  delay is 0** (a punctual train runs to timetable), so the "return to equilibrium" metric is
  meaningful. Extra passengers ∝ delay, and boarding time grows **exponentially** with delay
  (`e^(k·delay_min)`). `dwell_time` (deterministic) and `dwell_time_random` (Monte Carlo via `rand`).

- **`simulation_engine.rs`** — `Simulator::run(service_id)` drives a `BinaryHeap` priority
  queue of `Arrive`/`Depart` events over a time window, for trains of one `service_id`.
  - **Signalling**: effective block capacity = `max(1, run_secs / min_block_headway_secs)`
    (models multiple signal blocks per section, so the Barcelona trunk doesn't gridlock).
    A train that can't enter an occupied block or a full platform is re-queued every
    `RETRY_STEP` seconds, accumulating delay — this is how knock-on delay propagates.
  - **Incidents** (`Incident` enum): `TrainDelay` (by train number + station name),
    `BlockSegment` (by station name) and `BlockSegmentById` (by stop_id, exact).
  - **Stability metric**: samples network total/mean delay each minute (`Sample`), reports
    peak accumulated delay and the recovery time back below the equilibrium threshold.

- **`topology.rs`** — operational data GTFS lacks: `platform_tracks(name)` (real track counts,
  Sants 14…, default 2) and `single_track_pairs(net)` (undirected segments that are single-track;
  configured in `SINGLE_TRACK`, e.g. R3 north of Montcada Bifurcació). Derived from public/Godot
  reference data. Buses are `TrainService.is_bus` (GTFS `route_type==3`).

- **`signaling.rs`** — pure three-aspect block logic (`Aspect` Green/Yellow/Red, `Signals`).
  Green = nominal; Yellow (destination platform occupied) = run time × `yellow_slowdown`; Red
  (canton occupied) = the engine holds the train. The engine calls it when `strict_signaling`
  is on (capacity 1 per canton and platform).

- **`optimizer/`** — timetable optimization. `potential.rs`: `V(H)` components (regularity,
  passenger-weighted delay, conflict). **`system.rs` is the main mode**: `optimize_system`
  coordinates the WHOLE network (all lines) over a full weekday (05:00–00:00) by simulated
  annealing over a **per-line phase offset** (±5 min); each candidate is scored by the mean `V`
  over a FIXED set of Monte-Carlo scenarios (each with several incidents spread across the day),
  evaluated in parallel with rayon. `V = w_delay·passenger_weighted_delay + w_conflict·held`.
  Physics: block-model cantons + single-track token + real platform counts + buses excluded.
  `search.rs::optimize_line` is the legacy per-line mode (`optimize-line` CLI). Conflicts count
  distinct red stops (a `waiting` flag on `TrainRt`), not every 10s retry. Engine canton capacity
  is always the block model now; hard exclusion comes from the single-track token + platforms.

- **`cadence.rs`** — clock-face timetables from the `/horarios` creator (`game/web/cadencia.js`,
  saved in the browser as `rod-cadences`). `build_catalog` derives origin→destination patterns from
  GTFS (template trip + offset to a reference station: Sants, Cerdanyola U., La Garriga, Tarragona,
  Lleida); `generate` shifts those templates to the marked minutes (single markers + line GROUPS
  that rotate a sequence over the slots). `POST /api/cadence/load` → `cad=<id>` on
  schedule/trace/simulate/export; `Engine::with_schedule` runs it. `free=1` (visual only) =
  "sin horarios": only the terminus departure is kept (`EngineCfg.free_run`), with single-track
  chain reservation to avoid deadlocks (`cadena_unica`).

- **`exporter.rs`** — writes `report/optimized/<LINE>_optimized.csv`, a per-line **PDF**
  timetable (`<LINE>_horari.pdf`, printpdf; stations×trains grid, paginated), and the console
  comparison. Strict rail sim in the optimizer sets `strict_signaling` + `exclude_buses` +
  `single_track` (from `topology`) and per-station platform capacity from `topology`.

- **`map.rs`** — `network_map_svg` projects stations from lat/lon (equirectangular, cos-lat
  corrected), draws cantons, and animates a sample of trains along their real routes with SMIL
  `animateMotion` (keyTimes from the schedule) — no JS, works in the static file. Returned via
  `SimView.map_svg`.

- **`scenario.rs`** — the single source of truth for turning parameters (`SimParams`:
  window, line filter, block/delay minutes, platform capacity, min block headway, stochastic
  toggle) into `report::*View` structs, with **no console output**. Used by both `main` (console
  + static dashboard) and `server` (per-request). `res_view` runs the resilience sweep with
  **rayon**. Also holds `now_utc_string` (date without external crates).

- **`report.rs`** — renders the dashboard from `*View` structs. `render_body` = the sections
  (inline CSS + Rust-generated **SVG** chart; no external assets). `render_html` wraps it for
  the offline file; `render_interactive_page` adds the controls form + a small vanilla-JS
  `<script>` that `fetch`es `/api/render` and swaps `#dashboard`. CSS/JS live in `const`s so
  `format!` never has to brace-escape them. `SimView` reuses the engine's `CtcEvent`. If you
  add a field to a `*View`, render it (otherwise dead-code warning).

- **`server.rs`** — minimal tokio HTTP/1.1 server (GET only, `Connection: close`, localhost).
  Routes: `/`, `/api/render?…`, `/health`, plus the optimizer: `/api/optimize/start` (spawns a
  std::thread running `optimize_system`, progress written to `Arc<Mutex<OptJob>>`),
  `/api/optimize/status` (live JSON: iter/total, base_v, best_v, history, per-line `files`…), and
  `/report/optimized/<file>` (serves generated CSV/PDF; response body is `Vec<u8>` for binaries).
  The web `OPT_SCRIPT` polls status, draws the descending V(H) curve on a canvas, and on finish
  shows the metrics + a per-line download table.

- **`realism.rs`** — `RealismCfg`: EVERY tunable realism parameter with its reasonable default
  (`report/realism.json`, page `/configuracion`, `#[serde(default)]` so new fields never break a
  saved file) + `assumptions()` (page `/supuestos`). Rule: if you simplify physics, declare it there.
- **`demand.rs`** — real Renfe demand as CUMULATIVE curves per station (`cum_at`); loaded once
  (`demand::global()`).
- **`engine.rs` realism layer** (`Engine::with_schedule_ext(…, &RealismCfg, &[FreightTrain])`):
  passenger dwell (`intercambio_viajeros`: pools per station·line·direction, more headway ⇒ more
  people ⇒ longer dwell, capacity/left-behind, mass), rotation inferred from the timetable
  (`asignar_rotaciones`, `unidad_lista`: 3 min cab change, late in ⇒ late out, reserve unit),
  terminal departure crossing (`cross_id`, `salida_cruce_libre`), ASFA caution aspect
  (`actualizar_asfa`), service braking + anticipating speed zones (`limites_por_delante`),
  driver reaction (`reaccion_pendiente`), Davis resistance, train length (tail keeps the previous
  block), track speed gaps filled (`fill_zone_gaps`), tunnel margin. Freight: no priority
  (`claim_eta` at junctions, single-track priority), pulls into sidings for the passenger train
  behind (`decidir_apartado`/`pasajero_detras`), excluded from passenger metrics.
- **`freight.rs`** — freight scenarios (`report/freight.json`, page `/mercancias`): corridors
  generated per seed (`plan`) or fixed lists; routed on the atomic GTFS graph
  (`game::AtomicRouter`), so they share cantons/signals with Rodalies. Run param `frt=<id>`.
- **`compare.rs`** — the `compare` command (before/now/real per line, too-fast segments).
- **`main.rs`** — `#[tokio::main]`. Checks `./data/gtfs`, loads (timed in ms), builds the
  default `*View`s via `scenario`, prints them, writes the static dashboard, then either
  exits (`--static`) or serves the interactive UI. This is why `rayon`, `rand` and `tokio`
  are all real dependencies.

## Conventions

- Console output and code comments are in Catalan/Spanish; keep that voice.
- Keep `cargo check` clean: the model structs' fields are deliberately surfaced in reports so
  they aren't dead code — if you add a field, actually use it (in a report or the engine)
  rather than silencing the warning.

## Reglas de código (rendimiento primero, lógica compleja bien estructurada)

Norte del proyecto: un simulador de Rodalies **hiperrealista**. Ante cualquier disyuntiva, elige
lo físicamente/operativamente correcto; si simplificas un modelo y eso sacrifica realismo,
decláralo en vez de asumirlo.

**Rendimiento computacional (es un motor de simulación, no una app CRUD):**
- El coste está en los bucles calientes: el motor de eventos (`simulation_engine`), el barrido de
  resiliencia y el optimizador (Monte-Carlo), y el bucle de simulación del juego web (`paso()`),
  que corre por fotograma sobre ~960 trenes. Piensa siempre en la complejidad de lo que metas ahí.
- Precalcula fuera del bucle: índices/mapas una vez al cargar (p. ej. `edgeLines`, `subBlocks`,
  `singleTrackUsers`, la secuencia de bloques por tren), no recalcules por fotograma ni por tren.
- Prefiere estructuras con la complejidad adecuada (`HashMap`/`Set`/`BinaryHeap`) a búsquedas
  lineales repetidas; evita clonar en caliente (usa referencias/índices) y asignaciones por frame.
- Paraleliza el trabajo independiente con `rayon` (ya se hace en escenarios/optimizador); mantén
  el trabajo por fotograma en el cliente O(trenes)+O(señales visibles), no O(trenes×señales).
- Regla del bucle a velocidad variable: **nada de ventanas de tiempo fijas**; usa sub-pasos
  acotados y comprobaciones por umbral, y verifica SIEMPRE a ×25 (donde afloran los bugs de salto).
- Mide antes de optimizar (los tiempos de carga ya se cronometran en ms); no compliques sin dato.

**Estructuración de lógica compleja:**
- Una responsabilidad por módulo (respeta la arquitectura GTFS→`Network`→`Simulator`; el flujo de
  datos va en una sola dirección). Nueva lógica de física va en el motor, no en la capa de reporte.
- Funciones puras y con nombre para cada regla del dominio (atomicidad de cantones por geometría,
  ocupación de bloque, aspecto de señal, parada mínima…), testeables de forma aislada; nada de
  condiciones mágicas embebidas: extrae la regla a una función con un comentario del *porqué*.
- Comenta el **motivo** (la física/operación real), no el *qué*; mantén los invariantes explícitos
  (p. ej. «dwell extra = 0 con retraso 0», «1 tren por bloque», «vía única = testigo compartido»).
- Deriva de datos reales (GTFS, ADIF, topología) antes que constantes inventadas; si necesitas una
  constante operativa, ponla nombrada y con su fuente.
- Idioma: consola y comentarios en catalán/castellano (convención existente); código y APIs en
  inglés donde ya lo estén. `cargo check` sin warnings.
