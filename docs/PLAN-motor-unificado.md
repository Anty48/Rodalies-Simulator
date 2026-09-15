# Plan técnico — Motor de simulación unificado (Rodalies)

> **Estado:** aprobado como rumbo (2026-09-14). Orden en §8. Trabajos previos en §7.
>
> ### PROGRESO
> - ✅ **Fase 1 (fundamentos) HECHA** (`src/engine.rs`, subcomando `cargo run --release -- simulate`):
>   `Engine` con `step`/`observe`/`run`/`metrics`, cinemática por cantón con dwell y retraso,
>   determinista, tests. `cargo check` limpio.
> - ✅ **Fase 1b (señalización de BLOQUES) HECHA** (`src/engine.rs`, reescrito al **modelo de bloques
>   + `shift`** portado 1:1 de `game/web/app.js::paso()`). Verificado headless (`simulate`, jornada
>   961 trenes): **pico retenidos 6, final 0 (recupera), retraso máx ~767 s ≈ 12,8 min** — PARIDAD
>   con el run JS verificado en memoria (máx ~13 min, pico ~4, final 0). Incluye:
>   1. ✅ **Ocupación de cantón** (1 tren/bloque, andén = cola del último sub-bloque, platCap) +
>      retención sin rebote (`bloque_ocupado`/`ocupar`/`bloque_entrada`, Pass 1 + Pass 2).
>   2. ✅ **Aspecto de señal** rojo/ámbar/verde (`aspecto_bloque`/`siguiente_rojo`/`salida_ocupada`),
>      expuesto en `observe()` para el visor (informativo; el ROJO ya detiene vía el gating).
>      *(No se absorbió `signaling.rs`: usa `Signals::traversal_time` que aquí no aplica; queda para
>      la Fase B, cuando el ámbar ralentice de verdad.)*
>   3. ✅ **Vía única** (testigo canónico compartido + liberación por apartadero `te >= llegada` +
>      prioridad regional `regional_entrando_pronto`).
>   5. ✅ **Atascos**: emergen solos (un tren clavado en rojo retiene a los de detrás).
>   **CLAVE de unificación:** el motor consume el MISMO horario descompuesto por cantones atómicos
>   que la API del juego, vía el nuevo `game::build_schedule()` (extraído de `schedule_json`) — así
>   exprés y trenes de parada comparten cantón físico real. Sub-bloques/vía única/terminales/vías de
>   andén se derivan de `net`+`topology` (portados `calc_sub_blocks`, `imponer_parada_minima`).
> - ✅ **Fase B (velocidades por FÍSICA) HECHA** (`src/engine.rs` reescrito a integración CINEMÁTICA
>   por tick; `cargo run --release -- simulate`). La velocidad la fija `min(Vmax material, Vmax vía
>   CVM/ADIF, LTV, frenada anticipada)`; acelera a ¾ de la máxima; ámbar → −20%; **un tren nunca sale
>   antes de su hora programada** + dwell mínimo. Reutiliza `src/calculator/` (física, material,
>   `adif_profile`, LTV) y `processed/adif/`. Material por línea (`fleet_for_line`): R2→450; regional
>   no R11/R3→470; R1/R3/R4→465 (Civia, **añadida** a `rolling_stock` con dato oficial); resto→447.
>   Verificado: R16 regional a 140 km/h (su Vmax), pico retenidos 4, final 0 (recupera), retraso
>   acumulado 22200 tren·min (baja vs Fase 1b), runtime ~12 s (más caro; deuda de claves `String`).
>   `cargo check` limpio, `cargo test` 32/32.
> - ✅ **Fase A (incidencias probabilísticas) HECHA en el MOTOR** (`src/engine.rs`, subcomando
>   `simulate <seed>`). `IncidentEffect` = 2 tipos (señal en rojo 1/2 sentidos; tramo doble→vía única).
>   `IncidentTemplate` (motivo, efecto, duración, prob. diaria, enabled) + `default_templates()`
>   (4 plantillas razonables, eliminables). Tirada **pre-día** determinista (`roll_incidents`, RNG
>   sembrado, sin máximo): Bernoulli(prob) → ubica ∝ **pesos espaciales** (`spatial_weights`:
>   nudos/Barcelona, estaciones grandes, tramos frecuentados, terminales) y fecha ∝ **densidad
>   temporal** (PERFIL, hora punta). Efectos aplicados vía `dyn_effects`/`Dyn` en la ocupación.
>   Verificado: seed 3 → avería en Barcelona El Clot (nudo) 7–8h, +2065 PAX·min; determinista.
>   `cargo test` 34/34. **PENDIENTE de Fase A = solo su parte de frontend** (creador + headless), en C.
> - ✅ **Fase C (frontend, visual & headless) — SERVIDOR + CREADOR HECHOS** (verificado por API):
>   - Servidor (`src/server.rs`): plantillas persistentes en `report/incidents.json` (se siembran los
>     defaults si falta), `GET/POST /api/incidents/templates` (el frontend envía la lista completa al
>     añadir/borrar/toggle), y `GET /api/simulate?seed=` que corre el motor unificado headless con las
>     plantillas activas → JSON de métricas + incidencias del roll. **Determinista** (se ordenó
>     `spatial_weights`; CLI y servidor coinciden).
>   - Frontend (`game/web/`): **creador de incidencias probabilístico** (motivo/efecto/duración/prob),
>     plantillas eliminables y toggleables; panel **"Simular sin visual (números)"**. Módulo JS aislado
>     que solo habla con la API (no toca el motor del juego). Verificado por curl (crear→persistir→
>     simular usa la plantilla; prob 1.0 → sale en Barcelona Sants, nudo).
> - ✅ **FRONTEND-REPRODUCTOR HECHO** (verificado headless con Playwright): el modo visual del juego
>   YA NO calcula física propia — **reproduce el TRACE del motor Rust**. Motor: `Engine::run_trace(dt)`
>   graba por tren `(cantón, fracción, velocidad)` muestreado (5 s) → `Trace` (arrays de enteros,
>   ~12 MB/jornada); endpoint `GET /api/game/trace?seed=`. Frontend (`app.js`): `cargarTrace` +
>   `pasoRepro` reemplazan a `paso` (respaldo al motor JS si falla); `render` dibuja desde el trace y
>   reconstruye la ocupación para colorear señales como el motor. **Verificado: trenes con velocidad
>   variable real 0–130 km/h (30 velocidades distintas), señales coloreadas, consola limpia.** «Lo que
>   ves = lo que se simula en masa» (mismo motor que el modo números).
> - ✅ **MENÚ de 3 opciones** (`/`): **Información** (`/info`: cómo se calculan los tiempos por física y
>   todas las probabilidades del modelo de incidencias) · **Modo visual** (`/game`, reproductor) ·
>   **Varias runs solo números** (`/numeros`: corre `/api/simulate` para un rango de semillas y compara
>   métricas). Añadidas `render_info_page`/`render_numbers_page` en `report.rs`.
> - ✅ **"Lanzar incidencia AHORA"** (sustituye al generador de incidencias PROGRAMADAS): elige una
>   plantilla + un **tramo** (lista de cantones con nombres acortados) y el motor **re-simula la jornada**
>   con la incidencia inyectada desde la hora actual (`Engine::inject_incident` + params en
>   `/api/game/trace`). Como la semilla es la misma, el trace hasta "ahora" es idéntico → reanuda sin
>   salto. Verificado: lanzar en el tronco de BCN retiene trenes en la ventana.
> - ✅ **PRERREQUISITO DE RENDIMIENTO HECHO** (`src/engine.rs`): claves `String`→índices `u32`. Se
>   internan los `stop_id`→`nid` y se precalculan los **recursos de ocupación** por bloque (`res`
>   normal + `res_single` canónico + `cap_double`); `occ` pasa de `HashMap<String,…>` a
>   `Vec<Vec<usize>>` denso indexado por recurso (se reutiliza entre steps, cero realloc/hashing en
>   caliente). La ruta fría (visor `observe`) resuelve por lookup en `Infra`. **Paridad EXACTA**
>   (retard 22200 tren·min, pic 2124s, pic retinguts 4, final 0, KPI 82920) y **jornada 15175 ms →
>   ~580 ms (~26×)**. `cargo test` 34/34. Determinismo del roll intacto (seed 3 reproduce). Deuda
>   pendiente menor: reuso de la infra/zonas entre runs del optimizador (hoy `Engine::new` reperfila
>   ADIF por run → ~1,4 s/run; ver abajo).
> - ✅ **OPTIMIZADOR MIGRADO al motor unificado** (`src/optimizer/system.rs`): `optimize_system` ya
>   conduce `engine.rs` en vez de `simulation_engine.rs`. Añadido al motor: **desfase de fase por
>   línea** (`Engine::new(..., offsets)` desplaza `pts.a/d` por `route_short_name`), **línea temporal
>   de retraso acumulado** (`Engine::timeline()` cada 60 s → PAX-weighted delay + pico + recuperación
>   en `RunMetrics`) y **conflictos distintos** (`held_events`, flanco de subida de `waiting`). Cada
>   run MC = una **semilla fija por índice** (independiente del desfase → comparación justa; los pesos
>   espaciales/temporales del roll no dependen de la fase). `V = w_delay·PAX-weighted-delay +
>   w_conflict·held`. **Verificado end-to-end** (CLI `optimize`): V 2735→2174 (−20,5%), pico 4282→4011 s,
>   retenciones 359→269; 953 trenes, 201×8 = 1608 runs en 414 s; CSV+PDF exportados. `adif`/`ltv` se
>   cargan en `main`/`server` y se pasan al optimizador. `PotentialWeights::factor` ahora `pub` +
>   `weighted_delay_from_timeline`. Retirados los campos legado `min_block_headway`/`incidents_per_run`
>   de `SystemSearch`. `search.rs` (modo `optimize-line`) sigue en el motor antiguo (se migrará al final).
> - ✅ **MIGRACIÓN A RUST CERRADA** (2026-09-15). El motor unificado `engine.rs` es la ÚNICA fuente
>   de verdad de la física para TODOS los consumidores:
>   1. ✅ **Dashboard `/panel`** (`scenario::sim_view`/`res_view`) migrado a `engine.rs`: reshape de
>      `SimView`/`ResView` (línea temporal `(u32,f64)` del motor, `held_events`, pico, recuperación),
>      se quitó el log CTC y las métricas que el motor no produce; los knobs de capacidad/headway/
>      demanda se retiraron (el motor los DERIVA de ADIF + perfil). `Engine::new` toma `line_filter`.
>      Verificado en vivo (curl): `/panel` 200, `/api/render` da métricas del motor.
>   2. ✅ **`optimize-line` RETIRADO** (`search.rs` borrado): el optimizador de sistema (per-línea,
>      todas a la vez) lo subsume. Limpiado `potential.rs` (fuera `potential`/`regularity_cost`/
>      `passenger_weighted_delay`/`w_regularity`), `exporter.rs` y el CLI.
>   3. ✅ **`simulation_engine.rs` + `signaling.rs` + `passenger_model.rs` BORRADOS** (sin
>      consumidores). También `TrackEdge.capacity` (solo lo leía el motor viejo). `cargo check`
>      LIMPIO, `cargo test` 28/28.
>   4. ✅ **Motor JS del juego RETIRADO** (`app.js`): el reproductor del trace es el ÚNICO motor
>      visual; borrado el stepper `paso`/`bloqueEntrada`/`ocuparBloque` y el respaldo de `render`.
>      Verificado con Playwright: `/game` reproduce el trace a ×25 (reloj 05:00→05:58, 20 trenes
>      dibujados / 40 activos), consola limpia (solo favicon 404 preexistente).
> - ⏭️ **ÚNICO PENDIENTE (perf, no bloqueante):** reusar infra/zonas entre runs del optimizador/panel
>   (plantilla inmutable + instancia barata) para no reperfilar ADIF cada `Engine::new` (~1,4 s/run;
>   optimizador 414 s → ~120 s estimado; el panel `res_view` reconstruye 4 motores por request).
> - ⏭️ **Pendientes menores previos:** **enclavamiento de estación** (`internoRojo`/corredores, inerte
>   en lote); **deuda de rendimiento** (claves `String`→índices `u32`, jornada ~12 s) antes del MC masivo;
>   **interactividad avanzada** (forzar señal manual en el reproductor = re-simular); trace más ligero
>   (submuestreo adaptativo) y KPI/retraso reales en el visor (hoy el trace no lleva retraso por tren).
>   Luego: **Fase B** (velocidades por física: reescribir los tiempos de `pts` con `calculator/`
>   antes de construir la secuencia), **Fase A** (incidencias probabilísticas: `inject`+taxonomía),
>   frontend-reproductor (trace de `observe()`) y migración del optimizador.
>   **Deuda de rendimiento (plan §3.5):** las claves de bloque/recurso se construyen como `String`
>   en el bucle caliente (`key`/`canon`/`sb_res`) → jornada en ~4,1 s a dt=2 s. Internar `stop_id`
>   a índices `u32` ANTES del Monte-Carlo masivo. También falta el `dispersarFlota`/`encadenarFlota`
>   del juego (reparto inicial de material), que aquí no se modela (todos salen a su hora GTFS).

## 1. Objetivo y principio rector

Un **único motor de simulación en Rust** que sea la **fuente única de verdad** de la física
(señalización, cantones, atascos, cruces, vía única, enclavamientos, PAX·min de retraso). Sobre
ese motor, **dos consumidores**:

- **Ver** (frontend juego): reproduce una simulación **lenta y bonita**, se puede tocar. El
  frontend **solo lee y dibuja**; no calcula física.
- **Simular en masa** (headless): correr la MISMA física **miles de veces** en paralelo para
  sacar los distintos PAX·min de retraso y encontrar horarios definitivos.

Invariante clave: **lo que ves es exactamente lo que se simula en masa** (mismo código). Se
elimina la duplicación actual (motor JS del juego vs motor Rust de lote).

## 2. El mecanismo que hace posible "ver" y "correr en masa" con un solo motor

Separar dos responsabilidades dentro del motor:

1. **Paso de física (puro, rápido)**: `step(dt) -> ()` avanza el estado un sub-paso acotado. No
   dibuja, no serializa, no espera. Determinista dado (semilla, entradas).
2. **Observación (opcional)**: emitir el estado observable (posiciones, aspectos de señal,
   retenciones, retraso). Solo se activa al *ver*.

El **driver** decide el modo:
- **Lote**: llama a `step` hasta fin de jornada sin observar → devuelve solo métricas (PAX·min,
  pico de retraso, recuperación, conflictos). Máxima velocidad, `rayon` entre runs.
- **Ver**: corre el motor **grabando un trace** (línea de tiempo de estados) y lo sirve al
  frontend, que lo **reproduce** a la velocidad deseada (lento/bonito/rebobinable).

## 3. Diseño del motor (Rust)

### 3.1 Paradigma: sub-pasos acotados (NO eventos discretos)
Es el paradigma del juego actual (`paso()`), y es el correcto para **física continua**: con
velocidades por física los trenes aceleran/frenan, se alcanzan y forman atascos graduales, y los
cruces se resuelven de forma continua — todo emerge naturalmente por tick. (El motor de eventos
actual es bueno para colas/horarios puros, pero modela mal la cinemática continua.)
Regla obligatoria (feedback del usuario, ver [[feedback-simulation-engine-design]]): **nada de
ventanas de tiempo fijas**; sub-paso acotado (`PASO_MAX_S`), y **verificar SIEMPRE a ×25**, donde
afloran los bugs de salto.

### 3.2 Estado (`SimState`)
- Trenes: posición (cantón + fracción), velocidad actual, índice en su secuencia de cantones,
  estado (en marcha / detenido / retenido y por qué), retraso acumulado, dwell restante.
- Infraestructura: ocupación por bloque (1 tren/bloque; vía única = testigo compartido),
  aspecto de cada señal (derivado), enclavamientos por estación (vías, verde por lado, ocupación).
- Reloj de jornada, RNG sembrado, acumulador de KPI (PAX·min).

### 3.3 API del motor (borrador)
```rust
pub struct Engine<'n> { net: &'n Network, cfg: EngineCfg, state: SimState, rng: StdRng }
impl<'n> Engine<'n> {
    pub fn new(net: &'n Network, cfg: EngineCfg) -> Self;   // precalcula índices (§3.5)
    pub fn step(&mut self, dt_sim: f64);                    // un sub-paso (puro)
    pub fn observe(&self) -> FrameView;                     // estado observable (para trace/stream)
    pub fn metrics(&self) -> RunMetrics;                    // PAX·min, pico, recuperación, held
    pub fn inject(&mut self, inc: Incident);               // incidencia en caliente (interactividad)
}
```
- **Lote**: `while t < fin { step(dt) }; metrics()`.
- **Ver**: `while t < fin { step(dt); trace.push(observe()) }` (o submuestreo por segundo).

### 3.4 Modelo de recorrido de cantón (PLUGGABLE) — clave para el trabajo previo B
El tiempo/velocidad al recorrer un cantón sale de un **trait**:
```rust
trait TraversalModel { fn speed(&self, ctx: &BlockCtx) -> f64; /* o run_time */ }
```
- **Fase 1 (paridad):** `ScheduleTraversal` — usa `nominal_run_secs` del GTFS (comportamiento
  actual; permite verificar que el juego se ve igual tras portar a Rust).
- **Fase B (realismo):** `PhysicsTraversal` — velocidad por **Vmax del tramo + LTV (limitaciones
  temporales) + física (aceleración/frenada)**, reutilizando `src/calculator/` (physics.rs,
  line_analysis.rs, ltv.rs, infrastructure.rs). El horario deja de dictar la velocidad.

### 3.5 Rendimiento (bucle caliente)
Objetivo: un día completo (~960 trenes) con señalización en **< X ms** (fijar X con benchmark).
- Precalcular UNA vez al construir: secuencia de cantones por tren, índices de arista→líneas,
  usuarios de vía única, sub-bloques. Nada de recalcular por tick ni por tren.
- Cero asignaciones por tick; trabajo por tick O(trenes)+O(señales activas), no O(trenes×señales).
- `rayon` entre runs independientes en lote (patrón ya usado en `optimizer/system.rs`).

### 3.6 Determinismo
RNG sembrado (`StdRng::seed_from_u64`) para Monte-Carlo reproducible y para que el replay coincida
con el run. Sin aleatoriedad no sembrada (el JS actual usa `Math.random` en sitios: no portar eso).

## 4. Física a portar desde `game/web/app.js` (regla por regla, con test)
`bloqueEntrada` (avance/retención), `aspectoBloque`/`siguienteRojo` (rojo/ámbar/verde),
`bloqueOcupado`/`ocuparBloque` (1 tren/bloque), vía única (testigo canónico + prioridad regional),
`interlock`/`internoRojo`/`viaPrincipal` (enclavamiento), `imponerParadaMinima` (dwell), atascos
(seguir al de delante), cruces (cizallamiento/shear). Cada regla = función pura y testeable.

## 5. Transporte hacia el frontend
- **Fase 1: grabar→reproducir.** Endpoint que corre el motor, graba el trace (submuestreado, p.ej.
  1–4 Hz de tiempo de juego con interpolación en cliente) y lo sirve como JSON/binario. El frontend
  se convierte en **reproductor**: dibuja posiciones y aspectos, con control de velocidad y barra
  de tiempo. Sin física en cliente.
- **Fase 2: interactividad.** Lanzar incidencia / forzar semáforo a media marcha → **re-simular
  desde ese instante** (el motor Rust es tan rápido que es casi instantáneo) o *stepping* en vivo
  por **WebSocket** (deltas paced a la velocidad de visualización).

## 6. Estructura de módulos (propuesta)
```
src/engine/
  mod.rs         # Engine, EngineCfg, SimState, RunMetrics, FrameView
  step.rs        # el sub-paso (orquestación por tick)
  blocks.rs      # ocupación de cantón / sub-bloques / vía única (testigo)
  signals.rs     # aspecto de señal (reutiliza/absorbe signaling.rs)
  interlock.rs   # enclavamiento de estación (vías, vía principal, corredores)
  traversal.rs   # trait TraversalModel + ScheduleTraversal + PhysicsTraversal
  incidents.rs   # taxonomía + modelo probabilístico (trabajo previo A)
  metrics.rs     # PAX·min ponderado, pico, recuperación, conflictos
  trace.rs       # FrameView + grabación/submuestreo para el reproductor
batch: subcomando CLI `simulate`/`batch` (headless, rayon) → vuelca métricas.
```
El `simulation_engine.rs` (eventos) y el optimizador se migran a este motor cuando haya paridad;
hasta entonces conviven.

## 7. Trabajos PREVIOS (antes de la unificación completa)

### A. Incidencias: definirlas y darles probabilidad
- **Taxonomía** completa (avería de puertas, falta de tensión/catenaria, avería mecánica grave,
  bloqueo de cantón, retraso de tren, restricción de vía…), cada una con efecto físico claro
  (parada, ralentización, bloqueo) y duración.
- **Modelo probabilístico**: probabilidad por unidad de tiempo/tramo/hora punta, para generar
  escenarios Monte-Carlo reproducibles (semilla). Es lo que alimentará la búsqueda de horarios.

### B. Tiempos por FÍSICA, no por horario  ← decisión de diseño central
Los trenes **no** derivan su tiempo de marcha del horario. La velocidad sale de: **Vmax del tramo
+ limitaciones temporales de velocidad (LTV) + física (aceleración/frenada)**. El horario pasa a
ser un **objetivo**, no una imposición de velocidad: dicta **cuándo deberían estar los trenes en
las estaciones**, y sobre todo el **margen de espera (dwell) en estación** y las **horas de salida
de las cabeceras**. Como en la realidad: el sistema *intenta* seguir el horario, pero la física
manda la velocidad máxima alcanzable. (La física ya existe en `src/calculator/`.)

## 8. Orden recomendado (ver decisión en el chat)
Recomendación: **1) núcleo del motor unificado en Rust con PARIDAD** (portar la física actual,
modelo de recorrido = horario) para tener la fuente única y verificar que el juego se ve igual →
**2) B (realismo de velocidades)** dentro del motor → **3) A (incidencias probabilísticas)** dentro
del motor → 4) frontend como reproductor puro + interactividad → 5) migrar el optimizador al motor
unificado. Motivo: A y B son física que DEBE vivir en el motor único para poder verse Y correrse en
masa; implementarlas antes en el JS del juego o en el motor de eventos actual (que se sustituye)
sería trabajo tirado.

## 9. Decisiones abiertas
- Objetivo de rendimiento concreto (ms/día-simulado) → fijar con benchmark inicial.
- Frecuencia de submuestreo del trace y si es JSON o binario.
- Interactividad: re-simular desde instante vs WebSocket en vivo (empezar por re-simular).
- Formato del PAX·min definitivo (ya existe `passenger_weighted_delay`; confirmar que es el KPI).
