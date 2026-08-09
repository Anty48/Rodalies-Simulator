//! Motor de simulación por eventos discretos (cola de prioridades por tiempo).
//!
//! Modela:
//!   * Ocupación de andenes/vías por estación (capacidad configurable).
//!   * Ocupación de cantones (aristas): 1 tren por sección → señalización.
//!   * Retención segundo a segundo cuando el cantón/andén siguiente está ocupado.
//!   * Inyección de incidencias (retraso puntual a un tren, bloqueo de cantón).
//!   * Métrica de estabilidad: retraso medio de la red muestreado en el tiempo.

use std::collections::{BinaryHeap, HashMap, HashSet};
use std::cmp::Reverse;

use petgraph::graph::NodeIndex;
use rand::rngs::StdRng;
use rand::SeedableRng;

use crate::gtfs_loader::{fmt_hms, Network};
use crate::passenger_model::PassengerModel;
use crate::signaling::{Aspect, Signals};

/// Granularidad de reintento cuando un tren queda retenido por señal (segundos).
const RETRY_STEP: u32 = 10;

// --------------------------------------------------------------------------
// Configuración e incidencias
// --------------------------------------------------------------------------

pub struct SimConfig {
    pub start_sec: u32,
    pub end_sec: u32,
    pub passenger: PassengerModel,
    /// Vías/andenes por estación (capacidad de nodo).
    pub platform_capacity: u32,
    /// Separación mínima entre trenes dentro de una misma sección (segundos).
    /// La capacidad efectiva de un cantón = max(1, marcha / este valor), lo que
    /// modela que una sección larga contiene varios bloques de señalización y por
    /// tanto admite varios trenes en marcha (p.ej. el tronc central de Barcelona).
    pub min_block_headway_secs: u32,
    /// Estaciones cuyo movimiento se reporta en el log tipo CTC (subcadenas de nombre).
    pub key_stations: Vec<String>,
    /// Si es `Some`, la generación de pasajeros es estocástica con esta semilla
    /// (usa `PassengerModel::dwell_time_random`). `None` → dwell determinista.
    pub seed: Option<u64>,
    /// Si es `Some`, solo se simulan los servicios de esa línea (route_short_name).
    pub line_filter: Option<String>,
    /// Señalización estricta: capacidad 1 por cantón y por andén (block system).
    /// Cuando es `true` ignora `platform_capacity`/`min_block_headway_secs` y aplica
    /// el aspecto amarillo (ralentización) de `signals`.
    pub strict_signaling: bool,
    /// Parámetros de los tres aspectos (verde/amarillo/rojo).
    pub signals: Signals,
    /// Desplazamientos (segundos) por `trip_id` respecto al horario GTFS base.
    /// Los usa el optimizador para probar variaciones de la hora de salida.
    pub offsets: std::sync::Arc<std::collections::HashMap<String, i64>>,
    /// Segmentos de vía única (pares de nodos NO dirigidos): un solo tren por tramo
    /// en cualquiera de los dos sentidos (testigo). Solo aplica en modo estricto.
    pub single_track: std::sync::Arc<std::collections::HashSet<(NodeIndex, NodeIndex)>>,
    /// Excluir servicios de autobús (route_type 3) de la simulación ferroviaria.
    pub exclude_buses: bool,
}

impl Default for SimConfig {
    fn default() -> Self {
        SimConfig {
            start_sec: 7 * 3600,
            end_sec: 9 * 3600,
            passenger: PassengerModel::default(),
            platform_capacity: 4,
            min_block_headway_secs: 120,
            key_stations: vec![
                "Clot".into(),
                "Passeig de Gràcia".into(),
                "Catalunya".into(),
                "Sants".into(),
                "França".into(),
                "Arc de Triomf".into(),
            ],
            seed: None,
            line_filter: None,
            strict_signaling: false,
            signals: Signals::default(),
            offsets: std::sync::Arc::new(std::collections::HashMap::new()),
            single_track: std::sync::Arc::new(std::collections::HashSet::new()),
            exclude_buses: false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Incident {
    /// Retraso puntual: al llegar el tren `train_number` a `at_stop_name`, +`extra_secs`.
    TrainDelay {
        train_number: String,
        at_stop_name: String,
        extra_secs: u32,
    },
    /// Bloqueo de un cantón identificado por `stop_id` (resolución exacta).
    BlockSegmentById {
        from_stop_id: String,
        to_stop_id: String,
        from_sec: u32,
        dur_secs: u32,
    },
}

// --------------------------------------------------------------------------
// Eventos
// --------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventKind {
    /// El tren intenta ENTRAR/llegar a `schedule[idx]`.
    Arrive,
    /// El tren intenta SALIR de `schedule[idx]` hacia el siguiente cantón.
    Depart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Event {
    time: u32,
    trip_idx: usize,
    stop_idx: usize,
    kind: EventKind,
}

// Orden para BinaryHeap: menor tiempo primero (usamos Reverse en la cola).
impl Ord for Event {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.time
            .cmp(&other.time)
            .then_with(|| self.trip_idx.cmp(&other.trip_idx))
            .then_with(|| self.stop_idx.cmp(&other.stop_idx))
    }
}
impl PartialOrd for Event {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

// --------------------------------------------------------------------------
// Estado por tren
// --------------------------------------------------------------------------

struct TrainRt {
    /// Retraso actual respecto al horario (segundos, puede ser negativo si adelanta).
    delay: i64,
    finished: bool,
    active: bool,
    /// Nodo del andén que ocupa ahora mismo (si está parado en estación).
    at_node: Option<NodeIndex>,
    /// Vía/andén asignada dinámicamente en la estación actual (para el log CTC).
    track: Option<usize>,
    /// Cantón que ocupa (arista) y debe liberar al llegar a la siguiente parada.
    holding_edge: Option<(NodeIndex, NodeIndex)>,
    /// Incidencia de retraso ya aplicada (para no repetirla).
    delay_incident_done: bool,
    /// `true` mientras está retenido por señal (para contar conflictos distintos, no
    /// cada reintento de 10 s).
    waiting: bool,
}

// --------------------------------------------------------------------------
// Resultado
// --------------------------------------------------------------------------

/// Una muestra de la métrica de estabilidad en un instante.
#[derive(Debug, Clone, Copy)]
pub struct Sample {
    pub time: u32,
    /// Retraso acumulado global de la red (suma de retrasos positivos, en segundos).
    pub total_delay: i64,
    /// Retraso medio entre los trenes activos (segundos).
    pub mean_active: f64,
    /// Trenes con retraso apreciable (> 30 s).
    pub delayed: usize,
}

/// Evento estructurado del log tipo CTC (para consumo por el dashboard HTML).
#[derive(Debug, Clone)]
pub struct CtcEvent {
    pub time: u32,
    pub train: String,
    pub line: String,
    /// "ARRIBA", "SURT" o "INCIDÈNCIA".
    pub kind: String,
    pub station: String,
    pub track: String,
    pub delay: i64,
}

pub struct SimResult {
    pub timeline: Vec<Sample>,
    /// Declaraciones de incidencias inyectadas (texto).
    pub incidents: Vec<String>,
    /// Eventos CTC estructurados (para el dashboard).
    pub events: Vec<CtcEvent>,
    pub trains_run: usize,
    pub total_arrivals: usize,
    pub held_events: usize,
    /// Pico de retraso acumulado global (segundos) y el instante en que ocurre.
    pub peak_total_delay: i64,
    pub peak_time: u32,
    /// Nº máximo de trenes retrasados simultáneamente.
    pub peak_delayed: usize,
    /// Instante en que la red vuelve al equilibrio tras el pico.
    pub recovery_time: Option<u32>,
}

// --------------------------------------------------------------------------
// Motor
// --------------------------------------------------------------------------

pub struct Simulator<'a> {
    net: &'a Network,
    cfg: SimConfig,
    incidents: Vec<Incident>,
}

impl<'a> Simulator<'a> {
    pub fn new(net: &'a Network, cfg: SimConfig) -> Self {
        Simulator {
            net,
            cfg,
            incidents: Vec::new(),
        }
    }

    pub fn add_incident(&mut self, inc: Incident) {
        self.incidents.push(inc);
    }

    fn is_key_station(&self, node: NodeIndex) -> bool {
        let name = self.net.graph[node].stop_name.to_lowercase();
        self.cfg
            .key_stations
            .iter()
            .any(|k| name.contains(&k.to_lowercase()))
    }

    /// Ejecuta la simulación. Solo se consideran los servicios de `service_id`
    /// cuyo primer paso cae dentro de la ventana [start, end].
    pub fn run(&self, service_id: &str) -> SimResult {
        let net = self.net;
        let cfg = &self.cfg;

        // --- Selección de trenes participantes ---
        let mut participants: Vec<usize> = Vec::new();
        for (idx, svc) in net.services.iter().enumerate() {
            if svc.service_id != service_id {
                continue;
            }
            if let Some(lf) = &cfg.line_filter {
                if &svc.route_short_name != lf {
                    continue;
                }
            }
            if cfg.exclude_buses && svc.is_bus {
                continue;
            }
            match svc.first_time() {
                Some(t) if t >= cfg.start_sec && t <= cfg.end_sec => participants.push(idx),
                _ => {}
            }
        }

        // RNG opcional para la generación probabilística de pasajeros.
        let mut rng = cfg.seed.map(StdRng::seed_from_u64);

        // Desplazamiento horario (segundos) del trip respecto al GTFS base.
        let off = |idx: usize| -> i64 {
            cfg.offsets
                .get(&net.services[idx].trip_id)
                .copied()
                .unwrap_or(0)
        };
        // Capacidad de andén: en modo estricto usa el nº real de vías de la estación
        // (dato operativo, ver `topology`); si no, la capacidad uniforme configurada.
        let node_cap = |node: NodeIndex| -> usize {
            if cfg.strict_signaling {
                crate::topology::platform_tracks(&net.graph[node].stop_name).max(1) as usize
            } else {
                cfg.platform_capacity.max(1) as usize
            }
        };

        let mut trains: HashMap<usize, TrainRt> = HashMap::new();
        let mut heap: BinaryHeap<Reverse<Event>> = BinaryHeap::new();

        for &idx in &participants {
            trains.insert(
                idx,
                TrainRt {
                    delay: 0,
                    finished: false,
                    active: false,
                    at_node: None,
                    track: None,
                    holding_edge: None,
                    delay_incident_done: false,
                    waiting: false,
                },
            );
            let t0 = (net.services[idx].schedule[0].arrival_sec as i64 + off(idx)).max(0) as u32;
            heap.push(Reverse(Event {
                time: t0,
                trip_idx: idx,
                stop_idx: 0,
                kind: EventKind::Arrive,
            }));
        }

        // --- Estado de ocupación de la infraestructura ---
        // node -> vías ocupadas (Vec de longitud capacity, Some(trip) si ocupada)
        let mut node_tracks: HashMap<NodeIndex, Vec<Option<usize>>> = HashMap::new();
        // cantón (from,to) -> nº de trenes actualmente en la sección (bloques ocupados)
        let mut edge_busy: HashMap<(NodeIndex, NodeIndex), u32> = HashMap::new();
        // Vía única: par NO dirigido -> (sentido activo dirigido, nº de trenes en el tramo).
        let mut st_token: HashMap<(NodeIndex, NodeIndex), ((NodeIndex, NodeIndex), u32)> =
            HashMap::new();

        // Bloqueos de cantón por incidencia: (from,to) -> (desde, hasta)
        let mut blocked: HashMap<(NodeIndex, NodeIndex), (u32, u32)> = HashMap::new();
        // Retrasos puntuales por incidencia: (train_number, stop_node) -> extra_secs
        let mut delay_incidents: HashMap<(String, NodeIndex), u32> = HashMap::new();

        let mut incident_log: Vec<String> = Vec::new();
        for inc in &self.incidents {
            match inc {
                Incident::BlockSegmentById {
                    from_stop_id,
                    to_stop_id,
                    from_sec,
                    dur_secs,
                } => match (net.node(from_stop_id), net.node(to_stop_id)) {
                    (Some(na), Some(nb)) => {
                        blocked.insert((na, nb), (*from_sec, from_sec + dur_secs));
                        incident_log.push(format!(
                            "  ⛔ INCIDÈNCIA: bloqueig del cantó {} → {} de {} a {}",
                            net.stop_name(from_stop_id),
                            net.stop_name(to_stop_id),
                            fmt_hms(*from_sec),
                            fmt_hms(from_sec + dur_secs)
                        ));
                    }
                    _ => incident_log.push(format!(
                        "  ⚠ Incidència de bloqueig (id) ignorada: {} → {}",
                        from_stop_id, to_stop_id
                    )),
                },
                Incident::TrainDelay {
                    train_number,
                    at_stop_name,
                    extra_secs,
                } => match net.find_stop_by_name(at_stop_name) {
                    Some(s) => {
                        let n = net.node_of_stop[&s.stop_id];
                        delay_incidents.insert((train_number.clone(), n), *extra_secs);
                        incident_log.push(format!(
                            "  ⛔ INCIDÈNCIA: retard de +{} min al tren {} a {}",
                            extra_secs / 60,
                            train_number,
                            s.stop_name
                        ));
                    }
                    None => incident_log.push(format!(
                        "  ⚠ Incidència de retard ignorada: no trobo l'estació {}",
                        at_stop_name
                    )),
                },
            }
        }

        // --- Métrica de estabilidad ---
        let mut timeline: Vec<Sample> = Vec::new();
        let incidents: Vec<String> = incident_log;
        let mut events: Vec<CtcEvent> = Vec::new();
        let mut total_arrivals = 0usize;
        let mut held_events = 0usize;
        let mut next_sample = cfg.start_sec;

        let sample = |time: u32, trains: &HashMap<usize, TrainRt>, timeline: &mut Vec<Sample>| {
            let mut total = 0i64;
            let mut n = 0usize;
            let mut delayed = 0usize;
            for t in trains.values() {
                if t.active && !t.finished {
                    let d = t.delay.max(0);
                    total += d;
                    n += 1;
                    if d > 30 {
                        delayed += 1;
                    }
                }
            }
            let mean = if n > 0 { total as f64 / n as f64 } else { 0.0 };
            timeline.push(Sample {
                time,
                total_delay: total,
                mean_active: mean,
                delayed,
            });
        };

        // --- Bucle de eventos ---
        while let Some(Reverse(ev)) = heap.pop() {
            if ev.time > cfg.end_sec {
                break;
            }

            // Muestreo de la métrica de estabilidad hasta el tiempo del evento.
            while next_sample <= ev.time && next_sample <= cfg.end_sec {
                sample(next_sample, &trains, &mut timeline);
                next_sample += 60;
            }

            let svc = &net.services[ev.trip_idx];
            match ev.kind {
                EventKind::Arrive => {
                    let stop = &svc.schedule[ev.stop_idx];
                    let node = match net.node_of_stop.get(&stop.stop_id) {
                        Some(&n) => n,
                        None => continue,
                    };

                    // Señalización de andén: ¿hay vía libre?
                    let cap = node_cap(node);
                    let slots = node_tracks.entry(node).or_insert_with(|| vec![None; cap]);
                    let free = slots.iter().position(|s| s.is_none());
                    let Some(track) = free else {
                        // Andén saturado: el tren espera en el cantón anterior,
                        // acumulando retraso segundo a segundo.
                        if let Some(t) = trains.get_mut(&ev.trip_idx) {
                            if !t.waiting {
                                held_events += 1;
                                t.waiting = true;
                            }
                            t.delay += RETRY_STEP as i64;
                        }
                        heap.push(Reverse(Event {
                            time: ev.time + RETRY_STEP,
                            ..ev
                        }));
                        continue;
                    };
                    slots[track] = Some(ev.trip_idx);

                    // Libera un bloque del cantón por el que venía (ya entró en la estación).
                    if let Some(t) = trains.get(&ev.trip_idx) {
                        if let Some(edge) = t.holding_edge {
                            if let Some(c) = edge_busy.get_mut(&edge) {
                                *c = c.saturating_sub(1);
                                if *c == 0 {
                                    edge_busy.remove(&edge);
                                }
                            }
                            // Libera el testigo de vía única si el tramo lo era.
                            let key = crate::topology::unordered(edge.0, edge.1);
                            if let Some(tok) = st_token.get_mut(&key) {
                                tok.1 = tok.1.saturating_sub(1);
                                if tok.1 == 0 {
                                    st_token.remove(&key);
                                }
                            }
                        }
                    }

                    let arrival_delay;
                    {
                        let t = trains.get_mut(&ev.trip_idx).unwrap();
                        t.active = true;
                        t.waiting = false;
                        t.at_node = Some(node);
                        t.track = Some(track);
                        t.holding_edge = None;
                        t.delay = ev.time as i64 - (stop.arrival_sec as i64 + off(ev.trip_idx));

                        // Incidencia de retraso puntual.
                        if !t.delay_incident_done {
                            if let Some(&extra) =
                                delay_incidents.get(&(svc.train_number.clone(), node))
                            {
                                t.delay += extra as i64;
                                t.delay_incident_done = true;
                                events.push(CtcEvent {
                                    time: ev.time,
                                    train: svc.train_number.clone(),
                                    line: svc.route_short_name.clone(),
                                    kind: "INCIDÈNCIA".into(),
                                    station: format!(
                                        "[{}] Tren {} rep +{} min a {}",
                                        fmt_hms(ev.time),
                                        svc.train_number,
                                        extra / 60,
                                        net.graph[node].stop_name
                                    ),
                                    track: String::new(),
                                    delay: 0,
                                });
                            }
                        }
                        arrival_delay = t.delay;
                    }
                    total_arrivals += 1;

                    if self.is_key_station(node) {
                        events.push(CtcEvent {
                            time: ev.time,
                            train: svc.train_number.clone(),
                            line: svc.route_short_name.clone(),
                            kind: "ARRIBA".into(),
                            station: net.graph[node].stop_name.clone(),
                            track: (track + 1).to_string(),
                            delay: arrival_delay,
                        });
                    }

                    // Cálculo de dwell con el modelo de pasajeros (determinista o estocástico).
                    let theoretical = stop.departure_sec.saturating_sub(stop.arrival_sec);
                    let dwell = match rng.as_mut() {
                        Some(r) => cfg.passenger.dwell_time_random(theoretical, arrival_delay, r),
                        None => cfg.passenger.dwell_time(theoretical, arrival_delay),
                    };

                    // ¿Es la última parada? Termina el servicio.
                    if ev.stop_idx + 1 >= svc.schedule.len() {
                        // Libera andén y finaliza.
                        if let Some(slots) = node_tracks.get_mut(&node) {
                            slots[track] = None;
                        }
                        let t = trains.get_mut(&ev.trip_idx).unwrap();
                        t.finished = true;
                        t.active = false;
                        t.at_node = None;
                        continue;
                    }

                    heap.push(Reverse(Event {
                        time: ev.time + dwell,
                        trip_idx: ev.trip_idx,
                        stop_idx: ev.stop_idx,
                        kind: EventKind::Depart,
                    }));
                }

                EventKind::Depart => {
                    let stop = &svc.schedule[ev.stop_idx];
                    let next = &svc.schedule[ev.stop_idx + 1];
                    let (na, nb) = match (
                        net.node_of_stop.get(&stop.stop_id),
                        net.node_of_stop.get(&next.stop_id),
                    ) {
                        (Some(&a), Some(&b)) => (a, b),
                        _ => continue,
                    };

                    // Tiempo de marcha por el cantón (arista del grafo o fallback horario).
                    let run = net
                        .edge_between(&stop.stop_id, &next.stop_id)
                        .map(|e| e.nominal_run_secs)
                        .unwrap_or_else(|| next.arrival_sec.saturating_sub(stop.departure_sec).max(30));

                    // Capacidad efectiva del cantón: nº de bloques de señalización que
                    // caben en la sección = max(1, marcha / separación mínima).
                    let base_cap = net
                        .edge_between(&stop.stop_id, &next.stop_id)
                        .map(|e| e.capacity as u32)
                        .unwrap_or(1);
                    // Capacidad del cantón = nº de bloques de señalización que caben en la
                    // sección (marcha / separación mínima). Modela la vía múltiple: varios
                    // trenes en marcha espaciados. La exclusión real (un solo tren en
                    // cualquier sentido) la aporta el testigo de VÍA ÚNICA, más abajo.
                    let eff_cap = base_cap.max((run / cfg.min_block_headway_secs.max(1)).max(1));

                    // ¿Cantón bloqueado por incidencia?
                    if let Some(&(from, until)) = blocked.get(&(na, nb)) {
                        if ev.time >= from && ev.time < until {
                            if let Some(t) = trains.get_mut(&ev.trip_idx) {
                                if !t.waiting {
                                    held_events += 1;
                                    t.waiting = true;
                                }
                                t.delay += RETRY_STEP as i64;
                            }
                            heap.push(Reverse(Event {
                                time: ev.time + RETRY_STEP,
                                ..ev
                            }));
                            continue;
                        }
                    }

                    // Vía única: el tramo (no dirigido) debe estar libre o reservado por
                    // nuestro MISMO sentido (testigo/bastón piloto).
                    let st_key = crate::topology::unordered(na, nb);
                    let single = cfg.strict_signaling && cfg.single_track.contains(&st_key);
                    if single {
                        if let Some((dir, cnt)) = st_token.get(&st_key) {
                            if *cnt > 0 && *dir != (na, nb) {
                                // Sentido contrario ocupando el tramo → ROJO.
                                if let Some(t) = trains.get_mut(&ev.trip_idx) {
                                    if !t.waiting {
                                        held_events += 1;
                                        t.waiting = true;
                                    }
                                    t.delay += RETRY_STEP as i64;
                                }
                                heap.push(Reverse(Event { time: ev.time + RETRY_STEP, ..ev }));
                                continue;
                            }
                        }
                    }

                    // Señalización de cantón (aspecto ROJO): ¿quedan bloques libres?
                    let occ = edge_busy.get(&(na, nb)).copied().unwrap_or(0);
                    if occ >= eff_cap {
                        if let Some(t) = trains.get_mut(&ev.trip_idx) {
                            if !t.waiting {
                                held_events += 1;
                                t.waiting = true;
                            }
                            t.delay += RETRY_STEP as i64;
                        }
                        heap.push(Reverse(Event {
                            time: ev.time + RETRY_STEP,
                            ..ev
                        }));
                        continue;
                    }

                    // Reserva el testigo de vía única para nuestro sentido.
                    if single {
                        let e = st_token.entry(st_key).or_insert(((na, nb), 0));
                        e.0 = (na, nb);
                        e.1 += 1;
                    }

                    // El tren sale: libera andén, ocupa un bloque del cantón.
                    let track = trains.get(&ev.trip_idx).and_then(|t| t.track);
                    if let (Some(slots), Some(tr)) = (node_tracks.get_mut(&na), track) {
                        if tr < slots.len() {
                            slots[tr] = None;
                        }
                    }
                    *edge_busy.entry((na, nb)).or_insert(0) += 1;

                    // Aspecto de señal: verde si la andana de destino está libre,
                    // amarillo si está ocupada (ralentiza para poder frenar a temps).
                    let platform_ahead_free = node_tracks
                        .get(&nb)
                        .map(|s| s.iter().any(|x| x.is_none()))
                        .unwrap_or(true);
                    let aspect = Signals::aspect(true, platform_ahead_free);
                    let eff_run = if cfg.strict_signaling {
                        cfg.signals.traversal_time(run, aspect)
                    } else {
                        run
                    };

                    if self.is_key_station(na) {
                        let d = trains.get(&ev.trip_idx).map(|t| t.delay).unwrap_or(0);
                        let kind = if aspect == Aspect::Yellow && cfg.strict_signaling {
                            "SURT⚠"
                        } else {
                            "SURT"
                        };
                        events.push(CtcEvent {
                            time: ev.time,
                            train: svc.train_number.clone(),
                            line: svc.route_short_name.clone(),
                            kind: kind.into(),
                            station: net.graph[na].stop_name.clone(),
                            track: track.map(|t| t + 1).unwrap_or(0).to_string(),
                            delay: d,
                        });
                    }

                    if let Some(t) = trains.get_mut(&ev.trip_idx) {
                        t.at_node = None;
                        t.track = None;
                        t.holding_edge = Some((na, nb));
                        t.waiting = false;
                    }

                    heap.push(Reverse(Event {
                        time: ev.time + eff_run,
                        trip_idx: ev.trip_idx,
                        stop_idx: ev.stop_idx + 1,
                        kind: EventKind::Arrive,
                    }));
                }
            }
        }

        // Muestreo final restante.
        while next_sample <= cfg.end_sec {
            sample(next_sample, &trains, &mut timeline);
            next_sample += 60;
        }

        // --- Análisis de estabilidad ---
        // Pico de retraso acumulado global de la red.
        let (peak_total_delay, peak_time) = timeline.iter().fold(
            (0i64, cfg.start_sec),
            |(pd, pt), s| {
                if s.total_delay > pd {
                    (s.total_delay, s.time)
                } else {
                    (pd, pt)
                }
            },
        );
        let peak_delayed = timeline.iter().map(|s| s.delayed).max().unwrap_or(0);

        // Recuperación: primer instante TRAS el pico en que el retraso acumulado
        // vuelve por debajo del umbral de equilibrio (< 120 s de retraso total).
        let recovery_time = timeline
            .iter()
            .filter(|s| s.time > peak_time)
            .find(|s| s.total_delay < 120)
            .map(|s| s.time);

        SimResult {
            timeline,
            incidents,
            events,
            trains_run: participants.len(),
            total_arrivals,
            held_events,
            peak_total_delay,
            peak_time,
            peak_delayed,
            recovery_time,
        }
    }
}

/// Devuelve el conjunto de estaciones clave presentes en la red (para diagnóstico).
pub fn key_station_names(net: &Network, patterns: &[String]) -> HashSet<String> {
    let mut set = HashSet::new();
    for n in net.graph.node_weights() {
        let name = n.stop_name.to_lowercase();
        if patterns.iter().any(|p| name.contains(&p.to_lowercase())) {
            set.insert(n.stop_name.clone());
        }
    }
    set
}
