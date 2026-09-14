//! Núcleo del **motor de simulación unificado** (Rodalies) — FASE 1: fundamentos + cinemática.
//!
//! Objetivo del motor (ver `docs/PLAN-motor-unificado.md`): ser la **fuente única de verdad** de
//! la física, servida a la vez para **ver** (frontend como reproductor) y para **simular en masa**
//! (headless, `rayon`). El principio es separar dos responsabilidades:
//!   * `step(dt)` — un sub-paso de física, **puro y rápido** (sin dibujar, sin serializar).
//!   * `observe()` — el estado observable (posiciones/retraso), **solo** cuando se quiere ver.
//! El driver decide: lote = `run()` sin observar → métricas; ver = `step`+`observe` → trace.
//!
//! **Estado de esta fase (paridad):** los trenes recorren su secuencia real de cantones usando el
//! tiempo de marcha del GTFS (`ScheduleTraversal`), respetan su parada mínima (dwell) y acumulan
//! retraso frente al horario. El **modelo de recorrido es enchufable** (`TraversalModel`) para que
//! la Fase B lo sustituya por física (Vmax + LTV + acel./frenada) sin tocar el bucle.
//!
//! **PENDIENTE (marcado como Fase 1b/1c en el plan; NO está aquí todavía):**
//!   * Ocupación de cantón (1 tren/bloque) + retención + aspecto de señal (rojo/ámbar/verde).
//!   * Vía única (testigo compartido) + enclavamiento de estación + atascos (seguir al de delante).
//!   * RNG sembrado + incidencias probabilísticas (Fase A) y velocidades por física (Fase B).
//! Hasta que esté 1b, este motor mueve trenes "a horario" sin conflictos: sirve de esqueleto
//! verificable, no de simulación completa.
//!
//! Regla de oro (ver memoria de feedback): **nada de ventanas de tiempo fijas**; el sub-paso `dt`
//! lo acota el driver y hay que **verificar SIEMPRE a ×25** (donde afloran los bugs de salto).

use crate::gtfs_loader::Network;

/// Cómo se traduce un cantón en tiempo de marcha. Fase 1 = horario (paridad); Fase B = física.
pub trait TraversalModel {
    /// Segundos que tarda un tren en recorrer un cantón cuyo tiempo nominal GTFS es `nominal_secs`.
    fn run_secs(&self, nominal_secs: u32) -> f64;
}

/// Paridad con el comportamiento actual: el tiempo de marcha es el nominal del GTFS.
pub struct ScheduleTraversal;
impl TraversalModel for ScheduleTraversal {
    fn run_secs(&self, nominal_secs: u32) -> f64 {
        (nominal_secs.max(1)) as f64
    }
}

/// Un cantón del recorrido de un tren (por ahora con ids de estación; la Fase de rendimiento los
/// cambiará por índices de nodo para evitar clones en caliente — ver plan §3.5).
#[derive(Clone)]
struct Segment {
    from: String,
    to: String,
    run_secs: u32,
}

/// Estado en marcha de un tren.
struct TrainRt {
    id: String,
    line: String,
    segments: Vec<Segment>,
    dwell_secs: Vec<u32>, // parada reglamentaria en el destino de cada cantón (0 si pasa/terminal)
    sched_arr: Vec<u32>,  // llegada PROGRAMADA al destino de cada cantón (para medir retraso)
    sched_dep0: u32,      // salida programada de la cabecera
    // progreso:
    seg_idx: usize,   // cantón actual
    seg_elapsed: f64, // segundos dentro del cantón actual
    dwelling: f64,    // segundos de parada restantes en estación
    delay_s: f64,     // retraso actual (actualizado en cada llegada)
    done: bool,
}

impl TrainRt {
    /// Avanza el tren un sub-paso. Fase 1: sin conflictos (ni ocupación ni señales) — eso llega en
    /// la Fase 1b. El horario solo marca cuándo sale de cabecera y la parada mínima.
    fn advance(&mut self, now: f64, dt: f64, model: &dyn TraversalModel) {
        if self.done || now < self.sched_dep0 as f64 {
            return;
        }
        if self.seg_idx >= self.segments.len() {
            self.done = true;
            return;
        }
        if self.dwelling > 0.0 {
            self.dwelling = (self.dwelling - dt).max(0.0);
            return;
        }
        let run = model.run_secs(self.segments[self.seg_idx].run_secs).max(1e-6);
        self.seg_elapsed += dt;
        if self.seg_elapsed >= run {
            // Llegada al destino del cantón: mide retraso, aplica parada y pasa al siguiente.
            self.delay_s = (now - self.sched_arr[self.seg_idx] as f64).max(0.0);
            self.dwelling = self.dwell_secs[self.seg_idx] as f64;
            self.seg_idx += 1;
            self.seg_elapsed = 0.0;
            if self.seg_idx >= self.segments.len() {
                self.done = true;
            }
        }
    }
}

/// Parámetros de una corrida.
#[derive(Clone, Copy)]
pub struct EngineCfg {
    pub start_sec: u32,
    pub end_sec: u32,
    pub dt_sec: f64,
}
impl EngineCfg {
    /// Jornada completa (05:00–00:00) con sub-paso acotado por defecto.
    pub fn day() -> Self {
        EngineCfg { start_sec: 5 * 3600, end_sec: 24 * 3600, dt_sec: 2.0 }
    }
}

/// Métricas de una corrida (headless). El PAX·min ponderado por pasajeros llegará al integrar el
/// `passenger_model`; de momento se reporta el retraso en **tren·min** (proxy honesto).
#[derive(Clone, Copy)]
pub struct RunMetrics {
    pub n_trains: usize,
    pub sim_seconds: u32,
    pub train_min_delay: f64,
    pub max_delay_s: f64,
}

/// Una foto del estado observable de un tren (para el reproductor del frontend).
pub struct FrameTrain {
    pub id: String,
    pub line: String,
    pub from: String,
    pub to: String,
    pub frac: f64, // fracción recorrida del cantón actual [0,1]
    pub delay_s: f64,
}

/// El motor. No guarda `&Network` tras construir: precalcula la secuencia de cantones de cada tren
/// en `new()` y luego opera sobre esa estructura (encaja con "precalcular fuera del bucle").
pub struct Engine {
    cfg: EngineCfg,
    clock: f64,
    trains: Vec<TrainRt>,
    model: Box<dyn TraversalModel>,
    train_min_delay: f64,
    max_delay: f64,
}

impl Engine {
    /// Construye el motor desde la red: toma el servicio dominante (día tipo), excluye buses y
    /// precalcula, por tren, sus cantones con el tiempo nominal de marcha (edge del grafo, con
    /// respaldo al hueco del horario si falta la arista).
    pub fn new(net: &Network, cfg: EngineCfg) -> Self {
        let service_id = net.dominant_service().unwrap_or_default();
        let mut trains = Vec::new();
        for svc in &net.services {
            if svc.is_bus || svc.service_id != service_id || svc.schedule.len() < 2 {
                continue;
            }
            let n = svc.schedule.len() - 1;
            let mut segments = Vec::with_capacity(n);
            let mut dwell_secs = Vec::with_capacity(n);
            let mut sched_arr = Vec::with_capacity(n);
            for w in svc.schedule.windows(2) {
                let run = net
                    .edge_between(&w[0].stop_id, &w[1].stop_id)
                    .map(|e| e.nominal_run_secs)
                    .unwrap_or_else(|| w[1].arrival_sec.saturating_sub(w[0].departure_sec))
                    .max(1);
                segments.push(Segment { from: w[0].stop_id.clone(), to: w[1].stop_id.clone(), run_secs: run });
                sched_arr.push(w[1].arrival_sec);
                dwell_secs.push(w[1].departure_sec.saturating_sub(w[1].arrival_sec));
            }
            trains.push(TrainRt {
                id: svc.train_number.clone(),
                line: svc.route_short_name.clone(),
                segments,
                dwell_secs,
                sched_arr,
                sched_dep0: svc.schedule[0].departure_sec,
                seg_idx: 0,
                seg_elapsed: 0.0,
                dwelling: 0.0,
                delay_s: 0.0,
                done: false,
            });
        }
        Engine {
            cfg,
            clock: cfg.start_sec as f64,
            trains,
            model: Box::new(ScheduleTraversal),
            train_min_delay: 0.0,
            max_delay: 0.0,
        }
    }

    /// Reloj de simulación actual (segundos desde medianoche).
    pub fn clock(&self) -> f64 {
        self.clock
    }

    /// Un sub-paso de física para toda la red.
    pub fn step(&mut self, dt: f64) {
        self.clock += dt;
        let now = self.clock;
        let dt_min = dt / 60.0;
        for t in &mut self.trains {
            t.advance(now, dt, self.model.as_ref());
            if !t.done && now >= t.sched_dep0 as f64 {
                self.train_min_delay += (t.delay_s / 60.0) * dt_min;
                if t.delay_s > self.max_delay {
                    self.max_delay = t.delay_s;
                }
            }
        }
    }

    /// Corre hasta el fin de la ventana y devuelve las métricas (camino de LOTE: sin observar).
    pub fn run(&mut self) -> RunMetrics {
        while self.clock < self.cfg.end_sec as f64 {
            self.step(self.cfg.dt_sec);
        }
        self.metrics()
    }

    /// Estado observable de los trenes en marcha ahora (camino de VER: para el trace/reproductor).
    pub fn observe(&self) -> Vec<FrameTrain> {
        let now = self.clock;
        self.trains
            .iter()
            .filter(|t| !t.done && now >= t.sched_dep0 as f64 && t.seg_idx < t.segments.len())
            .map(|t| {
                let seg = &t.segments[t.seg_idx];
                let run = self.model.run_secs(seg.run_secs).max(1e-6);
                FrameTrain {
                    id: t.id.clone(),
                    line: t.line.clone(),
                    from: seg.from.clone(),
                    to: seg.to.clone(),
                    frac: (t.seg_elapsed / run).clamp(0.0, 1.0),
                    delay_s: t.delay_s,
                }
            })
            .collect()
    }

    /// Métricas actuales.
    pub fn metrics(&self) -> RunMetrics {
        RunMetrics {
            n_trains: self.trains.len(),
            sim_seconds: self.cfg.end_sec - self.cfg.start_sec,
            train_min_delay: self.train_min_delay,
            max_delay_s: self.max_delay,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_traversal_devuelve_nominal() {
        assert_eq!(ScheduleTraversal.run_secs(90), 90.0);
        assert!(ScheduleTraversal.run_secs(0) >= 1.0); // nunca 0 (evita división por cero)
    }

    #[test]
    fn tren_avanza_por_cantones_y_termina() {
        // Tren con 2 cantones de 60 s y una parada de 30 s en la estación intermedia.
        let mut t = TrainRt {
            id: "T".into(),
            line: "R1".into(),
            segments: vec![
                Segment { from: "A".into(), to: "B".into(), run_secs: 60 },
                Segment { from: "B".into(), to: "C".into(), run_secs: 60 },
            ],
            dwell_secs: vec![30, 0],
            sched_arr: vec![60, 150],
            sched_dep0: 0,
            seg_idx: 0,
            seg_elapsed: 0.0,
            dwelling: 0.0,
            delay_s: 0.0,
            done: false,
        };
        let m = ScheduleTraversal;
        let mut now = 0.0;
        let dt = 1.0;
        for _ in 0..1000 {
            now += dt;
            t.advance(now, dt, &m);
            if t.done {
                break;
            }
        }
        assert!(t.done);
        assert_eq!(t.seg_idx, 2);
        // A horario nominal el retraso final debe ser pequeño (± un sub-paso).
        assert!(t.delay_s <= 2.0, "retraso inesperado: {}", t.delay_s);
    }
}
