# CODEMAP — mapa de código de los *hot paths*

> **Qué es esto.** Índice función-por-función de las zonas donde más se itera (motor del juego
> web + motor de simulación Rust), para saltar directo al sitio exacto sin releer ficheros
> enteros. `CLAUDE.md` cubre la arquitectura a nivel de módulo; **esto baja a nivel de función
> con nº de línea**.
>
> **Alcance:** solo *hot paths* (juego web + simulación). NO cubre calculator/, report.rs,
> exporter.rs, gtfs_loader.rs, main.rs, scenario.rs, map.rs — esos están en `CLAUDE.md`.
>
> **Los nº de línea envejecen.** Son un punto de partida, no una verdad absoluta: tras editar,
> re-lee ±20 líneas alrededor. Para regenerar el mapa cuando se desincronice: **skill `/codemap`**.
>
> - Generado contra commit: **`e571597`** · fecha: **2026-09-14**
> - Convención: `fichero:línea` es clicable.

---

## 1. Juego web — `game/web/app.js` (1310 líneas) · EL HOT PATH PRINCIPAL

Réplica en JS de las mecánicas del simulador Godot sobre mapa satélite (Leaflet). Corre `paso()`
por fotograma sobre ~960 trenes. **Regla de oro (ver memoria [[feedback-simulation-engine-design]]):**
nada de ventanas de tiempo fijas; trabajo por fotograma O(trenes)+O(señales visibles); precálculo
de índices al cargar (`edgeLines`, `subBlocks`, `singleTrackUsers`, secuencia de bloques por tren).

### Estado y constantes (leer primero para orientarse)
| Símbolo | Línea | Qué es |
|---|---|---|
| `DIA_INICIO/DIA_FIN/FACTOR_BASE`, `VELOCIDADES`, `PERFIL`, `NOMINAL_PAX` | `app.js:16` | Constantes de `Global.gd` (jornada 05:00–24:00, factores de velocidad ×0.05…×25, perfil de demanda horaria, pax nominales del KPI) |
| `SIGNAL_SPACING_KM`, `MARGIN_KM` | `app.js:34` | Densidad de señales de tramo (1 cada 1.6 km, tope duro; sin densidad adaptativa) |
| `S` (objeto de estado global) | `app.js:75` | **Todo el estado runtime**: `net`, `trains`, `stationById`, `edgeLines`, `occ` (1 tren/bloque), `subBlocks`, `interlocks`, `singleTrack`, `singleTrackUsers`, `units`, `incidentes`, `cantonMode`, `kpi`, `simT`… |
| `CORREDORES` | `app.js:98` | Grupos de vías independientes por estación (Sants, El Clot, Castellbisbal, Cerdanyola U.) — el GTFS no lo trae |
| `$` / `map` | `app.js:112` / `115` | Helper `getElementById` / instancia Leaflet |

### Carga y precálculo (una vez, al arrancar)
| Función | Línea | Responsabilidad |
|---|---|---|
| `distKm(a,b)` | `app.js:37` | Distancia Haversine entre coords |
| `calcSubBlocks(a,b,unica)` | `app.js:48` | Nº de sub-bloques de un cantón por distancia (tope de densidad) |
| `cargarRed()` | `app.js:145` | `async` — fetch `/api/game/network`; construye `stationById`, `edgeLines`, `subBlocks`, `shearMap`, `singleTrack`, bounds del mapa |
| `dibujarLineas()` | `app.js:214` | Traza las líneas por cantones reales (corrige trazados espurios); vía doble = 2 trazos con offset perpendicular |
| `trazarVia`/`perpOffset` | `app.js:205` / `199` | Polilínea de un cantón / desplazamiento perpendicular por vía |
| `cargarHorarios()` | `app.js:325` | `async` — fetch `/api/game/schedule`; construye `S.trains` con `pts` (paradas), tiempos por arista |
| `encadenarFlota()` | `app.js:247` | Asigna unidades físicas a servicios encadenados por línea (reutilización de material) |
| `dispersarFlota()` | `app.js:307` | Reparte material inicial en slots/apartaderos (evita solapes al inicio del día) |
| `construirSecuenciaBloques()` | `app.js:476` | **Precálculo caliente**: secuencia de bloques que recorre cada tren (para no recalcular por frame) |
| `imponerParadaMinima()` | `app.js:452` | Fuerza dwell mínimo reglamentario en paradas (invariante: dwell extra = 0 sin retraso) |

### Enclavamiento de estación (vías, corredores, vía principal)
| Función | Línea | Responsabilidad |
|---|---|---|
| `corredoresDe`/`tieneCorredores`/`corredorDe` | `app.js:105`–`111` | Resolución de corredores de una estación |
| `interlock(id)` | `app.js:369` | Construye/cachea el enclavamiento de una estación (vías por corredor, verde por lado, ocupación) |
| `corrIdx`/`viaPrincipal`/`internoRojo` | `app.js:393`–`398` | Índice de corredor de una línea / vía principal por lado / semáforo interno en rojo |
| `sidingTracksDe`/`sidingsDe` | `app.js:401`/`407` | Vías de apartadero disponibles en una estación |
| `platCap(b)` | `app.js:492` | Capacidad de andén = `max(1, tracks/2)` |

### Ocupación de bloques y señalización (núcleo físico)
| Función | Línea | Responsabilidad |
|---|---|---|
| `subM(a,b)` / `sbRes(a,b,sb)` | `app.js:441`/`442` | Nº de sub-bloques / clave de recurso de un bloque (vía única = clave canónica compartida) |
| `resKey`/`key`/`canon` | `app.js:139`/`135`/`136` | Claves de recurso (dirigida vs canónica para vía única) |
| `bloqueOcupado(a,b,sb,except)` | `app.js:497` | ¿Hay otro tren en ese bloque? |
| `edgeOcupadoRes(c,d,except)` | `app.js:512` | ¿Ocupado el recurso arista en cualquier sub-bloque? |
| `ocuparBloque(t,b,add)` | `app.js:522` | Marca/libera ocupación de un bloque por un tren |
| `bloqueEntrada(t,nb)` | `app.js:533` | **Decisión de avance**: ¿puede el tren entrar al siguiente bloque? (devuelve `{ok,motivo,tEnter}`) |
| `aspectoBloque(a,b,sb,m,line)` | `app.js:547` | Aspecto de la señal: `red`/`amber`/`green` |
| `siguienteRojo(a,b,sb,m,line)` | `app.js:554` | ¿La señal siguiente está en rojo? (para ámbar) |
| `salidaOcupada`/`salidaBloqueada` | `app.js:560`/`570` | Bloqueo de salida de estación |
| `regionalEntrandoPronto(rk,except)` | `app.js:429` | Prioridad en vía única: ¿entra pronto un regional? |
| `incidenteEdge`/`estacionParada`/`edgeLento` | `app.js:423`/`582`/`583` | Consulta de incidencias activas por arista/estación |

### Bucle de simulación y render (por fotograma — CUIDADO con la complejidad)
| Función | Línea | Responsabilidad |
|---|---|---|
| **`paso(dtGame)`** | `app.js:633` | **EL bucle caliente.** Avanza `simT`, mueve cada tren por su secuencia de bloques, aplica bloqueos/retenciones, acumula KPI de retraso. Verificar SIEMPRE a ×25. |
| `render()` | `app.js:703` | Repinta markers de tren (solo posición/bearing visibles) |
| `segmentoDe`/`posDe`/`bearingDe`/`bearing` | `app.js:586`–`616` | Interpolación de posición y rumbo del tren en su arista |
| `bucle(now)` | `app.js:1172` | `requestAnimationFrame` — orquesta `paso`+`render`+refrescos de Uic |
| `posEnArista`/`durmientesEn`/`trenesEsperandoEn` | `app.js:768`/`750`/`776` | Posición fraccional / unidades apartadas / trenes en espera en un tramo |

### Señales visibles (markers) — solo al mover/zoom, NO por frame
| Función | Línea | Responsabilidad |
|---|---|---|
| `crearSemaforo(...)` | `app.js:789` | Crea un marker de señal con tooltip + clic → panel |
| `construirSemaforos()` | `app.js:813` | Reconstruye markers visibles (vía doble = fronteras internas por sentido; vía única = 2 señales de salida). Ligado a `map.on("moveend zoomend")` |
| `actualizarSemaforosAspecto()` | `app.js:849` | Repinta SOLO el color de las señales visibles — barato, por frame |

### Paneles / UI / incidencias / GTFS
| Función | Línea | Responsabilidad |
|---|---|---|
| `abrirSemaforo`/`buildSignalPanel`/`actualizarSemaforoDinamico` | `app.js:870`/`872`/`886` | Panel del semáforo de tramo (modos AUTO/Manual verde/rojo) |
| `cerrarDocks`/`abrirEstacion`/`buildStationPanel`/`actualizarEstacionDinamico` | `app.js:905`/`910`/`914`/`963` | Panel de estación (docks exclusivos) |
| `iniciarActualizacionGtfs`/`subirGtfs`/`pollGtfsStatus` | `app.js:1026`/`1032`/`1041` | Botón "Actualizar GTFS" (ver [[renfe-gtfs-source]]) |
| `poblarSelectores`/`rellenarDonde`/`programarIncidencia`/`activarIncidencias`/`refrescarIncidencias` | `app.js:1071`–`1103` | Generador de incidencias (tipo/dónde/cuándo) |
| `refrescarTrenes`/`irAlTren` | `app.js:1111`/`1130` | Panel de trenes (filtro/orden/ir a) |
| `finDelDia`/`actualizarKpi`/`pintarLeyenda`/`setSpeed` | `app.js:1137`–`1152` | Fin de jornada / KPI / leyenda / control de velocidad |
| `iniciar()` | `app.js:1187` | `async` — **entrypoint**: carga red+horarios, precálculos, arranca el bucle. Contiene closures `aplicarAislado`/`pintarRegistro`/`pintarSpark`/`refrescarAsa` |

---

## 2. Backend del juego — `src/game.rs` (629 líneas)

Sirve el JSON que consume `app.js`. Construye el grafo atómico de cantones **por geometría**
(no por alcanzabilidad de grafo — ver [[feedback-simulation-engine-design]]).

| Símbolo | Línea | Responsabilidad |
|---|---|---|
| `trenscat_diagrams()` | `game.rs:52` | Diagramas de línea de referencia (trenscat) para ordenar/validar trazados |
| `AtomicGraph` (`is_atomic`/`decompose`) | `game.rs:140`/`141`/`156` | Cantón atómico = no descomponible en sub-tramos por geometría |
| `haversine_m(a,b)` | `game.rs:197` | Distancia en metros entre coords |
| `build_atomic_graph(net)` | `game.rs:216` | Construye el grafo de cantones atómicos desde la red GTFS |
| `fallback_color(line)` | `game.rs:287` | Color por defecto si la línea no trae color oficial |
| **`network_json(net,lines,generated_at)`** | `game.rs:295` | Sirve `/api/game/network`: estaciones, líneas, cantones, vía única, corredores, shears |
| `optimized_offsets(dir)` | `game.rs:517` | Lee offsets de horarios optimizados desde `report/optimized/` |
| **`schedule_json(net,optimized,line_filter)`** | `game.rs:543` | Sirve `/api/game/schedule`: horarios del día (GTFS u optimizados) |

> Detalle técnico ampliado en memoria [[game-web-engine-internals]] e inventario Godot en
> [[godot-game-features]].

---

## 3. Motor de simulación (batch) — `src/simulation_engine.rs` (748 líneas)

Motor de eventos discretos (`BinaryHeap` de `Arrive`/`Depart`) para trenes de un `service_id`.
Es el motor del modo *reporte/optimizador* (distinto del bucle por-frame del juego, que vive en JS).

| Símbolo | Línea | Responsabilidad |
|---|---|---|
| `SimConfig` (+ `Default`) | `simulation_engine.rs:28`/`62` | Config: ventana, capacidad, headway mínimo, `strict_signaling`, `exclude_buses`, `single_track`… |
| `Incident` (enum) | `simulation_engine.rs:90` | `TrainDelay` / `BlockSegment` / `BlockSegmentById` |
| `Event` (+ `Ord`/`PartialOrd`) | `simulation_engine.rs:127`/`135` | Elemento de la cola de prioridad (orden temporal invertido para min-heap) |
| `Sample` / `CtcEvent` / `SimResult` | `simulation_engine.rs:169`/`181`/`192` | Muestra de estabilidad / evento CTC / resultado (pico de retraso, tiempo de recuperación) |
| `Simulator<'a>` | `simulation_engine.rs:214` | El motor (referencia a `Network` + `SimConfig`) |
| `Simulator::new` | `simulation_engine.rs:221` | Constructor |
| `Simulator::add_incident` | `simulation_engine.rs:229` | Registra una incidencia |
| `Simulator::is_key_station` | `simulation_engine.rs:233` | ¿Es estación clave (para métricas)? |
| **`Simulator::run(service_id)`** | `simulation_engine.rs:243` | **EL bucle de eventos** (líneas 243–738). Fases internas: selección de trenes (`:247`), estado de ocupación (`:313`), métrica de estabilidad (`:374`), **bucle de eventos** (`:405`), señalización de andén (`:426`), señalización de cantón/aspecto rojo (`:614`), análisis de estabilidad final (`:701`) |
| `key_station_names(net,patterns)` | `simulation_engine.rs:739` | Resuelve nombres de estaciones clave desde patrones |

---

## 4. Señalización pura — `src/signaling.rs` (88 líneas)

Lógica de tres aspectos, sin estado — función pura testeable en aislado.

| Símbolo | Línea | Responsabilidad |
|---|---|---|
| `Aspect` (enum Green/Yellow/Red) | `signaling.rs:20` | Aspecto de señal |
| `Signals` (+ `Default`) | `signaling.rs:31`/`37` | Config (yellow_slowdown) |
| `Signals::aspect(canton_free, platform_ahead_free)` | `signaling.rs:48` | Regla de aspecto: verde/ámbar (andén ocupado)/rojo (cantón ocupado) |
| `Signals::traversal_time(nominal, aspect)` | `signaling.rs:61` | Tiempo de recorrido según aspecto (ámbar = ×yellow_slowdown) |
| tests `aspect_priority`/`yellow_slows_down` | `signaling.rs:75`/`83` | Unit tests de la lógica |

---

## 5. Topología operativa — `src/topology.rs` (337 líneas)

Datos que el GTFS no trae (vías por andén, vía única, corredores, líneas regionales).

| Símbolo | Línea | Responsabilidad |
|---|---|---|
| `platform_tracks(name)` | `topology.rs:20` | Nº real de vías de andén (Sants 14…, default 2) |
| `PLATFORM_TRACKS` (const) | `topology.rs:29` | Tabla de vías por estación (fuente: datos públicos/Godot) |
| `SINGLE_TRACK` / `SINGLE_TRACK_ZONE_IDS` | `topology.rs:155`/`177` | Tramos de vía única (por par de nombres / por zona de stop_ids) |
| `RODALIES_METROPOLITANAS` (const) | `topology.rs:230` | Líneas metropolitanas vs regionales |
| `is_regional_line(line)` | `topology.rs:233` | ¿Es línea regional? |
| `single_track_pairs(net)` | `topology.rs:240` | Set de aristas (NodeIndex,NodeIndex) de vía única |
| `node_by_name(net,sub)` | `topology.rs:278` | Busca NodeIndex por subcadena de nombre |
| `shear_conflicts(net)` | `topology.rs:285` | Conflictos de cizallamiento (cruces a nivel) |
| `Corridor` / `corridors(name)` | `topology.rs:302`/`312` | Corredores de vías independientes por estación |
| `unordered(a,b)` | `topology.rs:331` | Par de nodos canónico (para claves de vía única) |

---

## 6. Optimizador — `src/optimizer/` (746 líneas)

Simulated annealing sobre offsets de fase por línea; scoring por Monte-Carlo en paralelo (rayon).

| Símbolo | Línea | Responsabilidad |
|---|---|---|
| `optimize_lines(...)` | `optimizer/mod.rs:15` | Dispatcher del CLI `optimize` |
| `PotentialWeights` (+ `Default`, `is_peak`, `factor`) | `optimizer/potential.rs:13`–`44` | Pesos de V(H) + ventana punta |
| `regularity_cost(times,w)` | `optimizer/potential.rs:56` | Componente de regularidad (headways) |
| `passenger_weighted_delay(sim,w)` | `optimizer/potential.rs:82` | Retraso ponderado por pasajeros |
| `potential(times,sim,w)` | `optimizer/potential.rs:93` | V(H) total |
| `SearchConfig`/`LineOptResult` | `optimizer/search.rs:23`/`48` | Config y resultado del modo legacy por línea |
| `optimize_line`/`optimize_line_cb` | `optimizer/search.rs:72`/`84` | Modo legacy `optimize-line` (con callback de progreso) |
| `run_once`/`build_mc_incidents` | `optimizer/search.rs:217`/`244` | Una evaluación / incidencias Monte-Carlo |
| `SystemSearch`/`SystemResult` | `optimizer/system.rs:27`/`56` | **Modo principal** (toda la red, jornada completa) |
| **`optimize_system(...)`** | `optimizer/system.rs:79` | Coordina la optimización global por annealing sobre offsets de fase por línea |
| `run_system`/`build_scenarios` | `optimizer/system.rs:211`/`238` | Evaluación de un candidato / set fijo de escenarios MC |

---

## Cómo usar este mapa (para Claude en sesiones futuras)

1. Para una tarea del **juego**, empieza por `S` (`app.js:75`) y `paso()` (`app.js:633`); de ahí
   salta a la función concreta por la tabla.
2. Para **física de simulación batch**, `Simulator::run` (`simulation_engine.rs:243`) + `signaling.rs`.
3. Lee ±20 líneas alrededor del ancla antes de editar (los nº de línea derivan).
4. Si el mapa está claramente desfasado, regenéralo con la skill **`/codemap`**.
