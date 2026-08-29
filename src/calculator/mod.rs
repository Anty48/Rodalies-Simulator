//! Calculador de **tiempo mínimo teórico** entre estaciones para varias series de tren.
//!
//! Es una sección independiente del resto de la app (no toca el simulador/optimizador):
//!   * `rolling_stock` — base de datos de series (datos con procedencia).
//!   * `infrastructure` — ruta real, distancia por polilínea y velocidades observadas.
//!   * `physics` — integrador del movimiento con frenado anticipado (+ tests).
//!   * `render` — HTML del panel y del fragmento de resultados.
//!
//! Flujo: `compute(net, origen, destino, series, dt)` → `CalcView` (datos ya listos
//! para pintar, sin salida por consola), igual que `scenario` para el dashboard.

pub mod infrastructure;
pub mod line_analysis;
pub mod ltv;
pub mod physics;
pub mod render;
pub mod rolling_stock;
pub mod schedules;

use crate::gtfs_loader::Network;
use infrastructure::{ObservedSegment, Route};
use physics::{Restriction, Vehicle};
use rolling_stock::Provenance;

/// Una fase del movimiento (aceleración / crucero / frenada) ya agregada.
#[derive(Debug, Clone)]
pub struct Phase {
    pub t0: f64,
    pub t1: f64,
    pub kind: String,
    pub detail: String,
}

/// Resultado del cálculo para una serie concreta.
#[derive(Debug, Clone)]
pub struct TrainResult {
    pub id: String,
    pub name: String,
    pub available: bool,
    pub time_s: f64,
    pub vmax_reached_kmh: f64,
    pub vmax_ficha_kmh: f64,
    pub reached_end: bool,
    pub phases: Vec<Phase>,
    /// Trayectoria velocidad–distancia: (km, km/h).
    pub xv: Vec<(f64, f64)>,
    /// Trayectoria velocidad–tiempo: (s, km/h).
    pub tv: Vec<(f64, f64)>,
    pub note: String,
    /// Procedencia "peor" de la ficha (aviso global de fiabilidad de los datos).
    pub prov: Provenance,
    /// Tiempo con la CVM real de ADIF aplicada (modelo refinado). `None` si sin datos.
    pub time_ref_s: Option<f64>,
    pub vmax_ref_reached_kmh: Option<f64>,
}

/// Una estación de la ruta para la tabla.
#[derive(Debug, Clone)]
pub struct RouteRow {
    pub name: String,
    pub cum_km: f64,
    pub arr: Option<u32>,
}

/// Vista completa del calculador (entrada del render).
#[derive(Debug, Clone)]
pub struct CalcView {
    pub origin_name: String,
    pub dest_name: String,
    pub line: Option<String>,
    pub route: Vec<RouteRow>,
    pub distance_km: f64,
    pub distance_source: String,
    pub has_times: bool,
    pub observed: Vec<ObservedSegment>,
    pub results: Vec<TrainResult>,
    pub dt: f64,
    /// CVM ADIF aplicada.
    pub adif_available: bool,
    pub adif_distance_km: Option<f64>,
    pub coverage_pct: f64,
    pub min_vmax_kmh: Option<f64>,
    /// LTV aplicadas (temporales).
    pub ltv_applied: usize,
    pub min_ltv_kmh: Option<f64>,
    pub ltv_snapshot: Option<String>,
    pub error: Option<String>,
}

/// Umbral de aceleración (m/s²) para clasificar una fase como acel./frenada.
const A_THRESH: f64 = 0.06;
/// Duración mínima de una fase (s); las más cortas se funden con la anterior.
const MIN_PHASE_S: f64 = 3.0;

/// Extrae las fases del movimiento a partir de la traza.
fn extract_phases(trace: &[physics::Trace]) -> Vec<Phase> {
    if trace.is_empty() {
        return Vec::new();
    }
    let kind_of = |a: f64| -> &'static str {
        if a > A_THRESH {
            "Aceleración"
        } else if a < -A_THRESH {
            "Frenada"
        } else {
            "Velocidad constante"
        }
    };
    // Fases crudas.
    struct Raw {
        t0: f64,
        t1: f64,
        kind: &'static str,
        v0: f64,
        v1: f64,
    }
    let mut raw: Vec<Raw> = Vec::new();
    let mut prev_t = 0.0;
    for s in trace {
        let k = kind_of(s.a);
        match raw.last_mut() {
            Some(r) if r.kind == k => {
                r.t1 = s.t;
                r.v1 = s.v;
            }
            _ => raw.push(Raw { t0: prev_t, t1: s.t, kind: k, v0: s.v, v1: s.v }),
        }
        prev_t = s.t;
    }
    // Fundir fases demasiado cortas con la anterior.
    let mut merged: Vec<Raw> = Vec::new();
    for r in raw {
        if let Some(last) = merged.last_mut() {
            if r.t1 - r.t0 < MIN_PHASE_S {
                last.t1 = r.t1;
                last.v1 = r.v1;
                continue;
            }
        }
        merged.push(r);
    }
    merged
        .into_iter()
        .map(|r| {
            let detail = match r.kind {
                "Velocidad constante" => {
                    format!("≈ {:.0} km/h", (r.v0 + r.v1) / 2.0 * 3.6)
                }
                _ => format!("{:.0} → {:.0} km/h", r.v0 * 3.6, r.v1 * 3.6),
            };
            Phase { t0: r.t0, t1: r.t1, kind: r.kind.to_string(), detail }
        })
        .collect()
}

/// Reduce una traza a como mucho `max_pts` puntos (km, km/h) o (s, km/h).
fn downsample(trace: &[physics::Trace], by_distance: bool, max_pts: usize) -> Vec<(f64, f64)> {
    if trace.is_empty() {
        return Vec::new();
    }
    let step = (trace.len() / max_pts).max(1);
    let mut out: Vec<(f64, f64)> = Vec::new();
    for (i, s) in trace.iter().enumerate() {
        if i % step == 0 {
            let xy = if by_distance {
                (s.x / 1000.0, s.v * 3.6)
            } else {
                (s.t, s.v * 3.6)
            };
            out.push(xy);
        }
    }
    // Asegurar el último punto.
    let last = trace.last().unwrap();
    let last_xy = if by_distance {
        (last.x / 1000.0, last.v * 3.6)
    } else {
        (last.t, last.v * 3.6)
    };
    if out.last() != Some(&last_xy) {
        out.push(last_xy);
    }
    out
}

/// Construye el `Vehicle` (SI) de una serie a partir de su ficha.
fn vehicle_of(s: &rolling_stock::TrainSeries) -> Vehicle {
    Vehicle {
        vmax: s.vmax_kmh.value / 3.6,
        power_w: s.power_kw.value * 1000.0,
        mass_kg: s.mass_t.value * 1000.0,
        accel_start: s.accel_start.value,
        decel: s.decel_service.value,
        efficiency: s.efficiency.value,
        rotary: s.rotary_mass.value,
        // Resistencia al avance desactivada por defecto (coeficientes por serie no
        // publicados). El modelo refinado se basa en la CVM oficial de ADIF, no en
        // una resistencia estimada, para que la comparación sea "oficial vs oficial".
        res_a: 0.0,
        res_b: 0.0,
        res_c: 0.0,
    }
}

/// Punto de entrada: calcula la vista del calculador.
pub fn compute(
    net: &Network,
    origin_id: &str,
    dest_id: &str,
    series_ids: &[String],
    dt: f64,
    adif: Option<&infrastructure::AdifNet>,
    ltv: Option<&ltv::LtvSet>,
) -> CalcView {
    let origin_name = net.stop_name(origin_id).to_string();
    let dest_name = net.stop_name(dest_id).to_string();
    let dt = if dt.is_finite() && dt > 0.0 { dt.clamp(0.02, 1.0) } else { 0.1 };

    // 1. Ruta.
    let route: Route = match infrastructure::find_route(net, origin_id, dest_id) {
        Some(r) => r,
        None => {
            return CalcView {
                origin_name,
                dest_name,
                line: None,
                route: Vec::new(),
                distance_km: 0.0,
                distance_source: String::new(),
                has_times: false,
                observed: Vec::new(),
                results: Vec::new(),
                dt,
                adif_available: false,
                adif_distance_km: None,
                coverage_pct: 0.0,
                min_vmax_kmh: None,
                ltv_applied: 0,
                min_ltv_kmh: None,
                ltv_snapshot: None,
                error: Some(
                    "No se ha encontrado ninguna ruta ferroviaria entre esas estaciones \
                     (¿misma red y sentido con servicios en el GTFS?)."
                        .into(),
                ),
            };
        }
    };

    let length_m = route.distance_m;
    let observed = infrastructure::observed_speeds(&route);
    let zones = infrastructure::speed_zones(&route);
    // Restricción obligatoria: parada en el destino (v = 0).
    let restrictions = vec![Restriction { x: length_m, v: 0.0 }];

    // Perfil ADIF (CVM real) para la ruta completa.
    let pts: Vec<(f64, f64)> = route
        .stops
        .iter()
        .filter_map(|s| Some((s.lat?, s.lon?)))
        .collect();
    let profile = adif.and_then(|a| infrastructure::adif_profile(a, ltv, &pts, 40.0, 140.0));
    let ref_zones: Vec<physics::SpeedZone> = profile
        .as_ref()
        .map(|p| {
            p.zones
                .iter()
                .map(|z| physics::SpeedZone { from: z.from_m, to: z.to_m, vmax: z.vmax_kmh / 3.6 })
                .collect()
        })
        .unwrap_or_default();
    let refine = !ref_zones.is_empty();
    let ref_restrictions = physics::restrictions_from_zones(&ref_zones, length_m);
    let adif_available = refine;
    let adif_distance_km = profile.as_ref().map(|p| p.adif_distance_m / 1000.0);
    let coverage_pct = profile.as_ref().map(|p| p.coverage * 100.0).unwrap_or(0.0);
    let min_vmax_kmh = profile.as_ref().and_then(|p| p.min_vmax_kmh);

    // 2. Simular cada serie.
    let mut results = Vec::new();
    for id in series_ids {
        let Some(s) = rolling_stock::get(id) else { continue };
        if !s.available {
            results.push(TrainResult {
                id: s.id.to_string(),
                name: s.name.to_string(),
                available: false,
                time_s: 0.0,
                vmax_reached_kmh: 0.0,
                vmax_ficha_kmh: 0.0,
                reached_end: false,
                phases: Vec::new(),
                xv: Vec::new(),
                tv: Vec::new(),
                note: s.notes.to_string(),
                prov: Provenance::NoDisponible,
                time_ref_s: None,
                vmax_ref_reached_kmh: None,
            });
            continue;
        }
        let prov = worst_provenance(&s);
        let veh = vehicle_of(&s);
        let sim = physics::simulate(length_m, &veh, &zones, &restrictions, dt);
        let (time_ref_s, vmax_ref_reached_kmh) = if refine {
            let sr = physics::simulate(length_m, &veh, &ref_zones, &ref_restrictions, dt);
            (Some(sr.time_s), Some(sr.vmax_reached * 3.6))
        } else {
            (None, None)
        };
        results.push(TrainResult {
            id: s.id.to_string(),
            name: s.name.to_string(),
            available: true,
            time_s: sim.time_s,
            vmax_reached_kmh: sim.vmax_reached * 3.6,
            vmax_ficha_kmh: s.vmax_kmh.value,
            reached_end: sim.reached_end,
            phases: extract_phases(&sim.trace),
            xv: downsample(&sim.trace, true, 300),
            tv: downsample(&sim.trace, false, 300),
            note: s.notes.to_string(),
            prov,
            time_ref_s,
            vmax_ref_reached_kmh,
        });
    }

    let route_rows = route
        .stops
        .iter()
        .map(|s| RouteRow {
            name: s.name.clone(),
            cum_km: s.cum_m / 1000.0,
            arr: s.arr,
        })
        .collect();

    CalcView {
        origin_name,
        dest_name,
        line: route.line.clone(),
        route: route_rows,
        distance_km: length_m / 1000.0,
        distance_source: route.source.clone(),
        has_times: route.has_times,
        observed,
        results,
        dt,
        adif_available,
        adif_distance_km,
        coverage_pct,
        min_vmax_kmh,
        ltv_applied: profile.as_ref().map(|p| p.ltv_applied).unwrap_or(0),
        min_ltv_kmh: profile.as_ref().and_then(|p| p.min_ltv_kmh),
        ltv_snapshot: ltv.map(|l| l.snapshot.clone()),
        error: None,
    }
}

/// Nivel de procedencia "peor" (más incierto) presente en la ficha de una serie, para
/// pintar un aviso global. Orden: NoDisponible > Suposición > Estimación > Secundaria >
/// Oficial.
pub fn worst_provenance(s: &rolling_stock::TrainSeries) -> Provenance {
    [
        s.vmax_kmh.prov,
        s.power_kw.prov,
        s.mass_t.prov,
        s.accel_start.prov,
        s.decel_service.prov,
    ]
    .into_iter()
    .max_by_key(|p| p.rank())
    .unwrap_or(Provenance::Oficial)
}
