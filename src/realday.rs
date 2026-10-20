//! Días REALES (GTFS-RT de Renfe grabado) como «runs» con el MISMO formato que las del motor, y las
//! estadísticas comunes con las que se comparan simulación y realidad.
//!
//! El script `scripts/gtfsrt_track.py` reduce los sondeos de un día a series por minuto (retraso vivo
//! de la red y por línea) y al retraso de llegada de cada tren a cada parada observada
//! (`processed/gtfsrt/track_<día>.json`). Aquí:
//!   * [`RealDay::run_json`] lo convierte en `{track, analysis, gaps}` —lo que devuelve el motor para una
//!     jornada—, así el seguimiento de retrasos y el análisis de recuperación (`recovery.rs`) se aplican
//!     IGUAL a un día real que a uno simulado;
//!   * [`day_stats`] calcula puntualidad, retrasos por línea y los tramos donde más retraso se genera a
//!     partir de pasos por parada ([`TripArrivals`]), que salen igual del motor ([`from_sim`]) que de los
//!     datos reales ([`RealDay::arrivals`]): misma medida a los dos lados;
//!   * [`match_trains`] empareja trenes simulados y reales por nº de tren + línea.
//!
//! Medidas (y su límite): el retraso real es el campo `delay` de trip_updates (s) con la resolución del
//! feed (~20-30 s); en la simulación, llegada real − llegada programada. Un retraso negativo (adelantado)
//! cuenta como 0 en las medias, igual que el retraso vivo del motor.

use crate::gtfs_loader::Network;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// Umbral de puntualidad: llegada con MENOS de este retraso (s). Se muestra explícito en la interfaz.
pub const PUNCT_S: f64 = 180.0;
/// Cuántos tramos se devuelven en el ranking de «dónde se genera el retraso».
const TOP_SEGMENTS: usize = 20;

#[derive(Deserialize)]
pub struct RealDay {
    pub day: String,
    pub polls: u32,
    pub t0: u32,
    pub dt: u32,
    pub total: Vec<f64>,
    pub active: Vec<u32>,
    pub lines: BTreeMap<String, RealLine>,
    pub gaps: Vec<[u32; 2]>,
    pub trips: Vec<RealTrip>,
    pub canceled: BTreeMap<String, u32>,
    #[serde(default)]
    pub notices: Vec<RealNotice>,
}

/// Aviso de incidencia publicado ese día (Gencat o Renfe): hora de publicación (s), líneas y texto.
#[derive(Deserialize)]
pub struct RealNotice {
    pub t: f64,
    pub source: String,
    pub lines: Vec<String>,
    pub text: String,
}

#[derive(Deserialize)]
pub struct RealLine {
    pub total: Vec<f64>,
    pub active: Vec<u32>,
}

#[derive(Deserialize)]
pub struct RealTrip {
    pub key: String,
    pub line: String,
    /// `(parada, hora s, retraso s)` al llegar.
    pub stops: Vec<(String, f64, f64)>,
}

/// Lee `processed/gtfsrt/track_<día>.json` (el script ya se ha ejecutado).
pub fn load(day: &str) -> Result<RealDay, String> {
    let path = format!("processed/gtfsrt/track_{day}.json");
    let txt = std::fs::read_to_string(&path).map_err(|e| format!("No se pudo leer {path}: {e}"))?;
    serde_json::from_str(&txt).map_err(|e| format!("{path} no es válido: {e}"))
}

impl RealDay {
    /// Línea temporal `(instante s, retraso acumulado s)` como la del motor.
    pub fn timeline(&self) -> Vec<(u32, f64)> {
        self.total.iter().enumerate().map(|(i, &d)| (self.t0 + i as u32 * self.dt, d)).collect()
    }

    /// La jornada real en el formato de una run del motor: `{track (+lines), analysis, gaps}`.
    pub fn run_json(&self, net: &Network) -> serde_json::Value {
        let tl = self.timeline();
        let tr = crate::recovery::delay_track(&tl, &self.active);
        let lines: Vec<_> = self
            .lines
            .iter()
            .map(|(l, s)| {
                let tot: Vec<f32> = s.total.iter().map(|&d| d as f32).collect();
                let act: Vec<u16> = s.active.iter().map(|&n| n.min(u16::MAX as u32) as u16).collect();
                crate::recovery::line_track(l, crate::game::line_color(net, l), &tot, &act)
            })
            .collect();
        let mut track = serde_json::to_value(&tr).unwrap_or_default();
        track["lines"] = serde_json::to_value(&lines).unwrap_or_default();
        serde_json::json!({
            "track": track,
            "analysis": { "mean": crate::recovery::analyze_track(&tl, &self.active) },
            "gaps": self.gaps,
        })
    }

    /// Pasos por parada de los trenes reales (para [`day_stats`]).
    pub fn arrivals(&self) -> Vec<TripArrivals> {
        self.trips
            .iter()
            .filter(|t| !t.stops.is_empty())
            .map(|t| TripArrivals {
                key: t.key.clone(),
                line: t.line.clone(),
                arr: t.stops.iter().map(|(s, tt, d)| Arrival { stop: s.clone(), t: *tt, delay_s: *d }).collect(),
            })
            .collect()
    }
}

/// Llegada de un tren a una parada: hora (s) y retraso (s, negativo = adelantado).
pub struct Arrival {
    pub stop: String,
    pub t: f64,
    pub delay_s: f64,
}

/// Recorrido de un tren como sucesión de llegadas (común a simulación y realidad).
pub struct TripArrivals {
    /// Nº de tren + línea («15004R15»): igual en el GTFS simulado y en el GTFS-RT del día.
    pub key: String,
    pub line: String,
    pub arr: Vec<Arrival>,
}

/// Clave común de un viaje: el id del GTFS sin el prefijo del día de servicio («5187V15004R15» →
/// «15004R15»). Si el id no sigue ese patrón, el propio id.
pub fn trip_key(id: &str) -> String {
    let b = id.as_bytes();
    if b.len() > 5 && b[..4].iter().all(u8::is_ascii_digit) && b[4].is_ascii_uppercase() && b[5].is_ascii_digit() {
        id[5..].to_string()
    } else {
        id.to_string()
    }
}

/// Pasos por parada de los trenes SIMULADOS (paradas comerciales a las que llegaron).
pub fn from_sim(records: &[crate::engine::TripRecord]) -> Vec<TripArrivals> {
    records
        .iter()
        .map(|r| TripArrivals {
            key: trip_key(&r.id),
            line: r.line.clone(),
            arr: r
                .stops
                .iter()
                .skip(1) // en el origen no hay llegada
                .filter(|s| s.arr.is_finite())
                .map(|s| Arrival { stop: s.stop.clone(), t: s.arr, delay_s: s.arr - s.sched_arr })
                .collect(),
        })
        .filter(|t| !t.arr.is_empty())
        .collect()
}

/// Estadísticas de llegadas de un conjunto de trenes (una línea o la red entera).
#[derive(Serialize, Clone)]
pub struct LineStats {
    pub line: String,
    pub color: String,
    pub trains: usize,
    pub arrivals: usize,
    /// Retraso medio / mediano / percentil 90 / máximo al llegar (min; adelantos = 0).
    pub mean_min: f64,
    pub p50_min: f64,
    pub p90_min: f64,
    pub max_min: f64,
    /// % de llegadas con retraso < `PUNCT_S`, ≥ 5 min y ≥ 15 min.
    pub punct_pct: f64,
    pub over5_pct: f64,
    pub over15_pct: f64,
    /// Retraso medio en la ÚLTIMA parada observada de cada tren (min).
    pub final_mean_min: f64,
    /// Viajes cancelados (solo datos reales).
    pub canceled: Option<u32>,
}

/// Un tramo (par de paradas consecutivas observadas) y el retraso que se GENERA en él: lo que sube el
/// retraso entre la llegada a `from` y la llegada a `to` (incluye la parada en `from`).
#[derive(Serialize, Clone)]
pub struct SegStat {
    pub from: String,
    pub to: String,
    pub from_name: String,
    pub to_name: String,
    pub n: usize,
    /// Σ de las subidas (min): retraso nuevo que se crea en el tramo.
    pub gain_min: f64,
    /// Subida media por paso (min, con signo: negativo = se recupera).
    pub mean_min: f64,
    /// (línea, min generados) de mayor a menor.
    pub lines: Vec<(String, f64)>,
}

/// Llegadas de una hora del día (toda la red): cuántas, puntualidad y retraso medio.
#[derive(Serialize)]
pub struct HourStat {
    pub hour: u32,
    pub arrivals: usize,
    pub punct_pct: f64,
    pub mean_min: f64,
}

#[derive(Serialize)]
pub struct DayStats {
    pub network: LineStats,
    pub lines: Vec<LineStats>,
    pub segments: Vec<SegStat>,
    /// Por hora de llegada (0–23; las de después de medianoche cuentan en su hora real).
    pub by_hour: Vec<HourStat>,
    pub punct_threshold_min: f64,
}

fn pct(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let k = (v.len() - 1) as f64 * p;
    let (lo, hi) = (k.floor() as usize, (k.ceil() as usize).min(v.len() - 1));
    v[lo] + (v[hi] - v[lo]) * (k - lo as f64)
}

fn line_stats(line: &str, color: String, trips: &[&TripArrivals], canceled: Option<u32>) -> LineStats {
    let mut d: Vec<f64> = trips.iter().flat_map(|t| t.arr.iter().map(|a| a.delay_s.max(0.0))).collect();
    d.sort_by(f64::total_cmp);
    let n = d.len().max(1) as f64;
    let share = |f: &dyn Fn(f64) -> bool| 100.0 * d.iter().filter(|&&x| f(x)).count() as f64 / n;
    let finals: Vec<f64> = trips.iter().filter_map(|t| t.arr.last()).map(|a| a.delay_s.max(0.0)).collect();
    let r2 = |x: f64| (x * 100.0).round() / 100.0;
    LineStats {
        line: line.to_string(),
        color,
        trains: trips.len(),
        arrivals: d.len(),
        mean_min: r2(d.iter().sum::<f64>() / n / 60.0),
        p50_min: r2(pct(&d, 0.5) / 60.0),
        p90_min: r2(pct(&d, 0.9) / 60.0),
        max_min: r2(d.last().copied().unwrap_or(0.0) / 60.0),
        punct_pct: r2(share(&|x| x < PUNCT_S)),
        over5_pct: r2(share(&|x| x >= 300.0)),
        over15_pct: r2(share(&|x| x >= 900.0)),
        final_mean_min: r2(finals.iter().sum::<f64>() / finals.len().max(1) as f64 / 60.0),
        canceled,
    }
}

/// Puntualidad y retraso medio por hora de llegada (toda la red).
fn by_hour(trips: &[TripArrivals]) -> Vec<HourStat> {
    let mut acc = [(0usize, 0usize, 0.0f64); 24]; // (llegadas, puntuales, Σ retraso s)
    for a in trips.iter().flat_map(|t| &t.arr) {
        let h = ((a.t.max(0.0) / 3600.0) as usize) % 24;
        let d = a.delay_s.max(0.0);
        acc[h].0 += 1;
        acc[h].1 += usize::from(d < PUNCT_S);
        acc[h].2 += d;
    }
    let r2 = |x: f64| (x * 100.0).round() / 100.0;
    acc.iter()
        .enumerate()
        .filter(|(_, a)| a.0 > 0)
        .map(|(h, a)| HourStat {
            hour: h as u32,
            arrivals: a.0,
            punct_pct: r2(100.0 * a.1 as f64 / a.0 as f64),
            mean_min: r2(a.2 / a.0 as f64 / 60.0),
        })
        .collect()
}

/// Lo acumulado en un tramo: pasos, Σ subidas (s), Σ neto (s) y subidas por línea (s).
struct SegAcc {
    n: usize,
    gain: f64,
    net: f64,
    lines: HashMap<String, f64>,
}

/// Retraso que se GENERA en cada tramo: la subida del retraso entre llegadas consecutivas de un mismo
/// tren (incluye la parada en la primera). Solo cuentan las subidas en `gain`; `net` lleva el signo
/// (un tramo donde se recupera resta).
fn segment_gains(trips: &[TripArrivals]) -> HashMap<(&str, &str), SegAcc> {
    let mut seg: HashMap<(&str, &str), SegAcc> = HashMap::new();
    for t in trips {
        for w in t.arr.windows(2) {
            let g = w[1].delay_s - w[0].delay_s;
            let a = seg
                .entry((w[0].stop.as_str(), w[1].stop.as_str()))
                .or_insert(SegAcc { n: 0, gain: 0.0, net: 0.0, lines: HashMap::new() });
            a.n += 1;
            a.net += g;
            if g > 0.0 {
                a.gain += g;
                *a.lines.entry(t.line.clone()).or_insert(0.0) += g;
            }
        }
    }
    seg
}

/// Puntualidad y retrasos (red y por línea) + tramos donde más retraso se genera.
pub fn day_stats(net: &Network, trips: &[TripArrivals], canceled: Option<&BTreeMap<String, u32>>) -> DayStats {
    let mut by_line: BTreeMap<&str, Vec<&TripArrivals>> = BTreeMap::new();
    for t in trips {
        by_line.entry(t.line.as_str()).or_default().push(t);
    }
    let all: Vec<&TripArrivals> = trips.iter().collect();
    let total_canceled = canceled.map(|c| c.values().sum());
    let network = line_stats("Red", "var(--accent)".into(), &all, total_canceled);
    let lines = by_line
        .iter()
        .map(|(l, ts)| line_stats(l, crate::game::line_color(net, l), ts, canceled.map(|c| c.get(*l).copied().unwrap_or(0))))
        .collect();

    let r1 = |x: f64| (x * 10.0).round() / 10.0;
    let mut segments: Vec<SegStat> = segment_gains(trips)
        .into_iter()
        .filter(|(_, a)| a.gain > 0.0)
        .map(|((f, to), a)| {
            let mut lines: Vec<(String, f64)> = a.lines.into_iter().map(|(l, s)| (l, r1(s / 60.0))).collect();
            lines.sort_by(|x, y| y.1.total_cmp(&x.1).then(x.0.cmp(&y.0)));
            SegStat {
                from: f.to_string(),
                to: to.to_string(),
                from_name: net.stop_name(f).to_string(),
                to_name: net.stop_name(to).to_string(),
                n: a.n,
                gain_min: r1(a.gain / 60.0),
                mean_min: (a.net / a.n as f64 / 60.0 * 100.0).round() / 100.0,
                lines,
            }
        })
        .collect();
    // Orden estable: por minutos generados y, a igualdad, por ids (el HashMap itera al azar).
    segments.sort_by(|a, b| b.gain_min.total_cmp(&a.gain_min).then(a.from.cmp(&b.from)).then(a.to.cmp(&b.to)));
    segments.truncate(TOP_SEGMENTS);
    DayStats { network, lines, segments, by_hour: by_hour(trips), punct_threshold_min: PUNCT_S / 60.0 }
}

/// Un tren presente en la simulación y en el día real.
#[derive(Serialize)]
pub struct TrainPair {
    pub key: String,
    pub line: String,
    /// Retraso medio en sus llegadas y en la última llegada (min), simulado y real.
    pub sim_mean_min: f64,
    pub real_mean_min: f64,
    pub sim_final_min: f64,
    pub real_final_min: f64,
}

#[derive(Serialize)]
pub struct Matching {
    pub sim_trains: usize,
    pub real_trains: usize,
    pub matched: usize,
    /// Diferencia media (sim − real) del retraso medio por tren y su valor absoluto medio (min).
    pub bias_min: f64,
    pub mae_min: f64,
    /// Correlación de Pearson del retraso medio por tren (¿se retrasan los MISMOS trenes?).
    pub corr: Option<f64>,
    /// Los 25 trenes con más diferencia, de mayor a menor.
    pub worst: Vec<TrainPair>,
}

/// Empareja trenes simulados y reales por nº de tren + línea y mide si se retrasan los mismos.
pub fn match_trains(sim: &[TripArrivals], real: &[TripArrivals]) -> Matching {
    let mean = |t: &TripArrivals| t.arr.iter().map(|a| a.delay_s.max(0.0)).sum::<f64>() / t.arr.len().max(1) as f64 / 60.0;
    let fin = |t: &TripArrivals| t.arr.last().map_or(0.0, |a| a.delay_s.max(0.0)) / 60.0;
    let real_by: HashMap<&str, &TripArrivals> = real.iter().map(|t| (t.key.as_str(), t)).collect();
    let r2 = |x: f64| (x * 100.0).round() / 100.0;
    let mut pairs: Vec<TrainPair> = sim
        .iter()
        .filter_map(|s| {
            real_by.get(s.key.as_str()).map(|r| TrainPair {
                key: s.key.clone(),
                line: s.line.clone(),
                sim_mean_min: r2(mean(s)),
                real_mean_min: r2(mean(r)),
                sim_final_min: r2(fin(s)),
                real_final_min: r2(fin(r)),
            })
        })
        .collect();
    let n = pairs.len();
    let (mut bias, mut mae) = (0.0, 0.0);
    for p in &pairs {
        bias += p.sim_mean_min - p.real_mean_min;
        mae += (p.sim_mean_min - p.real_mean_min).abs();
    }
    let corr = if n >= 3 {
        let (mx, my) = (
            pairs.iter().map(|p| p.sim_mean_min).sum::<f64>() / n as f64,
            pairs.iter().map(|p| p.real_mean_min).sum::<f64>() / n as f64,
        );
        let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
        for p in &pairs {
            let (dx, dy) = (p.sim_mean_min - mx, p.real_mean_min - my);
            sxy += dx * dy;
            sxx += dx * dx;
            syy += dy * dy;
        }
        (sxx > 0.0 && syy > 0.0).then(|| r2(sxy / (sxx * syy).sqrt()))
    } else {
        None
    };
    pairs.sort_by(|a, b| {
        (b.sim_mean_min - b.real_mean_min).abs().total_cmp(&(a.sim_mean_min - a.real_mean_min).abs()).then(a.key.cmp(&b.key))
    });
    pairs.truncate(25);
    Matching {
        sim_trains: sim.len(),
        real_trains: real.len(),
        matched: n,
        bias_min: r2(bias / n.max(1) as f64),
        mae_min: r2(mae / n.max(1) as f64),
        corr,
        worst: pairs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clave_de_tren_sin_prefijo_del_dia() {
        assert_eq!(trip_key("5187V15004R15"), "15004R15");
        assert_eq!(trip_key("5190L77803R7"), "77803R7");
        assert_eq!(trip_key("R2-especial"), "R2-especial");
    }

    #[test]
    fn el_retraso_se_genera_en_el_tramo_donde_sube() {
        let tren = |linea: &str, d: [f64; 3]| TripArrivals {
            key: "1".into(),
            line: linea.into(),
            arr: ["A", "B", "C"].iter().zip(d).map(|(s, x)| Arrival { stop: (*s).into(), t: 0.0, delay_s: x }).collect(),
        };
        // R1: +5 min en A→B y recupera 2 en B→C; R2: +1 min en A→B.
        let trips = [tren("R1", [60.0, 360.0, 240.0]), tren("R2", [0.0, 60.0, 60.0])];
        let g = segment_gains(&trips);
        let ab = &g[&("A", "B")];
        assert_eq!((ab.n, ab.gain, ab.net), (2, 360.0, 360.0));
        assert_eq!(ab.lines["R1"], 300.0);
        let bc = &g[&("B", "C")];
        assert_eq!((bc.gain, bc.net), (0.0, -120.0)); // se recupera: no genera retraso
    }

    #[test]
    fn puntualidad_por_hora_de_llegada() {
        let t = TripArrivals {
            key: "1".into(),
            line: "R1".into(),
            arr: vec![
                Arrival { stop: "A".into(), t: 8.0 * 3600.0 + 10.0, delay_s: 60.0 },  // 08 h, puntual
                Arrival { stop: "B".into(), t: 8.0 * 3600.0 + 900.0, delay_s: 600.0 }, // 08 h, 10 min
                Arrival { stop: "C".into(), t: 25.0 * 3600.0, delay_s: -30.0 },          // 01 h (tras medianoche), adelantado
            ],
        };
        let h = by_hour(&[t]);
        assert_eq!(h.len(), 2);
        assert_eq!((h[0].hour, h[0].arrivals), (1, 1));
        assert_eq!(h[0].mean_min, 0.0); // adelantado = 0
        assert_eq!((h[1].hour, h[1].arrivals, h[1].punct_pct), (8, 2, 50.0));
        assert_eq!(h[1].mean_min, 5.5);
    }

    #[test]
    fn emparejar_trenes_y_correlacion() {
        let mk = |k: &str, d: &[f64]| TripArrivals {
            key: k.into(),
            line: "R1".into(),
            arr: d.iter().enumerate().map(|(i, &x)| Arrival { stop: i.to_string(), t: i as f64, delay_s: x * 60.0 }).collect(),
        };
        let sim = vec![mk("1R1", &[1.0]), mk("2R1", &[5.0]), mk("3R1", &[10.0]), mk("9R1", &[3.0])];
        let real = vec![mk("1R1", &[2.0]), mk("2R1", &[6.0]), mk("3R1", &[11.0])];
        let m = match_trains(&sim, &real);
        assert_eq!(m.matched, 3);
        assert_eq!(m.bias_min, -1.0); // la simulación va 1 min por debajo
        assert_eq!(m.corr, Some(1.0)); // pero se retrasan los mismos trenes
    }
}
