//! **Comparación con Renfe**: ¿los tiempos y retrasos simulados se parecen a los reales?
//!
//! Corre dos jornadas sin incidencias con el mismo horario GTFS:
//!   * **antes** = el motor con todo el realismo nuevo apagado (física pura: parada fija, sin ASFA,
//!     sin rotación, frenada máxima, sin anticipar límites, huecos de vía a Vmax del material…);
//!   * **ahora** = el motor con los parámetros de realismo guardados (+ mercancías por defecto).
//!
//! y las compara, línea a línea, con (1) el horario de Renfe (GTFS) y (2) lo OBSERVADO en el GTFS-RT
//! de Renfe (`processed/gtfsrt/observed.json`, de `scripts/analyze_gtfsrt.py`): tiempos de tramo
//! (marcha entre paradas), tiempos de parada y distribución de retrasos.
//!
//! Uso: `cargo run --release -- compare [semilla]` → tabla en consola + `report/compare.json`.

use crate::calculator::{infrastructure, ltv};
use crate::engine::{Engine, EngineCfg, TripRecord};
use crate::gtfs_loader::Network;
use crate::realism::RealismCfg;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

pub const OBSERVED_PATH: &str = "processed/gtfsrt/observed.json";

/// Realismo apagado: reproduce el motor anterior a este paquete de cambios.
pub fn legacy_cfg() -> RealismCfg {
    RealismCfg {
        pax_enabled: false,
        rotation_enabled: false,
        asfa_enabled: false,
        service_decel: 1.5, // el maquinista frena con la deceleración máxima del material
        accel_fraction: 0.75,
        anticipate_limits: false,
        reaction_min_s: 0.0,
        reaction_max_s: 0.0,
        tunnel_speed_factor: 1.0,
        davis_enabled: false,
        fill_track_gaps: false,
        terminal_crossing: false,
        freight_yield: false,
        ..RealismCfg::default()
    }
}

#[derive(Serialize, Default, Clone)]
pub struct LineCompare {
    pub line: String,
    pub trips: usize,
    /// Mediana del tiempo de viaje completo (min): horario, simulado antes y ahora.
    pub trip_gtfs_min: f64,
    pub trip_before_min: f64,
    pub trip_now_min: f64,
    /// Sobre los tramos que también se observan en el GTFS-RT: suma de tiempos de marcha (min).
    pub seg_covered: usize,
    pub seg_gtfs_min: f64,
    pub seg_before_min: f64,
    pub seg_now_min: f64,
    pub seg_observed_min: f64,
    /// Parada media en las estaciones observadas (s).
    pub dwell_now_s: f64,
    pub dwell_observed_s: f64,
    /// Retraso en llegada a las paradas: % < 5 min y % ≥ 15 min, y percentil 90 (min).
    pub punct5_before: f64,
    pub punct5_now: f64,
    pub over15_before: f64,
    pub over15_now: f64,
    pub p90_now_min: f64,
    /// Lo mismo en la realidad (GTFS-RT), si hay datos de esa línea.
    pub punct5_real: Option<f64>,
    pub over15_real: Option<f64>,
    pub p90_real_min: Option<f64>,
}

/// Un tramo observado y cómo de lejos queda la simulación (para localizar dónde falla).
#[derive(Serialize, Clone)]
pub struct SegDev {
    pub from: String,
    pub to: String,
    pub lines: String,
    pub n_obs: u64,
    pub gtfs_s: f64,
    pub now_s: f64,
    pub observed_s: f64,
}

#[derive(Serialize)]
pub struct CompareReport {
    pub seed: u64,
    pub observed_days: Vec<String>,
    pub observed_polls: u64,
    pub lines: Vec<LineCompare>,
    pub total_delay_before_train_min: f64,
    pub total_delay_now_train_min: f64,
    /// Tramos observados donde la simulación es más RÁPIDA que la realidad (los peores primero).
    pub too_fast: Vec<SegDev>,
}

fn median(mut v: Vec<f64>) -> f64 {
    v.retain(|x| x.is_finite());
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        0.5 * (v[n / 2 - 1] + v[n / 2])
    }
}

fn pct(mut v: Vec<f64>, p: f64) -> f64 {
    v.retain(|x| x.is_finite());
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    v[(((v.len() - 1) as f64) * p).round() as usize]
}

/// Tiempos medianos de marcha por tramo (A,B) y de parada por estación de una jornada.
struct DayStats {
    seg: HashMap<(String, String), f64>,
    dwell: HashMap<String, f64>,
}

/// Máximo de paradas que puede abarcar un tramo observado (paradas intermedias no vistas).
const MAX_PAIR_SPAN: usize = 6;

fn day_stats(recs: &[TripRecord]) -> DayStats {
    let mut seg: HashMap<(String, String), Vec<f64>> = HashMap::new();
    let mut dwell: HashMap<String, Vec<f64>> = HashMap::new();
    for r in recs {
        for i in 0..r.stops.len() {
            for j in (i + 1)..r.stops.len().min(i + 1 + MAX_PAIR_SPAN) {
                let (a, b) = (&r.stops[i], &r.stops[j]);
                if a.dep.is_finite() && b.arr.is_finite() {
                    seg.entry((a.stop.clone(), b.stop.clone())).or_default().push(b.arr - a.dep);
                }
            }
        }
        let n = r.stops.len();
        for s in r.stops.iter().take(n.saturating_sub(1)).skip(1) {
            if s.arr.is_finite() && s.dep.is_finite() {
                dwell.entry(s.stop.clone()).or_default().push(s.dep - s.arr);
            }
        }
    }
    DayStats {
        seg: seg.into_iter().map(|(k, v)| (k, median(v))).collect(),
        dwell: dwell.into_iter().map(|(k, v)| (k, median(v))).collect(),
    }
}

/// Retrasos de llegada (s, ≥0) a todas las paradas (salvo el origen) de los trenes de una línea.
fn arrival_delays(recs: &[TripRecord], line: &str) -> Vec<f64> {
    recs.iter()
        .filter(|r| r.line == line)
        .flat_map(|r| r.stops.iter().skip(1).filter(|s| s.arr.is_finite()).map(|s| (s.arr - s.sched_arr).max(0.0)))
        .collect()
}

fn trip_minutes(recs: &[TripRecord], line: &str, sched: bool) -> f64 {
    median(
        recs.iter()
            .filter(|r| r.line == line && r.stops.len() >= 2)
            .map(|r| {
                let (a, b) = (r.stops.first().unwrap(), r.stops.last().unwrap());
                if sched {
                    (b.sched_arr - a.sched_dep) / 60.0
                } else {
                    (b.arr - a.dep) / 60.0
                }
            })
            .collect(),
    )
}

fn run_day(
    net: &Network,
    adif: Option<&infrastructure::AdifNet>,
    ltv: Option<&ltv::LtvSet>,
    rc: &RealismCfg,
    seed: u64,
    with_freight: bool,
) -> (Vec<TripRecord>, f64) {
    let mut cfg = EngineCfg::day();
    cfg.seed = seed;
    let trains = crate::game::build_schedule(net, false, None);
    let freight = if with_freight {
        let start = trains.iter().filter_map(|t| t.stops.first().map(|s| s.d)).min().unwrap_or(5 * 3600) as f64;
        crate::freight::load_scenarios()
            .into_iter()
            .find(|s| s.kind != "fixed")
            .map(|sc| crate::freight::build_trains(net, &crate::freight::plan(&sc, seed, start + 300.0, 86400.0)).0)
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let mut eng = Engine::with_schedule_ext(net, cfg, adif, ltv, &[], None, &trains, rc, &freight);
    let m = eng.run();
    (eng.stop_records(), m.train_min_delay)
}

pub fn compare(
    net: &Network,
    adif: Option<&infrastructure::AdifNet>,
    ltv: Option<&ltv::LtvSet>,
    seed: u64,
) -> CompareReport {
    let now_cfg = RealismCfg::load();
    let ((before, d_before), (now, d_now)) = rayon::join(
        || run_day(net, adif, ltv, &legacy_cfg(), seed, false),
        || run_day(net, adif, ltv, &now_cfg, seed, true),
    );
    let sb = day_stats(&before);
    let sn = day_stats(&now);

    let obs: serde_json::Value = std::fs::read_to_string(OBSERVED_PATH)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(serde_json::Value::Null);
    let obs_seg = |a: &str, b: &str| obs["segments"][format!("{a}|{b}")]["median_s"].as_f64();
    let obs_dwell = |a: &str| obs["dwells"][a]["median_s"].as_f64();

    let mut lines: BTreeMap<String, Vec<&TripRecord>> = BTreeMap::new();
    for r in &now {
        lines.entry(r.line.clone()).or_default().push(r);
    }
    let mut out = Vec::new();
    for (line, recs) in &lines {
        let mut lc = LineCompare { line: line.clone(), trips: recs.len(), ..Default::default() };
        lc.trip_gtfs_min = trip_minutes(&now, line, true);
        lc.trip_before_min = trip_minutes(&before, line, false);
        lc.trip_now_min = trip_minutes(&now, line, false);
        // Tramos observados: sumas sobre todos los viajes de la línea (pesa cada tramo por su uso).
        let mut dn = Vec::new();
        let mut dobs = Vec::new();
        for r in recs {
            // Tramos observados entre dos paradas A→C (con paradas intermedias si el feed no las vio).
            for i in 0..r.stops.len() {
                for j in (i + 1)..r.stops.len().min(i + 1 + MAX_PAIR_SPAN) {
                    let (a, b) = (&r.stops[i], &r.stops[j]);
                    let Some(o) = obs_seg(&a.stop, &b.stop) else { continue };
                    let key = (a.stop.clone(), b.stop.clone());
                    let (Some(&n), Some(&bf)) = (sn.seg.get(&key), sb.seg.get(&key)) else { continue };
                    if !(n.is_finite() && bf.is_finite()) {
                        continue;
                    }
                    lc.seg_covered += 1;
                    lc.seg_gtfs_min += (b.sched_arr - a.sched_dep) / 60.0;
                    lc.seg_before_min += bf / 60.0;
                    lc.seg_now_min += n / 60.0;
                    lc.seg_observed_min += o / 60.0;
                }
            }
            for s in r.stops.iter().skip(1).take(r.stops.len().saturating_sub(2)) {
                if let (Some(o), Some(&n)) = (obs_dwell(&s.stop), sn.dwell.get(&s.stop)) {
                    if n.is_finite() {
                        dobs.push(o);
                        dn.push(n);
                    }
                }
            }
        }
        lc.dwell_now_s = median(dn);
        lc.dwell_observed_s = median(dobs);
        let delays_b = arrival_delays(&before, line);
        let delays_n = arrival_delays(&now, line);
        let frac = |v: &[f64], f: &dyn Fn(f64) -> bool| {
            if v.is_empty() {
                f64::NAN
            } else {
                v.iter().filter(|&&x| f(x)).count() as f64 / v.len() as f64
            }
        };
        lc.punct5_before = frac(&delays_b, &|x| x < 300.0);
        lc.punct5_now = frac(&delays_n, &|x| x < 300.0);
        lc.over15_before = frac(&delays_b, &|x| x >= 900.0);
        lc.over15_now = frac(&delays_n, &|x| x >= 900.0);
        lc.p90_now_min = pct(delays_n, 0.9) / 60.0;
        let real = &obs["delays_by_line"][line.as_str()];
        lc.punct5_real = real["punct_5min"].as_f64();
        lc.over15_real = real["over_15min"].as_f64();
        lc.p90_real_min = real["p90_s"].as_f64().map(|s| s / 60.0);
        out.push(lc);
    }
    // Tramos observados (≥ 2 observaciones) con la mayor diferencia real − simulado.
    let mut too_fast: Vec<SegDev> = Vec::new();
    if let Some(m) = obs["segments"].as_object() {
        for (k, v) in m {
            let Some((a, b)) = k.split_once('|') else { continue };
            let n_obs = v["n"].as_u64().unwrap_or(0);
            let (Some(o), Some(&n)) = (v["median_s"].as_f64(), sn.seg.get(&(a.to_string(), b.to_string()))) else { continue };
            if n_obs < 2 || !n.is_finite() {
                continue;
            }
            let gtfs = now
                .iter()
                .find_map(|r| {
                    let i = r.stops.iter().position(|s| s.stop == a)?;
                    let j = r.stops.iter().position(|s| s.stop == b)?;
                    (j > i).then(|| r.stops[j].sched_arr - r.stops[i].sched_dep)
                })
                .unwrap_or(f64::NAN);
            too_fast.push(SegDev {
                from: net.stop_name(a).to_string(),
                to: net.stop_name(b).to_string(),
                lines: v["lines"].as_array().map(|l| l.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(",")).unwrap_or_default(),
                n_obs,
                gtfs_s: gtfs,
                now_s: n,
                observed_s: o,
            });
        }
    }
    too_fast.sort_by(|x, y| (y.observed_s - y.now_s).total_cmp(&(x.observed_s - x.now_s)));
    too_fast.truncate(25);
    CompareReport {
        seed,
        too_fast,
        observed_days: obs["days"]
            .as_array()
            .map(|a| a.iter().filter_map(|d| d.as_str().map(String::from)).collect())
            .unwrap_or_default(),
        observed_polls: obs["polls"].as_u64().unwrap_or(0),
        lines: out,
        total_delay_before_train_min: d_before,
        total_delay_now_train_min: d_now,
    }
}

/// Imprime la comparación en consola (catalán, como el resto de la salida) y la guarda en JSON.
pub fn print_and_save(rep: &CompareReport) {
    let f = |x: f64| if x.is_finite() { format!("{x:6.1}") } else { "     –".into() };
    let p = |x: f64| if x.is_finite() { format!("{:4.0}%", x * 100.0) } else { "   –".into() };
    let po = |x: Option<f64>| x.map(p).unwrap_or_else(|| "   –".into());
    println!(
        "\n── COMPARACIÓ AMB RENFE (llavor {}, sense incidències) · dades reals: {} sondeigs GTFS-RT {:?}",
        rep.seed, rep.observed_polls, rep.observed_days
    );
    println!(
        "   Retard acumulat de la xarxa: abans {:.0} tren·min → ara {:.0} tren·min",
        rep.total_delay_before_train_min, rep.total_delay_now_train_min
    );
    println!("\n  VIATGE COMPLET (mediana, min)            TRAMS OBSERVATS (suma de marxa, min)         PARADA (s)");
    println!("  línia   GTFS  abans   ara  │ trams   GTFS  abans    ara   REAL │   ara  REAL");
    for l in &rep.lines {
        println!(
            "  {:<5} {} {} {} │ {:5} {} {} {} {} │ {} {}",
            l.line,
            f(l.trip_gtfs_min),
            f(l.trip_before_min),
            f(l.trip_now_min),
            l.seg_covered,
            f(l.seg_gtfs_min),
            f(l.seg_before_min),
            f(l.seg_now_min),
            if l.seg_covered > 0 { f(l.seg_observed_min) } else { "     –".into() },
            f(l.dwell_now_s),
            f(l.dwell_observed_s),
        );
    }
    println!("\n  RETARD A LES PARADES      <5 min (abans → ara | REAL)      ≥15 min (abans → ara | REAL)     p90 min (ara | REAL)");
    for l in &rep.lines {
        println!(
            "  {:<5}                   {} → {} | {}              {} → {} | {}            {} | {}",
            l.line,
            p(l.punct5_before),
            p(l.punct5_now),
            po(l.punct5_real),
            p(l.over15_before),
            p(l.over15_now),
            po(l.over15_real),
            f(l.p90_now_min),
            l.p90_real_min.map(f).unwrap_or_else(|| "     –".into()),
        );
    }
    if !rep.too_fast.is_empty() {
        println!("
  TRAMS ON EL SIMULADOR VA MÉS RÀPID QUE LA REALITAT (mediana, s)");
        println!("  {:<34} {:<34} {:<9} {:>4} {:>6} {:>6} {:>6}", "de", "a", "línies", "n", "GTFS", "ara", "REAL");
        for d in &rep.too_fast {
            println!(
                "  {:<34} {:<34} {:<9} {:>4} {:>6.0} {:>6.0} {:>6.0}",
                d.from, d.to, d.lines, d.n_obs, d.gtfs_s, d.now_s, d.observed_s
            );
        }
    }
    std::fs::create_dir_all("report").ok();
    if std::fs::write("report/compare.json", serde_json::to_string_pretty(rep).unwrap_or_default()).is_ok() {
        println!("\n✓ report/compare.json");
    }
}
