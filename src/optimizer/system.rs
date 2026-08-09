//! Optimización del SISTEMA ENTERO (todas las líneas a la vez).
//!
//! El problema real no es una línea aislada, sino los **conflictos entre líneas** en los
//! cantones compartidos (p.ej. el tronco Sants–Passeig lo usan 8-9 líneas) y cómo el
//! sistema responde a las **incidencias**. Un buen juego de horarios hace que, ante una
//! incidencia, el sistema tienda a la estabilidad en vez de al caos.
//!
//! Se optimiza un **desfase de fase por línea** (±5 min) para todo un día laborable
//! (05:00–00:00). Cada candidato se evalúa con varias simulaciones Monte Carlo del
//! sistema completo, cada una con incidencias aleatorias repartidas por el día, en
//! paralelo (`rayon`). Se minimiza el potencial V (retraso ponderado por pasajeros +
//! penalización por conflicto), integrado sobre todo el día.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use petgraph::graph::NodeIndex;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rayon::prelude::*;

use crate::gtfs_loader::Network;
use crate::optimizer::potential::{passenger_weighted_delay, PotentialWeights};
use crate::simulation_engine::{Incident, SimConfig, Simulator};

#[derive(Debug, Clone, Copy)]
pub struct SystemSearch {
    pub window: (u32, u32),
    pub iters: usize,
    pub mc_runs: usize,
    pub incidents_per_run: usize,
    pub max_offset_min: i64,
    pub min_block_headway: u32,
    pub t0: f64,
    pub cooling: f64,
    pub seed: u64,
}

impl Default for SystemSearch {
    fn default() -> Self {
        SystemSearch {
            window: (5 * 3600, 24 * 3600), // 05:00 – 00:00
            iters: 200,
            mc_runs: 8,
            incidents_per_run: 4,
            max_offset_min: 5,
            min_block_headway: 100,
            t0: 8.0,
            cooling: 0.985,
            seed: 20260809,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SystemResult {
    pub lines: Vec<String>,
    pub offsets: HashMap<String, i64>, // línea -> desfase (segundos)
    pub base_v: f64,
    pub best_v: f64,
    pub delta_pct: f64,
    pub base_delay: f64, // pic de retard acumulat mitjà (s)
    pub best_delay: f64,
    pub base_recovery_min: f64,
    pub best_recovery_min: f64,
    pub base_held: f64,
    pub best_held: f64,
    pub trips: usize,
}

struct Eval {
    v: f64,
    delay: f64,
    recovery_min: f64,
    held: f64,
}

/// Optimiza el sistema completo. `progress(iter, v_actual, v_mejor)` para la UI.
pub fn optimize_system(
    net: &Network,
    sc: SystemSearch,
    w: PotentialWeights,
    progress: &(dyn Fn(usize, f64, f64) + Sync),
) -> Option<SystemResult> {
    let service_id = net.dominant_service()?;

    // Líneas de tren (no bus) con viajes en la ventana, ordenadas por nº de servicios.
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut trips: Vec<(String, String)> = Vec::new(); // (trip_id, línea)
    for s in &net.services {
        if s.service_id != service_id || s.is_bus {
            continue;
        }
        if !matches!(s.first_time(), Some(t) if t >= sc.window.0 && t <= sc.window.1) {
            continue;
        }
        *counts.entry(s.route_short_name.clone()).or_insert(0) += 1;
        trips.push((s.trip_id.clone(), s.route_short_name.clone()));
    }
    if trips.is_empty() {
        return None;
    }
    let mut lines: Vec<String> = counts.keys().cloned().collect();
    lines.sort_by(|a, b| counts[b].cmp(&counts[a]).then_with(|| a.cmp(b)));
    let line_idx: HashMap<&str, usize> =
        lines.iter().enumerate().map(|(i, l)| (l.as_str(), i)).collect();

    let single = Arc::new(crate::topology::single_track_pairs(net));
    let incidents = build_scenarios(net, &service_id, &sc);
    let max_off = sc.max_offset_min * 60;

    // Construye el mapa trip_id -> desfase (por línea) para un vector de offsets.
    let offsets_arc = |v: &[i64]| -> Arc<HashMap<String, i64>> {
        let mut m: HashMap<String, i64> = HashMap::with_capacity(trips.len());
        for (tid, line) in &trips {
            let o = v[line_idx[line.as_str()]];
            if o != 0 {
                m.insert(tid.clone(), o);
            }
        }
        Arc::new(m)
    };

    let evaluate = |v: &[i64]| -> Eval {
        let offsets = offsets_arc(v);
        let res: Vec<(f64, f64, f64, f64)> = incidents
            .par_iter()
            .enumerate()
            .map(|(k, scen)| {
                let sim = run_system(net, &service_id, sc, &offsets, &single, scen, k as u64);
                let v = passenger_weighted_delay(&sim, &w) * w.w_delay
                    + sim.held_events as f64 * w.w_conflict;
                let rec = match sim.recovery_time {
                    Some(t) => (t.saturating_sub(sim.peak_time)) as f64 / 60.0,
                    None => (sc.window.1.saturating_sub(sim.peak_time)) as f64 / 60.0,
                };
                (v, sim.peak_total_delay as f64, rec, sim.held_events as f64)
            })
            .collect();
        let n = res.len().max(1) as f64;
        Eval {
            v: res.iter().map(|r| r.0).sum::<f64>() / n,
            delay: res.iter().map(|r| r.1).sum::<f64>() / n,
            recovery_min: res.iter().map(|r| r.2).sum::<f64>() / n,
            held: res.iter().map(|r| r.3).sum::<f64>() / n,
        }
    };

    let n = lines.len();
    let base = evaluate(&vec![0i64; n]);

    let mut rng = StdRng::seed_from_u64(sc.seed);
    let mut cur = vec![0i64; n];
    let mut e_cur = base.v;
    let mut best = cur.clone();
    let mut e_best = e_cur;
    let mut temp = sc.t0;
    progress(0, e_cur, e_best);

    for it in 0..sc.iters {
        let mut cand = cur.clone();
        let j = rng.gen_range(0..n);
        let step = if rng.gen::<bool>() { 60 } else { -60 };
        cand[j] = (cand[j] + step).clamp(-max_off, max_off);
        if cand[j] == cur[j] {
            progress(it + 1, e_cur, e_best);
            continue;
        }
        let e = evaluate(&cand).v;
        if e < e_cur || rng.gen::<f64>() < ((e_cur - e) / temp).exp() {
            cur = cand;
            e_cur = e;
            if e < e_best {
                e_best = e;
                best = cur.clone();
            }
        }
        temp *= sc.cooling;
        progress(it + 1, e_cur, e_best);
    }

    let best_eval = evaluate(&best);
    let offsets: HashMap<String, i64> = lines
        .iter()
        .cloned()
        .zip(best.iter().cloned())
        .filter(|(_, o)| *o != 0)
        .collect();

    Some(SystemResult {
        lines,
        offsets,
        base_v: base.v,
        best_v: best_eval.v,
        delta_pct: if base.v.abs() > 1e-9 {
            (base.v - best_eval.v) / base.v * 100.0
        } else {
            0.0
        },
        base_delay: base.delay,
        best_delay: best_eval.delay,
        base_recovery_min: base.recovery_min,
        best_recovery_min: best_eval.recovery_min,
        base_held: base.held,
        best_held: best_eval.held,
        trips: trips.len(),
    })
}

#[allow(clippy::too_many_arguments)]
fn run_system(
    net: &Network,
    service_id: &str,
    sc: SystemSearch,
    offsets: &Arc<HashMap<String, i64>>,
    single: &Arc<HashSet<(NodeIndex, NodeIndex)>>,
    scenario: &[Incident],
    seed: u64,
) -> crate::simulation_engine::SimResult {
    let mut cfg = SimConfig::default();
    cfg.start_sec = sc.window.0;
    cfg.end_sec = sc.window.1;
    cfg.strict_signaling = true; // andanes reals + testigo de vía única + aspecte groc
    cfg.exclude_buses = true;
    cfg.min_block_headway_secs = sc.min_block_headway;
    cfg.single_track = single.clone();
    cfg.offsets = offsets.clone();
    cfg.seed = Some(2000 + seed);
    let mut sim = Simulator::new(net, cfg);
    for inc in scenario {
        sim.add_incident(inc.clone());
    }
    sim.run(service_id)
}

/// Genera `mc_runs` escenarios, cada uno con `incidents_per_run` incidencias repartidas
/// por el día en puntos críticos (retrasos 3-12 min y bloqueos de cantón compartido).
fn build_scenarios(net: &Network, service_id: &str, sc: &SystemSearch) -> Vec<Vec<Incident>> {
    let critical = ["clot", "arc de triomf", "barcelona-sants", "passeig de gràcia"];
    let mut delays: Vec<(String, String)> = Vec::new();
    let mut cantons: Vec<(String, String)> = Vec::new();
    for s in net.services.iter().filter(|s| {
        s.service_id == service_id
            && !s.is_bus
            && matches!(s.first_time(), Some(t) if t >= sc.window.0 && t <= sc.window.1)
    }) {
        for st in &s.schedule {
            let name = net.stop_name(&st.stop_id).to_lowercase();
            if critical.iter().any(|c| name.contains(c)) {
                delays.push((s.train_number.clone(), net.stop_name(&st.stop_id).to_string()));
            }
        }
        for w in s.schedule.windows(2) {
            cantons.push((w[0].stop_id.clone(), w[1].stop_id.clone()));
        }
    }
    delays.sort();
    delays.dedup();
    cantons.sort();
    cantons.dedup();

    let mut rng = StdRng::seed_from_u64(sc.seed ^ 0x5EED);
    (0..sc.mc_runs)
        .map(|_| {
            let mut scen = Vec::with_capacity(sc.incidents_per_run);
            for i in 0..sc.incidents_per_run {
                let extra = rng.gen_range(3..=12) * 60;
                // Reparte las incidencias por franjas del día.
                let span = sc.window.1 - sc.window.0;
                let from = sc.window.0 + (span * i as u32 / sc.incidents_per_run.max(1) as u32)
                    + rng.gen_range(0..(span / sc.incidents_per_run.max(1) as u32).max(1));
                if rng.gen::<bool>() && !delays.is_empty() {
                    let (tn, sta) = &delays[rng.gen_range(0..delays.len())];
                    scen.push(Incident::TrainDelay {
                        train_number: tn.clone(),
                        at_stop_name: sta.clone(),
                        extra_secs: extra,
                    });
                } else if !cantons.is_empty() {
                    let (a, b) = &cantons[rng.gen_range(0..cantons.len())];
                    scen.push(Incident::BlockSegmentById {
                        from_stop_id: a.clone(),
                        to_stop_id: b.clone(),
                        from_sec: from,
                        dur_secs: extra,
                    });
                }
            }
            scen
        })
        .collect()
}
