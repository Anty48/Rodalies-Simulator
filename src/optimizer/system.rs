//! Optimización del SISTEMA ENTERO (todas las líneas a la vez) sobre el **motor unificado**.
//!
//! El problema real no es una línea aislada, sino los **conflictos entre líneas** en los
//! cantones compartidos (p.ej. el tronco Sants–Passeig lo usan 8-9 líneas) y cómo el
//! sistema responde a las **incidencias**. Un buen juego de horarios hace que, ante una
//! incidencia, el sistema tienda a la estabilidad en vez de al caos.
//!
//! Se optimiza un **desfase de fase por línea** (±5 min) para todo un día laborable
//! (05:00–00:00). Cada candidato se evalúa con varias simulaciones Monte Carlo del
//! sistema completo con `src/engine.rs` (la MISMA física que ve el juego): cada run es una
//! semilla distinta cuya **tirada de incidencias** es reproducible e **independiente del
//! desfase** (los pesos espaciales/temporales no dependen de la fase) → comparación justa
//! entre candidatos. Se evalúan en paralelo (`rayon`) y se minimiza el potencial V (retraso
//! ponderado por pasajeros + penalización por conflicto), integrado sobre todo el día.

use std::collections::HashMap;

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rayon::prelude::*;

use crate::calculator::{infrastructure::AdifNet, ltv::LtvSet};
use crate::engine::{default_templates, Engine, EngineCfg, IncidentTemplate};
use crate::gtfs_loader::Network;
use crate::optimizer::potential::{weighted_delay_from_timeline, PotentialWeights};

#[derive(Debug, Clone, Copy)]
pub struct SystemSearch {
    pub window: (u32, u32),
    pub iters: usize,
    pub mc_runs: usize,
    pub max_offset_min: i64,
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
            max_offset_min: 5,
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

/// Optimiza el sistema completo con el motor unificado. `adif`/`ltv` = datos de física ya cargados
/// (si faltan, el motor cae a Vmax de vía por defecto). `progress(iter, v_actual, v_mejor)` para la UI.
pub fn optimize_system(
    net: &Network,
    adif: Option<&AdifNet>,
    ltv: Option<&LtvSet>,
    sc: SystemSearch,
    w: PotentialWeights,
    progress: &(dyn Fn(usize, f64, f64) + Sync),
) -> Option<SystemResult> {
    let service_id = net.dominant_service()?;

    // Líneas de tren (no bus) con viajes en la ventana, ordenadas por nº de servicios.
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut trips = 0usize;
    for s in &net.services {
        if s.service_id != service_id || s.is_bus {
            continue;
        }
        if !matches!(s.first_time(), Some(t) if t >= sc.window.0 && t <= sc.window.1) {
            continue;
        }
        *counts.entry(s.route_short_name.clone()).or_insert(0) += 1;
        trips += 1;
    }
    if trips == 0 {
        return None;
    }
    let mut lines: Vec<String> = counts.keys().cloned().collect();
    lines.sort_by(|a, b| counts[b].cmp(&counts[a]).then_with(|| a.cmp(b)));

    let templates = default_templates();
    let max_off = sc.max_offset_min * 60;

    // Mapa línea -> desfase (segundos) para un vector de offsets alineado con `lines`.
    let offsets_map = |v: &[i64]| -> HashMap<String, i64> {
        lines.iter().cloned().zip(v.iter().copied()).filter(|(_, o)| *o != 0).collect()
    };

    let evaluate = |v: &[i64]| -> Eval {
        let offsets = offsets_map(v);
        let res: Vec<(f64, f64, f64, f64)> = (0..sc.mc_runs)
            .into_par_iter()
            .map(|k| run_seed(net, adif, ltv, sc, &templates, &offsets, k as u64, &w))
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
        trips,
    })
}

/// Una simulación Monte-Carlo del sistema completo con el motor unificado, para un vector de
/// desfases y una semilla. Devuelve `(V, pic_retard_acumulat_s, recuperació_min, conflictes)`.
#[allow(clippy::too_many_arguments)]
fn run_seed(
    net: &Network,
    adif: Option<&AdifNet>,
    ltv: Option<&LtvSet>,
    sc: SystemSearch,
    templates: &[IncidentTemplate],
    offsets: &HashMap<String, i64>,
    k: u64,
    w: &PotentialWeights,
) -> (f64, f64, f64, f64) {
    let mut cfg = EngineCfg::day();
    cfg.start_sec = sc.window.0;
    cfg.end_sec = sc.window.1;
    // Semilla FIJA por índice de run (independiente del desfase) → comparación justa entre candidatos.
    cfg.seed = sc.seed.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(k);

    let off = if offsets.is_empty() { None } else { Some(offsets) };
    let mut eng = Engine::new(net, cfg, adif, ltv, templates, off, None);
    let m = eng.run();

    let delay = weighted_delay_from_timeline(eng.timeline(), w);
    let v = delay * w.w_delay + m.held_events as f64 * w.w_conflict;
    let recovery_min = match m.recovery_time {
        Some(t) => t.saturating_sub(m.peak_time) as f64 / 60.0,
        None => sc.window.1.saturating_sub(m.peak_time) as f64 / 60.0,
    };
    (v, m.peak_total_delay_s, recovery_min, m.held_events as f64)
}
