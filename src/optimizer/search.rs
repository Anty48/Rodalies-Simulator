//! Motor iterativo de optimización de horarios (Recocido Simulado / Simulated
//! Annealing) con evaluación Monte Carlo paralela (`rayon`).
//!
//! Para una línea, parte del horario GTFS base y prueba pequeñas variaciones (±N min)
//! en la hora de salida de origen de cada viaje. Cada candidato se evalúa lanzando
//! `mc_runs` simulaciones en paralelo con incidencias aleatorias en puntos críticos, y
//! se calcula el potencial medio V(H). Se aceptan los cambios que reducen V(H) (o, con
//! probabilidad decreciente, algunos que lo empeoran, para escapar de mínimos locales).

use std::collections::HashMap;
use std::sync::Arc;

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rayon::prelude::*;

use crate::gtfs_loader::Network;
use crate::optimizer::potential::{potential, PotentialWeights};
use crate::simulation_engine::{Incident, SimConfig, Simulator};

#[derive(Debug, Clone, Copy)]
pub struct SearchConfig {
    pub window: (u32, u32),
    pub iters: usize,
    pub mc_runs: usize,
    pub max_offset_min: i64,
    pub t0: f64,
    pub cooling: f64,
    pub seed: u64,
}

impl Default for SearchConfig {
    fn default() -> Self {
        SearchConfig {
            window: (6 * 3600, 10 * 3600),
            iters: 120,
            mc_runs: 6,
            max_offset_min: 5,
            t0: 6.0,
            cooling: 0.96,
            seed: 20260809,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LineOptResult {
    pub line: String,
    pub n_trips: usize,
    pub base_v: f64,
    pub best_v: f64,
    pub base_peak: f64,
    pub best_peak: f64,
    pub base_recovery_min: f64,
    pub best_recovery_min: f64,
    pub base_held: f64,
    pub best_held: f64,
    /// trip_id → offset aplicado (segundos).
    pub offsets: HashMap<String, i64>,
    pub trip_ids: Vec<String>,
}

struct Eval {
    v: f64,
    peak: f64,
    recovery_min: f64,
    held: f64,
}

/// Optimiza el horario de una línea. Devuelve `None` si no hay suficientes viajes.
pub fn optimize_line(
    net: &Network,
    line: &str,
    service_id: &str,
    sc: SearchConfig,
    w: PotentialWeights,
) -> Option<LineOptResult> {
    // Participantes: viajes de la línea en la ventana, ordenados por salida de origen.
    let mut parts: Vec<(String, u32)> = net
        .services
        .iter()
        .filter(|s| s.service_id == service_id && s.route_short_name == line)
        .filter_map(|s| {
            s.first_time().filter(|t| *t >= sc.window.0 && *t <= sc.window.1).map(|_| {
                (s.trip_id.clone(), s.schedule[0].departure_sec)
            })
        })
        .collect();
    parts.sort_by_key(|(_, t)| *t);
    if parts.len() < 4 {
        return None;
    }
    let trip_ids: Vec<String> = parts.iter().map(|(id, _)| id.clone()).collect();
    let base_origin: Vec<u32> = parts.iter().map(|(_, t)| *t).collect();

    // Conjunto FIJO de incidencias Monte Carlo (mismo para todos los candidatos, para
    // que las comparaciones de V sean justas). Retrasos de 3-12 min en puntos críticos.
    let incidents = build_mc_incidents(net, line, service_id, &sc);

    let max_off = sc.max_offset_min * 60;

    // Cierre de evaluación: energía media sobre las simulaciones Monte Carlo.
    let evaluate = |offsets_vec: &[i64]| -> Eval {
        let mut map: HashMap<String, i64> = HashMap::with_capacity(trip_ids.len());
        for (id, off) in trip_ids.iter().zip(offsets_vec) {
            if *off != 0 {
                map.insert(id.clone(), *off);
            }
        }
        let offsets = Arc::new(map);

        let mut origin: Vec<u32> = base_origin
            .iter()
            .zip(offsets_vec)
            .map(|(b, o)| (*b as i64 + *o).max(0) as u32)
            .collect();
        origin.sort_unstable();

        let evals: Vec<(f64, f64, f64, f64)> = incidents
            .par_iter()
            .enumerate()
            .map(|(k, inc)| {
                let sim = run_once(net, line, service_id, sc.window, &offsets, inc, k as u64);
                let v = potential(&origin, &sim, &w);
                let peak = sim.peak_total_delay as f64;
                let rec = match sim.recovery_time {
                    Some(t) => (t.saturating_sub(sim.peak_time)) as f64 / 60.0,
                    None => (sc.window.1.saturating_sub(sim.peak_time)) as f64 / 60.0,
                };
                (v, peak, rec, sim.held_events as f64)
            })
            .collect();
        let n = evals.len().max(1) as f64;
        Eval {
            v: evals.iter().map(|e| e.0).sum::<f64>() / n,
            peak: evals.iter().map(|e| e.1).sum::<f64>() / n,
            recovery_min: evals.iter().map(|e| e.2).sum::<f64>() / n,
            held: evals.iter().map(|e| e.3).sum::<f64>() / n,
        }
    };

    // Estado inicial: horario GTFS base (offsets 0).
    let n = trip_ids.len();
    let base = evaluate(&vec![0i64; n]);

    let mut rng = StdRng::seed_from_u64(sc.seed);
    let mut cur = vec![0i64; n];
    let mut e_cur = base.v;
    let mut best = cur.clone();
    let mut e_best = e_cur;
    let mut temp = sc.t0;

    for _ in 0..sc.iters {
        let mut cand = cur.clone();
        let j = rng.gen_range(0..n);
        let step = if rng.gen::<bool>() { 60 } else { -60 };
        cand[j] = (cand[j] + step).clamp(-max_off, max_off);
        if cand[j] == cur[j] {
            continue;
        }
        let e = evaluate(&cand).v;
        let accept = e < e_cur || rng.gen::<f64>() < ((e_cur - e) / temp).exp();
        if accept {
            cur = cand;
            e_cur = e;
            if e < e_best {
                e_best = e;
                best = cur.clone();
            }
        }
        temp *= sc.cooling;
    }

    let best_eval = evaluate(&best);
    let offsets: HashMap<String, i64> = trip_ids
        .iter()
        .cloned()
        .zip(best.iter().cloned())
        .filter(|(_, o)| *o != 0)
        .collect();

    Some(LineOptResult {
        line: line.to_string(),
        n_trips: n,
        base_v: base.v,
        best_v: best_eval.v,
        base_peak: base.peak,
        best_peak: best_eval.peak,
        base_recovery_min: base.recovery_min,
        best_recovery_min: best_eval.recovery_min,
        base_held: base.held,
        best_held: best_eval.held,
        offsets,
        trip_ids,
    })
}

fn run_once(
    net: &Network,
    line: &str,
    service_id: &str,
    window: (u32, u32),
    offsets: &Arc<HashMap<String, i64>>,
    incident: &Incident,
    seed: u64,
) -> crate::simulation_engine::SimResult {
    let mut cfg = SimConfig::default();
    cfg.start_sec = window.0;
    cfg.end_sec = window.1;
    cfg.line_filter = Some(line.to_string());
    cfg.strict_signaling = true; // block system estricto (capacitat 1 + groc/vermell)
    cfg.seed = Some(1000 + seed);
    cfg.offsets = offsets.clone();
    let mut sim = Simulator::new(net, cfg);
    sim.add_incident(incident.clone());
    sim.run(service_id)
}

/// Genera un conjunto de incidencias aleatorias en puntos críticos de la línea
/// (retrasos de 3-12 min a trenes que pasan por Clot / Arc de Triomf / Sants / Pg de
/// Gràcia); si la línea no los sirve, bloquea un cantón aleatorio de la línea.
fn build_mc_incidents(
    net: &Network,
    line: &str,
    service_id: &str,
    sc: &SearchConfig,
) -> Vec<Incident> {
    let critical = ["clot", "arc de triomf", "sants", "passeig de gràcia"];
    // Candidatos (train_number, station_name) en estaciones críticas de la línea.
    let mut cand: Vec<(String, String)> = Vec::new();
    for s in net.services.iter().filter(|s| {
        s.service_id == service_id
            && s.route_short_name == line
            && matches!(s.first_time(), Some(t) if t >= sc.window.0 && t <= sc.window.1)
    }) {
        for st in &s.schedule {
            let name = net.stop_name(&st.stop_id);
            let lname = name.to_lowercase();
            if critical.iter().any(|c| lname.contains(c)) {
                cand.push((s.train_number.clone(), name.to_string()));
            }
        }
    }
    cand.sort();
    cand.dedup();

    let mut rng = StdRng::seed_from_u64(sc.seed ^ 0xC0FFEE);
    let mut out = Vec::with_capacity(sc.mc_runs);

    // Cantones de la línea (por si no hay estaciones críticas).
    let line_edges: Vec<(String, String)> = net
        .services
        .iter()
        .filter(|s| s.service_id == service_id && s.route_short_name == line)
        .flat_map(|s| {
            s.schedule
                .windows(2)
                .map(|w| (w[0].stop_id.clone(), w[1].stop_id.clone()))
                .collect::<Vec<_>>()
        })
        .collect();

    for _ in 0..sc.mc_runs {
        let extra = rng.gen_range(3..=12) * 60;
        if !cand.is_empty() {
            let (tn, sta) = &cand[rng.gen_range(0..cand.len())];
            out.push(Incident::TrainDelay {
                train_number: tn.clone(),
                at_stop_name: sta.clone(),
                extra_secs: extra,
            });
        } else if !line_edges.is_empty() {
            let (a, b) = &line_edges[rng.gen_range(0..line_edges.len())];
            out.push(Incident::BlockSegmentById {
                from_stop_id: a.clone(),
                to_stop_id: b.clone(),
                from_sec: sc.window.0 + 20 * 60,
                dur_secs: extra,
            });
        }
    }
    if out.is_empty() {
        // Sin incidencias posibles: al menos una simulación limpia.
        out.push(Incident::TrainDelay {
            train_number: "—".into(),
            at_stop_name: "—".into(),
            extra_secs: 0,
        });
    }
    out
}
