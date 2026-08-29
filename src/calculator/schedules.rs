//! Horarios: tiempos de **parada** (dwell) y de **marcha programada** extraídos del
//! GTFS. Separa el tratamiento de horarios de la física y de la infraestructura.
//!
//! Fuente única: GTFS oficial de Rodalies/Cercanías (`data/gtfs`, `stop_times.txt` con
//! `arrival_time`/`departure_time`). El GTFS SÍ trae ambos, así que el dwell real por
//! estación = `departure − arrival` es un dato oficial (no inventado). No hay dataset
//! abierto de circulación OBSERVADA (hora real) de Renfe/Adif, así que aquí sólo se
//! trabaja con horario PROGRAMADO; la categoría "observado real" queda documentada como
//! no disponible en la capa de análisis.

use std::collections::HashMap;

use serde::Serialize;

use crate::gtfs_loader::{Network, TrainService};

/// Un sentido de circulación de una línea, identificado por sus extremos.
#[derive(Debug, Clone, Serialize)]
pub struct Direction {
    /// Clave interna `(primer_stop_id, último_stop_id)`.
    pub key: (String, String),
    pub label: String,
    pub canonical_trip: String,
    pub n_services: usize,
}

/// Una parada del itinerario canónico (servicio con más paradas del sentido).
#[derive(Debug, Clone, Serialize)]
pub struct ItinStop {
    pub stop_id: String,
    pub name: String,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    /// Horario de referencia del servicio canónico (s desde medianoche).
    pub arr: u32,
    pub dep: u32,
}

/// Itinerario canónico de un sentido: orden de estaciones + horario de referencia.
#[derive(Debug, Clone, Serialize)]
pub struct Itinerary {
    pub dir: Direction,
    pub stops: Vec<ItinStop>,
}

/// Estadística de dwell (parada) por estación, agregada sobre los servicios del sentido.
#[derive(Debug, Clone, Serialize, Default)]
pub struct DwellStat {
    pub median_s: u32,
    pub n: usize,
    pub min_s: u32,
    pub max_s: u32,
}

/// Estadística de un conjunto de duraciones (s).
#[derive(Debug, Clone, Serialize, Default)]
pub struct Stat {
    pub n: usize,
    pub min: u32,
    pub max: u32,
    pub mean: u32,
    pub median: u32,
    pub p10: u32,
    pub p90: u32,
}

impl Stat {
    fn from(mut v: Vec<u32>) -> Stat {
        if v.is_empty() {
            return Stat::default();
        }
        v.sort_unstable();
        let n = v.len();
        let sum: u64 = v.iter().map(|&x| x as u64).sum();
        let pct = |p: f64| -> u32 {
            let idx = ((n as f64 - 1.0) * p).round() as usize;
            v[idx.min(n - 1)]
        };
        Stat {
            n,
            min: v[0],
            max: v[n - 1],
            mean: (sum / n as u64) as u32,
            median: pct(0.5),
            p10: pct(0.10),
            p90: pct(0.90),
        }
    }
}

/// Servicios de tren (no bus) de una línea.
fn line_services<'a>(net: &'a Network, line: &str) -> Vec<&'a TrainService> {
    net.services
        .iter()
        .filter(|s| !s.is_bus && s.route_short_name == line && s.schedule.len() >= 2)
        .collect()
}

/// Detecta los sentidos de una línea agrupando por `(primer, último)` stop_id. Devuelve
/// los sentidos ordenados por nº de servicios (descendente). El itinerario canónico de
/// cada sentido es el servicio con MÁS paradas (variante que para en todas).
pub fn line_directions(net: &Network, line: &str) -> Vec<Direction> {
    let svcs = line_services(net, line);
    // Agrupar por extremos.
    let mut groups: HashMap<(String, String), Vec<&TrainService>> = HashMap::new();
    for s in &svcs {
        let first = s.schedule.first().unwrap().stop_id.clone();
        let last = s.schedule.last().unwrap().stop_id.clone();
        groups.entry((first, last)).or_default().push(s);
    }
    let mut dirs: Vec<Direction> = groups
        .into_iter()
        .map(|(key, list)| {
            let canonical = list.iter().max_by_key(|s| s.schedule.len()).unwrap();
            let label = format!(
                "{} → {}",
                net.stop_name(&key.0),
                net.stop_name(&key.1)
            );
            Direction {
                key,
                label,
                canonical_trip: canonical.trip_id.clone(),
                n_services: list.len(),
            }
        })
        .collect();
    dirs.sort_by(|a, b| b.n_services.cmp(&a.n_services));
    dirs
}

/// Construye el itinerario canónico de un sentido (por su `key`).
pub fn itinerary(net: &Network, line: &str, key: &(String, String)) -> Option<Itinerary> {
    let dirs = line_directions(net, line);
    let dir = dirs.into_iter().find(|d| &d.key == key)?;
    let svc = net.services.iter().find(|s| s.trip_id == dir.canonical_trip)?;
    let stops = svc
        .schedule
        .iter()
        .map(|s| {
            let node = net.node(&s.stop_id).map(|n| &net.graph[n]);
            ItinStop {
                stop_id: s.stop_id.clone(),
                name: net.stop_name(&s.stop_id).to_string(),
                lat: node.and_then(|n| n.lat),
                lon: node.and_then(|n| n.lon),
                arr: s.arrival_sec,
                dep: s.departure_sec,
            }
        })
        .collect();
    Some(Itinerary { dir, stops })
}

/// Servicios del sentido `key` que recorren el itinerario **completo** (para comparar de
/// forma justa con el modelo que para en todas las estaciones): mismos extremos y al
/// menos el 80 % de las paradas del canónico.
fn full_itinerary_services<'a>(
    net: &'a Network,
    line: &str,
    key: &(String, String),
    canonical_len: usize,
) -> Vec<&'a TrainService> {
    let min_stops = (canonical_len as f64 * 0.8).ceil() as usize;
    line_services(net, line)
        .into_iter()
        .filter(|s| {
            s.schedule.first().unwrap().stop_id == key.0
                && s.schedule.last().unwrap().stop_id == key.1
                && s.schedule.len() >= min_stops
        })
        .collect()
}

/// Dwell (parada) real por estación agregado sobre los servicios de itinerario completo.
/// Devuelve un mapa `stop_id -> DwellStat` (mediana de `dep − arr`).
pub fn dwell_by_station(
    net: &Network,
    line: &str,
    key: &(String, String),
    canonical_len: usize,
) -> HashMap<String, DwellStat> {
    let svcs = full_itinerary_services(net, line, key, canonical_len);
    let mut acc: HashMap<String, Vec<u32>> = HashMap::new();
    for s in &svcs {
        for st in &s.schedule {
            let d = st.departure_sec.saturating_sub(st.arrival_sec);
            acc.entry(st.stop_id.clone()).or_default().push(d);
        }
    }
    acc.into_iter()
        .map(|(id, mut v)| {
            v.sort_unstable();
            let n = v.len();
            let stat = DwellStat {
                median_s: v[n / 2],
                n,
                min_s: v[0],
                max_s: v[n - 1],
            };
            (id, stat)
        })
        .collect()
}

/// Estadística del tiempo total PROGRAMADO (última llegada − primera salida) sobre los
/// servicios de itinerario completo del sentido.
pub fn programmed_total(
    net: &Network,
    line: &str,
    key: &(String, String),
    canonical_len: usize,
) -> Stat {
    let svcs = full_itinerary_services(net, line, key, canonical_len);
    let durations: Vec<u32> = svcs
        .iter()
        .filter_map(|s| {
            let dep = s.schedule.first()?.departure_sec;
            let arr = s.schedule.last()?.arrival_sec;
            arr.checked_sub(dep)
        })
        .filter(|&d| d > 0)
        .collect();
    Stat::from(durations)
}

/// Tiempo programado ACUMULADO (s) desde la salida del origen hasta la llegada a cada
/// estación, según el horario del servicio canónico (timetable de referencia).
pub fn programmed_cumulative(net: &Network, canonical_trip: &str) -> Vec<(String, u32)> {
    let Some(svc) = net.services.iter().find(|s| s.trip_id == canonical_trip) else {
        return Vec::new();
    };
    let Some(t0) = svc.schedule.first().map(|s| s.departure_sec) else {
        return Vec::new();
    };
    svc.schedule
        .iter()
        .map(|s| (s.stop_id.clone(), s.arrival_sec.saturating_sub(t0)))
        .collect()
}

// --------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_percentiles_basicos() {
        let s = Stat::from(vec![10, 20, 30, 40, 50]);
        assert_eq!(s.n, 5);
        assert_eq!(s.min, 10);
        assert_eq!(s.max, 50);
        assert_eq!(s.median, 30);
        assert_eq!(s.mean, 30);
    }

    #[test]
    fn stat_vacia_es_cero() {
        let s = Stat::from(vec![]);
        assert_eq!(s.n, 0);
        assert_eq!(s.median, 0);
    }
}
