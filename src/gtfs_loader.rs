//! Carga 100% dinámica de la infraestructura Rodalies a partir de archivos GTFS.
//!
//! Lee `stops.txt`, `routes.txt`, `trips.txt` y `stop_times.txt` desde una carpeta
//! (por defecto `./data/gtfs`) y construye:
//!   * Un grafo dirigido de `petgraph` donde cada parada/andén es un nodo.
//!   * Aristas dirigidas (cantones) entre paradas consecutivas de cada `trip`.
//!   * La lista de servicios de tren (`TrainService`) con su horario ordenado.
//!
//! El feed real de Renfe/ADIF viene con campos rellenados con espacios (ancho fijo)
//! y a veces sin columnas opcionales (`parent_station`, `trip_short_name`); por eso
//! todo se lee con `Trim::All` y los campos opcionales usan `#[serde(default)]`.

use std::collections::HashMap;
use std::error::Error;
use std::path::Path;

use petgraph::graph::{DiGraph, EdgeIndex, NodeIndex};
use serde::Deserialize;

// --------------------------------------------------------------------------
// Registros crudos del GTFS (mapeados por nombre de columna)
// --------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RawStop {
    stop_id: String,
    stop_name: String,
    #[serde(default)]
    parent_station: Option<String>,
    #[serde(default)]
    stop_lat: Option<String>,
    #[serde(default)]
    stop_lon: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawRoute {
    route_id: String,
    #[serde(default)]
    route_short_name: String,
    #[serde(default)]
    route_long_name: Option<String>,
    /// GTFS route_type: 2 = tren (rail), 3 = autobús (substitució per obres).
    #[serde(default)]
    route_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawTrip {
    route_id: String,
    service_id: String,
    trip_id: String,
    /// El feed de Renfe normalmente NO trae `trip_short_name`; queda como `None`.
    #[serde(default)]
    trip_short_name: Option<String>,
    #[serde(default)]
    trip_headsign: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawStopTime {
    trip_id: String,
    arrival_time: String,
    departure_time: String,
    stop_id: String,
    stop_sequence: u32,
}

// --------------------------------------------------------------------------
// Modelo de dominio
// --------------------------------------------------------------------------

/// Nodo del grafo: una parada física (estación/andén/vía).
#[derive(Debug, Clone)]
pub struct StopNode {
    pub stop_id: String,
    pub stop_name: String,
    pub parent_station: Option<String>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
}

/// Arista dirigida = cantón/sección de vía entre dos paradas consecutivas.
#[derive(Debug, Clone)]
pub struct TrackEdge {
    pub from_stop: String,
    pub to_stop: String,
    /// Tiempo de marcha nominal: `arrival_next - departure_current` (segundos).
    pub nominal_run_secs: u32,
    /// Capacidad del cantón: por defecto 1 tren simultáneo por sección.
    pub capacity: u8,
}

/// Una parada dentro del horario de un servicio.
#[derive(Debug, Clone)]
pub struct ScheduledStop {
    pub stop_id: String,
    pub arrival_sec: u32,
    pub departure_sec: u32,
    pub seq: u32,
}

/// Un servicio de tren (una circulación concreta).
#[derive(Debug, Clone)]
pub struct TrainService {
    pub trip_id: String,
    /// Número de circulación oficial de Renfe (`trip_short_name`) o `trip_id` si falta.
    pub train_number: String,
    pub route_id: String,
    pub route_short_name: String,
    pub service_id: String,
    pub headsign: Option<String>,
    pub schedule: Vec<ScheduledStop>,
    /// `true` si el servicio es un autobús (route_type 3), p.ej. sustitución por obras.
    pub is_bus: bool,
}

impl TrainService {
    /// Primer instante en el que el tren aparece en la red (segundos desde medianoche).
    pub fn first_time(&self) -> Option<u32> {
        self.schedule.first().map(|s| s.arrival_sec)
    }
}

/// La red completa: grafo de infraestructura + servicios + índices auxiliares.
pub struct Network {
    pub graph: DiGraph<StopNode, TrackEdge>,
    /// stop_id -> índice del nodo en el grafo.
    pub node_of_stop: HashMap<String, NodeIndex>,
    /// route_id -> route_short_name (R1, R2N, R4, ...).
    pub routes: HashMap<String, String>,
    pub services: Vec<TrainService>,
}

impl Network {
    pub fn stop_name(&self, stop_id: &str) -> &str {
        self.node_of_stop
            .get(stop_id)
            .map(|&n| self.graph[n].stop_name.as_str())
            .unwrap_or("<desconocida>")
    }

    /// Devuelve el índice de nodo de una parada.
    pub fn node(&self, stop_id: &str) -> Option<NodeIndex> {
        self.node_of_stop.get(stop_id).copied()
    }

    /// Peso (cantón) entre dos paradas si existe la arista.
    pub fn edge_between(&self, from: &str, to: &str) -> Option<&TrackEdge> {
        let a = self.node(from)?;
        let b = self.node(to)?;
        let e = self.graph.find_edge(a, b)?;
        self.graph.edge_weight(e)
    }

    /// Busca el primer nodo cuyo nombre contenga (sin distinguir mayúsculas) el patrón.
    pub fn find_stop_by_name(&self, needle: &str) -> Option<&StopNode> {
        let needle = needle.to_lowercase();
        self.graph
            .node_weights()
            .find(|n| n.stop_name.to_lowercase().contains(&needle))
    }

    /// service_id con más circulaciones (día tipo dominante, p.ej. laborable).
    pub fn dominant_service(&self) -> Option<String> {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for s in &self.services {
            *counts.entry(s.service_id.as_str()).or_insert(0) += 1;
        }
        counts
            .into_iter()
            .max_by_key(|(_, c)| *c)
            .map(|(sid, _)| sid.to_string())
    }

    /// Segmento (par de paradas consecutivas) más transitado por los servicios de
    /// `service_id` que arrancan en [start, end], opcionalmente restringido a una
    /// línea. Útil para inyectar un bloqueo con impacto garantizado.
    /// Devuelve `(from_stop_id, to_stop_id, nº de circulaciones)`.
    pub fn busiest_segment_filtered(
        &self,
        service_id: &str,
        start: u32,
        end: u32,
        line: Option<&str>,
    ) -> Option<(String, String, usize)> {
        let mut counts: HashMap<(&str, &str), usize> = HashMap::new();
        for svc in &self.services {
            if svc.service_id != service_id {
                continue;
            }
            if let Some(l) = line {
                if svc.route_short_name != l {
                    continue;
                }
            }
            match svc.first_time() {
                Some(t) if t >= start && t <= end => {}
                _ => continue,
            }
            for w in svc.schedule.windows(2) {
                *counts
                    .entry((w[0].stop_id.as_str(), w[1].stop_id.as_str()))
                    .or_insert(0) += 1;
            }
        }
        counts
            .into_iter()
            .max_by_key(|(_, c)| *c)
            .map(|((a, b), c)| (a.to_string(), b.to_string(), c))
    }

    /// Asignación determinista de vía/andén para mostrar en informes estáticos.
    /// (El GTFS no trae andenes, así que se deriva de forma reproducible.)
    pub fn assigned_track(&self, stop_id: &str, seq: u32, platform_capacity: u32) -> u32 {
        let base: u32 = stop_id.bytes().map(|b| b as u32).sum();
        1 + ((base.wrapping_add(seq)) % platform_capacity.max(1))
    }
}

// --------------------------------------------------------------------------
// Utilidades de tiempo
// --------------------------------------------------------------------------

/// Convierte `HH:MM:SS` (con HH posiblemente > 24 para servicios de madrugada)
/// a segundos transcurridos desde la medianoche.
pub fn parse_gtfs_time(s: &str) -> Option<u32> {
    let mut it = s.trim().split(':');
    let h: u32 = it.next()?.trim().parse().ok()?;
    let m: u32 = it.next()?.trim().parse().ok()?;
    let sec: u32 = it.next()?.trim().parse().ok()?;
    if m >= 60 || sec >= 60 {
        return None;
    }
    Some(h * 3600 + m * 60 + sec)
}

/// Formatea segundos-desde-medianoche como `HH:MM:SS` (soporta > 24h).
pub fn fmt_hms(secs: u32) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}

// --------------------------------------------------------------------------
// Carga
// --------------------------------------------------------------------------

fn reader(path: &Path) -> Result<csv::Reader<std::fs::File>, Box<dyn Error>> {
    let rdr = csv::ReaderBuilder::new()
        .trim(csv::Trim::All) // quita relleno de ancho fijo y CR de CRLF
        .flexible(true)
        .from_path(path)?;
    Ok(rdr)
}

/// Carga la red completa desde una carpeta GTFS.
pub fn load(dir: &Path) -> Result<Network, Box<dyn Error>> {
    // 1. routes.txt -> route_id -> short_name (+ route_type para distinguir autobuses)
    let mut routes: HashMap<String, String> = HashMap::new();
    let mut route_is_bus: std::collections::HashSet<String> = std::collections::HashSet::new();
    {
        let mut rdr = reader(&dir.join("routes.txt"))?;
        for rec in rdr.deserialize() {
            let r: RawRoute = rec?;
            let short = if r.route_short_name.is_empty() {
                r.route_long_name.clone().unwrap_or_default()
            } else {
                r.route_short_name.clone()
            };
            let rid = r.route_id.trim().to_string();
            if r.route_type.as_deref().map(|t| t.trim()) == Some("3") {
                route_is_bus.insert(rid.clone());
            }
            routes.insert(rid, short.trim().to_string());
        }
    }

    // 2. stops.txt -> nodos del grafo
    let mut graph: DiGraph<StopNode, TrackEdge> = DiGraph::new();
    let mut node_of_stop: HashMap<String, NodeIndex> = HashMap::new();
    {
        let mut rdr = reader(&dir.join("stops.txt"))?;
        for rec in rdr.deserialize() {
            let s: RawStop = rec?;
            let stop_id = s.stop_id.trim().to_string();
            if stop_id.is_empty() {
                continue;
            }
            let node = graph.add_node(StopNode {
                stop_id: stop_id.clone(),
                stop_name: s.stop_name.trim().to_string(),
                parent_station: s
                    .parent_station
                    .map(|p| p.trim().to_string())
                    .filter(|p| !p.is_empty()),
                lat: s.stop_lat.and_then(|v| v.trim().parse().ok()),
                lon: s.stop_lon.and_then(|v| v.trim().parse().ok()),
            });
            node_of_stop.insert(stop_id, node);
        }
    }

    // 3. trips.txt -> metadatos por trip_id
    struct TripMeta {
        route_id: String,
        service_id: String,
        train_number: String,
        headsign: Option<String>,
    }
    let mut trips: HashMap<String, TripMeta> = HashMap::new();
    {
        let mut rdr = reader(&dir.join("trips.txt"))?;
        for rec in rdr.deserialize() {
            let t: RawTrip = rec?;
            let trip_id = t.trip_id.trim().to_string();
            let train_number = t
                .trip_short_name
                .map(|n| n.trim().to_string())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| trip_id.clone());
            trips.insert(
                trip_id,
                TripMeta {
                    route_id: t.route_id.trim().to_string(),
                    service_id: t.service_id.trim().to_string(),
                    train_number,
                    headsign: t
                        .trip_headsign
                        .map(|h| h.trim().to_string())
                        .filter(|h| !h.is_empty()),
                },
            );
        }
    }

    // 4. stop_times.txt -> horarios agrupados por trip_id
    let mut sched: HashMap<String, Vec<ScheduledStop>> = HashMap::new();
    {
        let mut rdr = reader(&dir.join("stop_times.txt"))?;
        for rec in rdr.deserialize() {
            let st: RawStopTime = rec?;
            let trip_id = st.trip_id.trim().to_string();
            let stop_id = st.stop_id.trim().to_string();
            if !node_of_stop.contains_key(&stop_id) {
                continue;
            }
            let (Some(arr), Some(dep)) =
                (parse_gtfs_time(&st.arrival_time), parse_gtfs_time(&st.departure_time))
            else {
                continue;
            };
            sched.entry(trip_id).or_default().push(ScheduledStop {
                stop_id,
                arrival_sec: arr,
                departure_sec: dep,
                seq: st.stop_sequence,
            });
        }
    }

    // 5. Ensamblar servicios y 6. construir aristas (cantones) de forma dinámica.
    let mut services: Vec<TrainService> = Vec::new();
    // Dedupe de aristas + quedarse con el menor tiempo de marcha observado.
    let mut edge_index: HashMap<(NodeIndex, NodeIndex), EdgeIndex> = HashMap::new();

    for (trip_id, mut stops) in sched {
        if stops.len() < 2 {
            continue;
        }
        stops.sort_by_key(|s| s.seq);

        let meta = trips.get(&trip_id);
        let route_id = meta.map(|m| m.route_id.clone()).unwrap_or_default();
        let route_short = routes.get(&route_id).cloned().unwrap_or_else(|| route_id.clone());

        // Construcción dinámica de aristas entre paradas consecutivas.
        for w in stops.windows(2) {
            let (a, b) = (&w[0], &w[1]);
            let (Some(&na), Some(&nb)) =
                (node_of_stop.get(&a.stop_id), node_of_stop.get(&b.stop_id))
            else {
                continue;
            };
            if na == nb {
                continue;
            }
            let run = b.arrival_sec.saturating_sub(a.departure_sec).max(30);
            match edge_index.get(&(na, nb)) {
                Some(&ei) => {
                    // Nos quedamos con el tiempo de marcha nominal mínimo.
                    let w = graph.edge_weight_mut(ei).unwrap();
                    if run < w.nominal_run_secs {
                        w.nominal_run_secs = run;
                    }
                }
                None => {
                    let ei = graph.add_edge(
                        na,
                        nb,
                        TrackEdge {
                            from_stop: a.stop_id.clone(),
                            to_stop: b.stop_id.clone(),
                            nominal_run_secs: run,
                            capacity: 1,
                        },
                    );
                    edge_index.insert((na, nb), ei);
                }
            }
        }

        let train_number = meta.map(|m| m.train_number.clone()).unwrap_or_else(|| trip_id.clone());
        let service_id = meta.map(|m| m.service_id.clone()).unwrap_or_default();
        let headsign = meta.and_then(|m| m.headsign.clone());

        let is_bus = route_is_bus.contains(&route_id);
        services.push(TrainService {
            trip_id,
            train_number,
            route_id,
            route_short_name: route_short,
            service_id,
            headsign,
            schedule: stops,
            is_bus,
        });
    }

    // Orden estable por hora de salida para informes reproducibles.
    services.sort_by(|a, b| {
        a.first_time()
            .cmp(&b.first_time())
            .then_with(|| a.trip_id.cmp(&b.trip_id))
    });

    Ok(Network {
        graph,
        node_of_stop,
        routes,
        services,
    })
}
