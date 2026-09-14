# Plan técnico — Motor de simulación unificado (Rodalies)

> **Estado:** aprobado como rumbo (2026-09-14). Orden en §8. Trabajos previos en §7.
>
> ### PROGRESO
> - ✅ **Fase 1 (fundamentos) HECHA** (`src/engine.rs`, subcomando `cargo run --release -- simulate`):
>   `Engine` con `step`/`observe`/`run`/`metrics`, `TraversalModel` enchufable (`ScheduleTraversal`
>   = paridad por horario), cinemática por cantón con dwell y retraso, determinista, 2 tests. Corre
>   la jornada completa (~961 trenes) headless en ~90 ms. `cargo check` limpio.
> - ⏭️ **SIGUE — Fase 1b (señalización/ocupación):** portar de `game/web/app.js` a `src/engine.rs`
>   (idealmente ya dividido en `src/engine/` según §6), regla por regla con test de paridad:
>   1. **Ocupación de cantón** (1 tren/bloque) + retención → `bloqueOcupado`/`ocuparBloque`/`bloqueEntrada`.
>   2. **Aspecto de señal** (rojo/ámbar/verde) → `aspectoBloque`/`siguienteRojo` (absorber `signaling.rs`).
>   3. **Vía única** (testigo compartido + prioridad regional) → `regionalEntrandoPronto`, claves canónicas.
>   4. **Enclavamiento de estación** (vías, vía principal, corredores) → `interlock`/`internoRojo`/`viaPrincipal`.
>   5. **Atascos** (seguir al de delante) — emergen al frenar por señal; verificar a ×25.
>   Luego: Fase B (velocidades por física, cambiar el `TraversalModel`), Fase A (incidencias
>   probabilísticas), y frontend-reproductor (trace) + migración del optimizador.
>   Nota de rendimiento pendiente: `Segment` usa ids `String`; cambiar a índices de nodo antes de
>   Monte-Carlo masivo (plan §3.5). El retraso que hoy reporta `simulate` es un **proxy** (paridad
>   por horario, sin conflictos): algunos exprés salen "tarde" porque el tiempo nominal del cantón
>   (mín. del grafo) es mayor que su hueco — se resuelve con la física de la Fase B.

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
