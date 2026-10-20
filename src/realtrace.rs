//! Reproducción VISUAL de un día REAL: un trace con el MISMO formato que el del motor
//! ([`crate::engine::Trace`]) pero hecho con las posiciones GPS grabadas del GTFS-RT de Renfe.
//!
//! El visor ya es un reproductor de traces (cantón, fracción, velocidad y retraso de cada tren cada
//! 5 s), así que un día real se ve con todo lo del visor: mapa, paneles, registro de llegadas tarde,
//! seguimiento de retrasos. Los SEMÁFOROS son solo visuales: el visor reconstruye la ocupación de
//! los bloques a partir de dónde están los trenes y pinta el aspecto que darían NUESTRAS señales;
//! los trenes no los obedecen (obedecen a las señales reales, que no conocemos). Sirven para ver si
//! un tren parado en plena vía lo está por un tren delante (sería rojo en nuestro modelo) o por otra
//! causa.
//!
//! Cómo se sitúa cada observación: el viaje real se empareja con su viaje del GTFS por nº de tren +
//! línea ([`crate::realday::trip_key`]); su recorrido (cantones con bifurcaciones y apartaderos, con
//! la geometría real de vía) es la referencia. Una observación «parado en X» se fija en la estación X;
//! el resto se proyecta sobre los cantones siguientes al último conocido (nunca hacia atrás). Entre
//! observaciones (~20-30 s) la posición se interpola linealmente: en los huecos sin datos del día el
//! tren se desliza en línea recta por la vía (no se inventa nada más).

use crate::engine::{LateArrival, Trace, TraceTrain};
use crate::game::GameTrain;
use crate::gtfs_loader::Network;
use serde::Deserialize;
use std::collections::HashMap;

/// Paso de muestreo del trace (s), el mismo que el del motor para el visor.
pub const SAMPLE_DT: f64 = 5.0;
/// Una posición a más de esto de la vía de su recorrido se descarta (GPS errático o desvío).
const MAX_OFF_TRACK_M: f64 = 600.0;
/// Cantones por delante del último conocido en los que se busca la siguiente posición.
const SEARCH_AHEAD: usize = 12;
/// Retraso mínimo (s) para anotar una llegada en el registro de llegadas tarde (como el motor).
const LATE_S: f64 = 60.0;
/// Las posiciones de más de esto ANTES de su salida programada son de la unidad haciendo su servicio
/// anterior (el feed asigna el viaje a la unidad con antelación): se descartan.
const PRE_TRIP_S: f64 = 900.0;
/// Velocidad máxima plausible entre dos observaciones (m/s, 200 km/h). Un avance más rápido es una
/// reubicación del feed (posición vieja corregida): no se interpola, el tren aparece en su sitio.
const MAX_SPEED_MS: f64 = 200.0 / 3.6;
/// Margen (m) para dar por buena una posición «en camino a X» que ya parece haber pasado X.
const BEYOND_STOP_M: f64 = 200.0;
/// «Parado en X» solo fija el tren en la estación X si su GPS está a menos de esto de ella (m).
const STOP_SNAP_M: f64 = 800.0;
/// Holgura (m) de esa comprobación (GPS y proyección).
const JUMP_SLACK_M: f64 = 300.0;
/// Arranque desde parado para decidir si una posición repetida estaba CONGELADA (el GPS del feed solo
/// cambia en ~1 de cada 4 refrescos): aceleración (m/s²) y velocidad de crucero (m/s) generosas.
const START_ACCEL: f64 = 0.5;
const START_VMAX: f64 = 120.0 / 3.6;
/// Completar el principio/final del recorrido que el feed no cubre: como mucho estos metros y segundos.
const FILL_MAX_M: f64 = 30_000.0;
const FILL_MAX_S: f64 = 45.0 * 60.0;
/// Velocidad media supuesta (m/s) para completar el final del recorrido.
const FILL_SPEED: f64 = 60.0 / 3.6;

/// Tiempo mínimo (s) para recorrer `d` m arrancando desde parado (aceleración y luego crucero).
fn t_desde_parado(d: f64) -> f64 {
    let d_acc = START_VMAX * START_VMAX / (2.0 * START_ACCEL);
    if d <= d_acc {
        (2.0 * d / START_ACCEL).sqrt()
    } else {
        START_VMAX / START_ACCEL + (d - d_acc) / START_VMAX
    }
}

/// Corrige las posiciones CONGELADAS: si tras una racha parado (misma posición) el tren aparece más
/// lejos de lo que puede recorrer arrancando desde parado, en realidad salió antes: se adelanta el
/// final de la racha al último instante en que pudo salir (sin pasar del principio de la racha).
fn descongelar(track: &mut [(f64, f64, f64)]) {
    for j in 1..track.len().saturating_sub(1) {
        let (p, c, q) = (track[j - 1], track[j], track[j + 1]);
        if (c.1 - p.1).abs() > 1e-6 || q.1 <= c.1 {
            continue; // no es el final de una racha parado seguida de movimiento
        }
        if (q.1 - c.1) > MAX_SPEED_MS * (q.0 - p.0) + JUMP_SLACK_M {
            continue; // reubicación del feed (imposible incluso desde el principio): sigue siendo un salto
        }
        let salida = q.0 - t_desde_parado(q.1 - c.1);
        if salida < c.0 {
            track[j].0 = salida.max(p.0 + 1.0);
        }
    }
}

/// Completa el principio y el final del recorrido que el feed no cubre (Renfe empieza a dar el viaje
/// tarde o deja de darlo antes de llegar): sale del origen a su hora + el retraso del primer dato (o
/// antes, si no le daría tiempo a llegar a donde se le vio) y llega al destino a su hora + el retraso
/// del último dato. Solo si falta poco (≤ `FILL_MAX_M` y `FILL_MAX_S`): más es un viaje recortado.
fn completar_extremos(track: &mut Vec<(f64, f64, f64)>, route: &Route, gt: &GameTrain) {
    let (t1, s1, d1) = track[0];
    if s1 > 1.0 && s1 <= FILL_MAX_M {
        let salida = (gt.stops[0].d as f64 + d1).min(t1 - t_desde_parado(s1) * 1.2);
        if t1 - salida <= FILL_MAX_S && salida < t1 {
            track.insert(0, (salida, 0.0, d1));
        }
    }
    let (tn, sn, dn) = track[track.len() - 1];
    let fin = *route.cum.last().unwrap();
    if fin - sn > 1.0 && fin - sn <= FILL_MAX_M {
        let llegada = (gt.stops[gt.stops.len() - 1].a as f64 + dn).max(tn + (fin - sn) / FILL_SPEED);
        if llegada - tn <= FILL_MAX_S {
            track.push((llegada, fin, dn));
        }
    }
}

#[derive(Deserialize)]
struct PosFile {
    trips: Vec<PosTrip>,
}

#[derive(Deserialize)]
struct PosTrip {
    key: String,
    /// `(hora s, lat, lon, estado S/I/T, parada, retraso s)`.
    obs: Vec<(f64, f64, f64, String, String, Option<f64>)>,
}

/// Lee `processed/gtfsrt/pos_<día>.json` (lo escribe `scripts/gtfsrt_track.py`).
fn load(day: &str) -> Result<PosFile, String> {
    let path = format!("processed/gtfsrt/pos_{day}.json");
    let txt = std::fs::read_to_string(&path).map_err(|e| format!("No se pudo leer {path}: {e}"))?;
    serde_json::from_str(&txt).map_err(|e| format!("{path} no es válido: {e}"))
}

/// Recorrido de un tren como polilínea por cantones: vértices `(lat, lon, s)` de cada cantón (s =
/// distancia desde el origen del recorrido) y distancia acumulada al inicio de cada punto.
struct Route {
    ids: Vec<String>,
    cantons: Vec<Vec<(f64, f64, f64)>>,
    cum: Vec<f64>,
}

impl Route {
    fn new(net: &Network, gt: &GameTrain) -> Option<Route> {
        let geom = crate::track_geometry::global();
        let ids: Vec<String> = gt.stops.iter().map(|s| s.s.clone()).collect();
        if ids.len() < 2 {
            return None;
        }
        let mut cantons = Vec::with_capacity(ids.len() - 1);
        let mut cum = vec![0.0];
        for w in ids.windows(2) {
            let pts: Vec<(f64, f64)> = match geom.and_then(|g| g.get(&w[0], &w[1])) {
                Some(g) if g.pts.len() >= 2 => g.pts,
                _ => vec![net.coord(&w[0])?, net.coord(&w[1])?],
            };
            let base = *cum.last().unwrap();
            let mut acc = base;
            let mut v = Vec::with_capacity(pts.len());
            for (i, p) in pts.iter().enumerate() {
                if i > 0 {
                    acc += crate::calculator::infrastructure::haversine_m(pts[i - 1].0, pts[i - 1].1, p.0, p.1);
                }
                v.push((p.0, p.1, acc));
            }
            cum.push(acc.max(base + 1.0));
            cantons.push(v);
        }
        Some(Route { ids, cantons, cum })
    }

    /// Cantón que contiene la distancia `s`.
    fn canton_at(&self, s: f64) -> usize {
        let k = self.cum.partition_point(|&c| c <= s);
        k.saturating_sub(1).min(self.cantons.len() - 1)
    }

    /// Proyección de `(lat, lon)` sobre los cantones `k0..=k1`: `(s, distancia a la vía m)`.
    fn project(&self, lat: f64, lon: f64, k0: usize, k1: usize) -> Option<(f64, f64)> {
        let kx = 111_320.0 * lat.to_radians().cos();
        let ky = 110_540.0;
        let mut best: Option<(f64, f64)> = None;
        for c in &self.cantons[k0..=k1.min(self.cantons.len() - 1)] {
            for w in c.windows(2) {
                let (ax, ay) = ((w[0].1 - lon) * kx, (w[0].0 - lat) * ky);
                let (bx, by) = ((w[1].1 - lon) * kx, (w[1].0 - lat) * ky);
                let (dx, dy) = (bx - ax, by - ay);
                let l2 = dx * dx + dy * dy;
                let t = if l2 <= 1e-9 { 0.0 } else { (-(ax * dx + ay * dy) / l2).clamp(0.0, 1.0) };
                let d = ((ax + t * dx).powi(2) + (ay + t * dy).powi(2)).sqrt();
                if best.is_none_or(|(_, bd)| d < bd) {
                    best = Some((w[0].2 + t * (w[1].2 - w[0].2), d));
                }
            }
        }
        best
    }

    /// Distancia sobre el recorrido de una posición GPS, buscando primero en los cantones cercanos por
    /// delante de `cur` y, si no está ahí (hueco largo sin datos), en todo lo que queda de recorrido.
    /// `None` si queda a más de `MAX_OFF_TRACK_M` de la vía.
    fn locate(&self, lat: f64, lon: f64, cur: usize) -> Option<f64> {
        let near = self.project(lat, lon, cur, cur + SEARCH_AHEAD).filter(|p| p.1 <= MAX_OFF_TRACK_M);
        near.or_else(|| self.project(lat, lon, cur, self.cantons.len() - 1).filter(|p| p.1 <= MAX_OFF_TRACK_M)).map(|p| p.0)
    }

    /// Muestra del trace para la distancia `s`: `(punto a, punto b, fracción)`. Parado en la estación
    /// `k` = final del cantón que llega a ella (como el motor). En el origen, antes de salir: `None`
    /// (el motor tampoco enseña los trenes que esperan en cabecera).
    fn sample(&self, s: f64) -> Option<(usize, usize, f64)> {
        let k = self.canton_at(s);
        let len = (self.cum[k + 1] - self.cum[k]).max(1.0);
        let frac = ((s - self.cum[k]) / len).clamp(0.0, 1.0);
        if frac < 1e-3 {
            return if k == 0 { None } else { Some((k - 1, k, 1.0)) };
        }
        Some((k, k + 1, frac))
    }
}

/// Resultado: el trace + cuántos trenes reales se han podido situar y cuántos no tenían viaje GTFS.
pub struct RealTrace {
    pub trace: Trace,
    pub matched: usize,
    pub unmatched: usize,
}

/// Trace de un día real a partir de sus posiciones y del horario GTFS (para los recorridos).
pub fn build(net: &Network, day: &str, game_trains: &[GameTrain]) -> Result<RealTrace, String> {
    let pos = load(day)?;
    let by_key: HashMap<String, &GameTrain> = game_trains.iter().map(|g| (crate::realday::trip_key(&g.train), g)).collect();
    let mut ids: Vec<String> = Vec::new();
    let mut id_ix: HashMap<String, u32> = HashMap::new();
    let mut intern = |s: &str, ids: &mut Vec<String>| -> u32 {
        if let Some(&i) = id_ix.get(s) {
            return i;
        }
        ids.push(s.to_string());
        id_ix.insert(s.to_string(), (ids.len() - 1) as u32);
        (ids.len() - 1) as u32
    };
    let (mut trains, mut arrivals) = (Vec::new(), Vec::new());
    let (mut matched, mut unmatched) = (0, 0);
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for pt in &pos.trips {
        let Some(gt) = by_key.get(&pt.key) else {
            unmatched += 1;
            continue;
        };
        if !seen.insert(gt.train.as_str()) {
            continue; // el mismo tren GTFS dos veces (viaje real partido): basta el primero
        }
        let Some(route) = Route::new(net, gt) else { continue };
        // 1) Cada observación → distancia sobre el recorrido (monótona) y retraso.
        let mut track: Vec<(f64, f64, f64)> = Vec::with_capacity(pt.obs.len()); // (t, s, retraso)
        let (mut cur, mut s_prev, mut d_prev) = (0usize, 0.0f64, 0.0f64);
        let mut logged: std::collections::HashSet<usize> = std::collections::HashSet::new();
        let first_dep = gt.stops[0].d as f64;
        for (t, lat, lon, st, stop, dl) in &pt.obs {
            if *t < first_dep - PRE_TRIP_S {
                continue;
            }
            let d = dl.unwrap_or(d_prev).max(0.0);
            let s = if st == "S" {
                // Parado en una estación de su recorrido por delante de donde iba (primero cerca; tras un
                // hueco sin datos, en cualquier punto por delante): exacto.
                let hasta = (cur + SEARCH_AHEAD + 1).min(route.ids.len());
                // Se fija en la estación salvo que su GPS lo ponga claramente MÁS ALLÁ de ella (el estado
                // se habría quedado atrás). Un GPS por detrás es un GPS atrasado (el feed repite a veces
                // las coordenadas de la estación anterior): manda el estado.
                let gps = route.locate(*lat, *lon, cur);
                let k_st = (cur..hasta).find(|&k| &route.ids[k] == stop).or_else(|| (hasta..route.ids.len()).find(|&k| &route.ids[k] == stop));
                match k_st.filter(|&k| gps.is_none_or(|g| g <= route.cum[k] + STOP_SNAP_M)) {
                    Some(k) => {
                        if d >= LATE_S && k > 0 && logged.insert(k) {
                            arrivals.push(LateArrival {
                                t: *t as u32,
                                train: gt.train.clone(),
                                line: gt.line.clone(),
                                stop: stop.clone(),
                                delay: d.min(65535.0) as u16,
                            });
                        }
                        Some(route.cum[k])
                    }
                    None => gps,
                }
            } else {
                // «En camino a X» / «llegando a X»: el tren aún no ha pasado X. Renfe da en estos
                // estados a menudo las coordenadas EXACTAS de la estación siguiente a X (no la posición
                // del tren): una posición más allá de X es falsa y se descarta.
                let hasta = (cur + SEARCH_AHEAD + 1).min(route.ids.len());
                let kx = (cur..hasta).find(|&k| &route.ids[k] == stop).or_else(|| (hasta..route.ids.len()).find(|&k| &route.ids[k] == stop));
                // (No se usa «llegando a X» para situarlo: el feed lo mantiene minutos con el tren aún en
                // la estación anterior.)
                route.locate(*lat, *lon, cur).filter(|&g| kx.is_none_or(|k| g <= route.cum[k] + BEYOND_STOP_M))
            };
            let Some(s) = s else { continue };
            let s = s.max(s_prev); // nunca hacia atrás (el GPS tiembla unos metros)
            // Salto imposible: se queda donde estaba hasta justo antes y aparece en la nueva posición.
            if let Some(&(tp, sp, dp)) = track.last() {
                if s - sp > MAX_SPEED_MS * (t - tp) + JUMP_SLACK_M && *t - 1.0 > tp {
                    track.push((*t - 1.0, sp, dp));
                }
            }
            cur = route.canton_at(s);
            track.push((*t, s, d));
            s_prev = s;
            d_prev = d;
        }
        if track.len() < 2 {
            continue;
        }
        descongelar(&mut track);
        completar_extremos(&mut track, &route, gt);
        // 2) Remuestreo cada SAMPLE_DT s (interpolación lineal entre observaciones).
        let t_first = (track[0].0 / SAMPLE_DT).ceil() * SAMPLE_DT;
        let t_last = track[track.len() - 1].0.min(24.0 * 3600.0);
        let mut tt = TraceTrain { id: gt.train.clone(), line: gt.line.clone(), t0: f64::NAN, a: Vec::new(), b: Vec::new(), f: Vec::new(), v: Vec::new(), d: Vec::new() };
        let mut j = 0usize;
        let mut t = t_first;
        while t <= t_last {
            while j + 1 < track.len() && track[j + 1].0 < t {
                j += 1;
            }
            let (p, q) = (track[j], track[(j + 1).min(track.len() - 1)]);
            let span = (q.0 - p.0).max(1e-6);
            let w = ((t - p.0) / span).clamp(0.0, 1.0);
            let s = p.1 + (q.1 - p.1) * w;
            let v = if q.0 > p.0 { (q.1 - p.1) / span } else { 0.0 };
            let v = if v > MAX_SPEED_MS { 0.0 } else { v }; // reubicación, no velocidad real
            if let Some((a, b, frac)) = route.sample(s) {
                if tt.t0.is_nan() {
                    tt.t0 = t;
                } else if tt.a.len() as f64 != ((t - tt.t0) / SAMPLE_DT).round() {
                    // Hueco en las muestras (volvió a cabecera, p. ej.): el visor indexa por tiempo,
                    // así que se rellena repitiendo la última muestra.
                    while (tt.a.len() as f64) < ((t - tt.t0) / SAMPLE_DT).round() {
                        let n = tt.a.len() - 1;
                        let (ra, rb, rf, rd) = (tt.a[n], tt.b[n], tt.f[n], tt.d[n]);
                        tt.a.push(ra);
                        tt.b.push(rb);
                        tt.f.push(rf);
                        tt.v.push(0);
                        tt.d.push(rd);
                    }
                }
                tt.a.push(intern(&route.ids[a], &mut ids));
                tt.b.push(intern(&route.ids[b], &mut ids));
                tt.f.push((frac * 1000.0).round() as u16);
                tt.v.push((v * 3.6).round().clamp(0.0, 400.0) as u16);
                tt.d.push((p.2 + (q.2 - p.2) * w).round().clamp(0.0, 65535.0) as u16);
            }
            t += SAMPLE_DT;
        }
        if !tt.a.is_empty() {
            matched += 1;
            trains.push(tt);
        }
    }
    arrivals.sort_by_key(|a| a.t);
    let start = trains.iter().map(|t| t.t0).fold(f64::INFINITY, f64::min);
    let start = if start.is_finite() { start } else { 5.0 * 3600.0 };
    Ok(RealTrace {
        trace: Trace { start, end: 24.0 * 3600.0, dt: SAMPLE_DT, ids, trains, arrivals },
        matched,
        unmatched,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Recorrido recto A → B → C de 1 km por cantón, hacia el norte.
    fn ruta() -> Route {
        let dlat = 1000.0 / 110_540.0;
        let p = |i: f64| (41.0 + i * dlat, 2.0);
        Route {
            ids: vec!["A".into(), "B".into(), "C".into()],
            cantons: vec![vec![(p(0.0).0, p(0.0).1, 0.0), (p(1.0).0, p(1.0).1, 1000.0)], vec![(p(1.0).0, p(1.0).1, 1000.0), (p(2.0).0, p(2.0).1, 2000.0)]],
            cum: vec![0.0, 1000.0, 2000.0],
        }
    }

    #[test]
    fn distancia_a_canton_y_fraccion_como_el_motor() {
        let r = ruta();
        assert_eq!(r.sample(0.0), None); // en cabecera antes de salir no se enseña
        assert_eq!(r.sample(500.0), Some((0, 1, 0.5)));
        assert_eq!(r.sample(1000.0), Some((0, 1, 1.0))); // parado en B = final del cantón A→B
        assert_eq!(r.sample(1500.0), Some((1, 2, 0.5)));
        assert_eq!(r.sample(2000.0), Some((1, 2, 1.0)));
    }

    #[test]
    fn una_posicion_congelada_adelanta_la_salida() {
        // Parado en s=0 de t=0 a t=100 (dos veces la misma posición) y a t=120 ya va 2 km más allá:
        // imposible arrancando a las 100 → salió antes.
        let mut tr = vec![(0.0, 0.0, 0.0), (100.0, 0.0, 0.0), (120.0, 2000.0, 0.0)];
        descongelar(&mut tr);
        let salida = 120.0 - t_desde_parado(2000.0);
        assert!((tr[1].0 - salida).abs() < 1e-9, "salida {}", tr[1].0);
        // Si sí da tiempo (100 m en 20 s), no se toca.
        let mut ok = vec![(0.0, 0.0, 0.0), (100.0, 0.0, 0.0), (120.0, 100.0, 0.0)];
        descongelar(&mut ok);
        assert_eq!(ok[1].0, 100.0);
    }

    #[test]
    fn proyeccion_sobre_la_via_del_recorrido() {
        let r = ruta();
        let dlat = 1000.0 / 110_540.0;
        // A 1,25 km del origen y 30 m al este de la vía.
        let (s, d) = r.project(41.0 + 1.25 * dlat, 2.0 + 30.0 / (111_320.0 * 41.0f64.to_radians().cos()), 0, 1).unwrap();
        assert!((s - 1250.0).abs() < 2.0, "s = {s}");
        assert!((d - 30.0).abs() < 1.0, "d = {d}");
    }
}
