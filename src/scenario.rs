//! Construcción de las "vistas" del dashboard a partir de parámetros de simulación.
//! Sin efectos de consola: lo usan tanto el ejecutable (modo consola) como el
//! servidor web interactivo.

use rayon::prelude::*;

use crate::calculator::infrastructure::AdifNet;
use crate::calculator::ltv::LtvSet;
use crate::engine::{default_templates, Engine, EngineCfg, IncidentEffect};
use crate::gtfs_loader::{fmt_hms, Network};
use crate::report::{ExampleView, ResRow, ResView, SimView, StopRow, SummaryView};

/// Parámetros ajustables desde la UI del panel (`/panel`). Los antiguos knobs de capacidad de
/// andén / headway / tasa de llegada se retiraron al migrar al **motor unificado** (`engine.rs`),
/// que los DERIVA de datos reales (topología ADIF + perfil de demanda) — no son ajustables.
#[derive(Debug, Clone)]
pub struct SimParams {
    pub start_sec: u32,
    pub end_sec: u32,
    /// `None` = todas las líneas.
    pub line: Option<String>,
    /// Minutos de bloqueo del cantón más cargado (señal en rojo, ambos sentidos). 0 = sin bloqueo.
    pub block_min: u32,
    /// Minutos de avería de señal (un sentido) en el cantón más cargado. 0 = sin avería.
    pub delay_min: u32,
    /// `true` = añade además las incidencias probabilísticas del sistema (por plantilla).
    pub random: bool,
}

impl Default for SimParams {
    fn default() -> Self {
        SimParams {
            start_sec: 7 * 3600,
            end_sec: 9 * 3600,
            line: None,
            block_min: 12,
            delay_min: 8,
            random: false,
        }
    }
}

/// Fecha/hora UTC actual como texto (sin dependencias externas).
pub fn now_utc_string() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86400) as i64;
    let tod = secs % 86400;
    let (y, m, d) = civil_from_days(days);
    format!("{:04}-{:02}-{:02} {:02}:{:02} UTC", y, m, d, tod / 3600, (tod % 3600) / 60)
}

/// Algoritmo de Howard Hinnant: días desde epoch → (año, mes, día) civil.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Líneas presentes (route_short_name) ordenadas por nº de servicios desc.
pub fn distinct_lines(net: &Network) -> Vec<String> {
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for s in &net.services {
        *counts.entry(s.route_short_name.as_str()).or_insert(0) += 1;
    }
    // Líneas de tren definidas en routes.txt sin ningún viaje activo ahora mismo (p. ej. una
    // supresión temporal por obras): la línea existe igualmente, solo con 0 servicios.
    for l in &net.rail_lines {
        counts.entry(l.as_str()).or_insert(0);
    }
    let mut v: Vec<(String, usize)> =
        counts.into_iter().map(|(k, c)| (k.to_string(), c)).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v.into_iter().map(|(k, _)| k).collect()
}

pub fn summary_view(net: &Network, load_ms: f64) -> SummaryView {
    let with_parent = net
        .graph
        .node_weights()
        .filter(|n| n.parent_station.is_some())
        .count();
    let fastest_edge = net
        .graph
        .edge_weights()
        .min_by_key(|e| e.nominal_run_secs)
        .map(|e| {
            (
                net.stop_name(&e.from_stop).to_string(),
                net.stop_name(&e.to_stop).to_string(),
                e.nominal_run_secs,
            )
        });
    let mut by_route: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for s in &net.services {
        *by_route.entry(s.route_short_name.as_str()).or_insert(0) += 1;
    }
    let mut per_line: Vec<(String, usize)> =
        by_route.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    per_line.sort_by(|a, b| b.1.cmp(&a.1));
    per_line.truncate(10);

    SummaryView {
        load_ms,
        n_stops: net.graph.node_count(),
        n_edges: net.graph.edge_count(),
        n_services: net.services.len(),
        n_routes: net.routes.len(),
        with_parent,
        per_line,
        fastest_edge,
    }
}

/// Elige un servicio de ejemplo: el número exacto si existe; si hay línea filtrada,
/// el más largo de esa línea; si no, el más largo que pase por Clot; si no, el más largo.
pub fn example_view(net: &Network, wanted: &str, line: Option<&str>) -> Option<ExampleView> {
    let svc = net
        .services
        .iter()
        .find(|s| s.train_number == wanted)
        .or_else(|| {
            if let Some(l) = line {
                net.services
                    .iter()
                    .filter(|s| s.route_short_name == l)
                    .max_by_key(|s| s.schedule.len())
            } else {
                None
            }
        })
        .or_else(|| {
            net.services
                .iter()
                .filter(|s| {
                    s.schedule
                        .iter()
                        .any(|st| net.stop_name(&st.stop_id).to_lowercase().contains("clot"))
                })
                .max_by_key(|s| s.schedule.len())
        })
        .or_else(|| net.services.iter().max_by_key(|s| s.schedule.len()))?;

    let cap = 2;
    let stops: Vec<StopRow> = svc
        .schedule
        .iter()
        .enumerate()
        .map(|(i, st)| {
            let run = if i + 1 < svc.schedule.len() {
                let nx = &svc.schedule[i + 1];
                net.edge_between(&st.stop_id, &nx.stop_id)
                    .map(|e| e.nominal_run_secs)
                    .unwrap_or_else(|| nx.arrival_sec.saturating_sub(st.departure_sec))
            } else {
                0
            };
            StopRow {
                seq: st.seq,
                name: net.stop_name(&st.stop_id).to_string(),
                arr: st.arrival_sec,
                dep: st.departure_sec,
                run_secs: run,
                track: net.assigned_track(&st.stop_id, st.seq, cap),
            }
        })
        .collect();

    Some(ExampleView {
        found: svc.train_number == wanted,
        wanted: wanted.to_string(),
        train_number: svc.train_number.clone(),
        route_short: svc.route_short_name.clone(),
        route_id: svc.route_id.clone(),
        trip_id: svc.trip_id.clone(),
        headsign: svc.headsign.clone().unwrap_or_default(),
        stops,
    })
}

/// Estaciones-nudo clave de la red (para los chips del panel), por coincidencia de nombre.
fn key_station_list(net: &Network) -> Vec<String> {
    const PATS: &[&str] = &[
        "barcelona-sants", "passeig de gràcia", "el clot", "arc de triomf",
        "plaça de catalunya", "barcelona nord", "l'hospitalet",
    ];
    let mut set: std::collections::BTreeSet<String> = Default::default();
    for n in net.graph.node_weights() {
        let name = net.stop_name(&n.stop_id);
        let lname = name.to_lowercase();
        if PATS.iter().any(|p| lname.contains(p)) {
            set.insert(name.to_string());
        }
    }
    set.into_iter().collect()
}

/// Configura el motor unificado para el panel (ventana + filtro de línea + semilla). `random`
/// añade las incidencias probabilísticas del sistema; si no, solo se aplican las inyectadas.
fn engine_for(
    net: &Network,
    adif: Option<&AdifNet>,
    ltv: Option<&LtvSet>,
    p: &SimParams,
    seed: u64,
) -> Engine {
    let mut cfg = EngineCfg::day();
    cfg.start_sec = p.start_sec;
    cfg.end_sec = p.end_sec;
    cfg.seed = seed;
    let templates = if p.random { default_templates() } else { Vec::new() };
    Engine::new(net, cfg, adif, ltv, &templates, None, p.line.as_deref())
}

pub fn sim_view(net: &Network, adif: Option<&AdifNet>, ltv: Option<&LtvSet>, p: &SimParams) -> SimView {
    let service_id = net.dominant_service().unwrap_or_default();
    let window = format!("{}–{}", fmt_hms(p.start_sec), fmt_hms(p.end_sec));
    let key_stations = key_station_list(net);

    let busiest_ids =
        net.busiest_segment_filtered(&service_id, p.start_sec, p.end_sec, p.line.as_deref());
    let busiest = busiest_ids
        .as_ref()
        .map(|(f, t, c)| (net.stop_name(f).to_string(), net.stop_name(t).to_string(), *c));

    let mut eng = engine_for(net, adif, ltv, p, 42);

    // Incidencias inyectadas según los knobs del panel, sobre el cantón más cargado.
    if let Some((from, to, _)) = &busiest_ids {
        if p.block_min > 0 {
            eng.inject_incident(
                from.clone(), to.clone(),
                IncidentEffect::RedSignal { both: true },
                (p.start_sec + 20 * 60) as f64, p.block_min as f64,
                "Bloqueig de cantó (2 sentits)".into(),
            );
        }
        if p.delay_min > 0 {
            eng.inject_incident(
                from.clone(), to.clone(),
                IncidentEffect::RedSignal { both: false },
                (p.start_sec + 10 * 60) as f64, p.delay_min as f64,
                "Avaria de senyalització (1 sentit)".into(),
            );
        }
    }

    // Descripción legible de todas las incidencias vigentes (inyectadas + rodadas si `random`).
    let incidents: Vec<String> = eng
        .incidents()
        .iter()
        .map(|inc| {
            format!(
                "{} · {} → {} · {}–{}",
                inc.motivo,
                net.stop_name(&inc.from),
                net.stop_name(&inc.to),
                fmt_hms(inc.t0 as u32),
                fmt_hms(inc.t1 as u32),
            )
        })
        .collect();

    let m = eng.run();
    let timeline = eng.timeline().to_vec();
    let recovery = m.recovery_time.map(|t| {
        format!("{} (+{} min del pic)", fmt_hms(t), t.saturating_sub(m.peak_time) / 60)
    });

    SimView {
        window,
        service_id,
        key_stations,
        busiest,
        incidents,
        trains_run: m.n_trains,
        held: m.held_events as usize,
        peak_total: m.peak_total_delay_s as i64,
        peak_time: fmt_hms(m.peak_time),
        recovery,
        timeline,
        map_svg: crate::map::network_map_svg(net, p),
    }
}

pub fn res_view(net: &Network, adif: Option<&AdifNet>, ltv: Option<&LtvSet>, p: &SimParams) -> ResView {
    let service_id = net.dominant_service().unwrap_or_default();
    let Some((from, to, _)) =
        net.busiest_segment_filtered(&service_id, p.start_sec, p.end_sec, p.line.as_deref())
    else {
        return ResView { segment: "—".into(), rows: Vec::new() };
    };
    let segment = format!(
        "{} → {} a les {}",
        net.stop_name(&from),
        net.stop_name(&to),
        fmt_hms(p.start_sec + 20 * 60)
    );

    // Barrido de duraciones de bloqueo (Monte-Carlo por escenario, en paralelo con rayon).
    // Cada fila reconstruye el motor unificado (reperfila ADIF) → pocas duraciones para la UI.
    let durations: [u32; 4] = [0, 4, 8, 12];
    let mut rows: Vec<ResRow> = durations
        .par_iter()
        .map(|&mins| {
            let mut eng = engine_for(net, adif, ltv, p, 1000 + mins as u64);
            if mins > 0 {
                eng.inject_incident(
                    from.clone(), to.clone(),
                    IncidentEffect::RedSignal { both: true },
                    (p.start_sec + 20 * 60) as f64, mins as f64,
                    "Bloqueig de cantó".into(),
                );
            }
            let r = eng.run();
            ResRow {
                mins,
                peak: r.peak_total_delay_s as i64,
                recovery: r.recovery_time.map(|t| t.saturating_sub(r.peak_time) / 60),
                held: r.held_events as usize,
            }
        })
        .collect();
    rows.sort_by_key(|r| r.mins);
    ResView { segment, rows }
}
