//! Infraestructura para el calculador: **ruta real** entre dos estaciones, **distancia
//! ferroviaria** y **perfil de velocidades**.
//!
//! ## Distancia (honestidad sobre la fuente)
//!
//! El encargo pide, por orden de prioridad: (1) PK oficial, (2) geometría real de la
//! vía, (3) otra fuente ferroviaria, (4) coordenadas + trazado como último recurso.
//!
//! En el repositorio **no hay** dataset de PK ni de geometría de vía de Adif (RINF/CVM),
//! así que se usa la opción (4) de forma lo más fiel posible: se sigue la **secuencia
//! real de estaciones** que recorre un tren entre origen y destino (según el GTFS) y se
//! suma la distancia geodésica entre estaciones consecutivas (**polilínea de
//! estaciones**). NO es una única línea recta origen→destino: sigue el encadenado real
//! de paradas. Aun así **infravalora** la longitud real de vía (ignora la sinuosidad
//! entre estaciones). Se marca claramente como aproximación y el módulo está preparado
//! para enchufar una tabla de PK oficial cuando se disponga de ella (`OFFICIAL_PK`).
//!
//! ## Perfil de velocidades
//!
//! No hay Cuadro de Velocidades Máximas (CVM) de Adif disponible, así que NO se inventan
//! límites por tramo. El perfil de infraestructura queda "sin dato" (manda la Vmax del
//! tren) y, como dato REAL de contexto, se calcula la **velocidad comercial media
//! observada** por tramo a partir de los horarios GTFS.

use std::collections::HashMap;
use std::path::Path;

use petgraph::algo::astar;
use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;

use crate::gtfs_loader::Network;

/// Radio medio terrestre (m).
const EARTH_R: f64 = 6_371_000.0;

/// Distancia geodésica (haversine) entre dos puntos lat/lon, en metros.
pub fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (r1, r2) = (lat1.to_radians(), lat2.to_radians());
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2) + r1.cos() * r2.cos() * (dlon / 2.0).sin().powi(2);
    2.0 * EARTH_R * a.sqrt().asin()
}

/// Una estación dentro de la ruta calculada.
#[derive(Debug, Clone)]
pub struct RouteStop {
    pub name: String,
    /// Distancia acumulada desde el origen (m).
    pub cum_m: f64,
    /// Hora de llegada/salida (s desde medianoche) si la ruta viene de un servicio real.
    pub arr: Option<u32>,
    pub dep: Option<u32>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
}

/// Ruta ferroviaria entre dos estaciones.
#[derive(Debug, Clone)]
pub struct Route {
    pub stops: Vec<RouteStop>,
    pub distance_m: f64,
    /// Descripción de la fuente/método de la distancia (para mostrar en la UI).
    pub source: String,
    /// `true` si la ruta proviene de un servicio real (con horarios → hay velocidades).
    pub has_times: bool,
    /// Línea del servicio de referencia, si aplica.
    pub line: Option<String>,
}

/// Segmento entre dos estaciones consecutivas con su velocidad comercial observada.
#[derive(Debug, Clone)]
pub struct ObservedSegment {
    pub from: String,
    pub to: String,
    pub dist_m: f64,
    pub run_s: u32,
    pub avg_kmh: f64,
}

fn coord(net: &Network, node: NodeIndex) -> Option<(f64, f64)> {
    let n = &net.graph[node];
    match (n.lat, n.lon) {
        (Some(la), Some(lo)) => Some((la, lo)),
        _ => None,
    }
}

/// Suma la polilínea de estaciones de una secuencia de nodos → distancia (m) y las
/// distancias acumuladas por estación.
fn polyline(net: &Network, nodes: &[NodeIndex]) -> (f64, Vec<f64>) {
    let mut cum = vec![0.0f64];
    let mut total = 0.0f64;
    for w in nodes.windows(2) {
        let d = match (coord(net, w[0]), coord(net, w[1])) {
            (Some((a, b)), Some((c, e))) => haversine_m(a, b, c, e),
            _ => 0.0,
        };
        total += d;
        cum.push(total);
    }
    (total, cum)
}

/// Busca un **servicio real** (no bus) cuyo horario contenga origen y destino en ese
/// orden; devuelve el que aporte MÁS estaciones intermedias (mejor resolución de la
/// polilínea). Devuelve los `ScheduledStop` del tramo origen→destino.
fn covering_service<'a>(
    net: &'a Network,
    origin: &str,
    dest: &str,
) -> Option<&'a crate::gtfs_loader::TrainService> {
    let mut best: Option<&crate::gtfs_loader::TrainService> = None;
    let mut best_len = 0usize;
    for svc in &net.services {
        if svc.is_bus {
            continue;
        }
        let io = svc.schedule.iter().position(|s| s.stop_id == origin);
        let id = svc.schedule.iter().position(|s| s.stop_id == dest);
        if let (Some(io), Some(idx)) = (io, id) {
            if io < idx {
                let len = idx - io;
                if len > best_len {
                    best_len = len;
                    best = Some(svc);
                }
            }
        }
    }
    best
}

/// Calcula la ruta entre dos `stop_id`. Estrategia:
///   1. Servicio real que cubra origen→destino (da ruta + horarios → velocidades).
///   2. Si no hay, camino más corto en el grafo por distancia geodésica (sin horarios).
pub fn find_route(net: &Network, origin: &str, dest: &str) -> Option<Route> {
    // --- 1. Servicio de cobertura ---
    if let Some(svc) = covering_service(net, origin, dest) {
        let io = svc.schedule.iter().position(|s| s.stop_id == origin).unwrap();
        let idx = svc.schedule.iter().position(|s| s.stop_id == dest).unwrap();
        let slice = &svc.schedule[io..=idx];
        let nodes: Vec<NodeIndex> =
            slice.iter().filter_map(|s| net.node(&s.stop_id)).collect();
        if nodes.len() == slice.len() && nodes.len() >= 2 {
            let (total, cum) = polyline(net, &nodes);
            let stops = slice
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let node = net.node(&s.stop_id).map(|n| &net.graph[n]);
                    RouteStop {
                        name: net.stop_name(&s.stop_id).to_string(),
                        cum_m: cum[i],
                        arr: Some(s.arrival_sec),
                        dep: Some(s.departure_sec),
                        lat: node.and_then(|n| n.lat),
                        lon: node.and_then(|n| n.lon),
                    }
                })
                .collect();
            return Some(Route {
                stops,
                distance_m: total,
                source: format!(
                    "Polilínea de estaciones GTFS a lo largo del recorrido real del \
                     servicio {} (línea {}). Aproximación (no PK oficial).",
                    svc.train_number, svc.route_short_name
                ),
                has_times: true,
                line: Some(svc.route_short_name.clone()),
            });
        }
    }

    // --- 2. Camino más corto en el grafo (distancia geodésica) ---
    let (a, b) = (net.node(origin)?, net.node(dest)?);
    let path = astar(
        &net.graph,
        a,
        |n| n == b,
        |e| {
            let (s, t) = (e.source(), e.target());
            match (coord(net, s), coord(net, t)) {
                (Some((la, lo)), Some((lc, ld))) => haversine_m(la, lo, lc, ld),
                _ => 1e9, // penaliza aristas sin coordenadas
            }
        },
        |_| 0.0,
    );
    let (_, nodes) = path?;
    if nodes.len() < 2 {
        return None;
    }
    let (total, cum) = polyline(net, &nodes);
    let stops = nodes
        .iter()
        .enumerate()
        .map(|(i, &nd)| {
            let sn = &net.graph[nd];
            RouteStop {
                name: sn.stop_name.clone(),
                cum_m: cum[i],
                arr: None,
                dep: None,
                lat: sn.lat,
                lon: sn.lon,
            }
        })
        .collect();
    Some(Route {
        stops,
        distance_m: total,
        source: "Camino más corto en el grafo de estaciones (distancia geodésica por \
                 polilínea). Aproximación (no PK oficial, sin horario de referencia)."
            .to_string(),
        has_times: false,
        line: None,
    })
}

/// Velocidades comerciales medias observadas por tramo (solo si la ruta trae horarios).
pub fn observed_speeds(route: &Route) -> Vec<ObservedSegment> {
    let mut out = Vec::new();
    for w in route.stops.windows(2) {
        let (Some(dep), Some(arr)) = (w[0].dep, w[1].arr) else { continue };
        let run = arr.saturating_sub(dep);
        let dist = w[1].cum_m - w[0].cum_m;
        if run == 0 || dist <= 0.0 {
            continue;
        }
        out.push(ObservedSegment {
            from: w[0].name.clone(),
            to: w[1].name.clone(),
            dist_m: dist,
            run_s: run,
            avg_kmh: dist / run as f64 * 3.6,
        });
    }
    out
}

/// Perfil de velocidades máximas de la INFRAESTRUCTURA por tramo.
///
/// No hay CVM de Adif disponible → se devuelve vacío (sin límites de vía conocidos: en
/// el modelo físico manda la Vmax del propio tren). El módulo está preparado para
/// rellenar aquí zonas reales cuando se disponga del Cuadro de Velocidades Máximas.
pub fn speed_zones(_route: &Route) -> Vec<crate::calculator::physics::SpeedZone> {
    Vec::new()
}

// --------------------------------------------------------------------------
// Infraestructura ADIF (velocidad de diseño + geometría real) — Fase 3
// --------------------------------------------------------------------------
//
// Carga `processed/adif/rfig_speed.json` (generado por scripts/fetch_adif_cvm.py desde
// el WFS INSPIRE de IDEADIF) y ofrece, para una ruta, el **perfil de velocidad máxima
// de la infraestructura** por proyección al enlace ADIF más cercano, además de una
// **distancia por geometría ADIF** (proyectada) para comparar con la polilínea GTFS.
// Si el fichero no existe, todo esto es `None` y el modelo cae al comportamiento previo.

const CELL: f64 = 0.01; // ~1,1 km por celda de la rejilla espacial

/// Red ADIF: segmentos de vía con su velocidad de diseño + índice de rejilla.
pub struct AdifNet {
    /// (a[lat,lon], b[lat,lon], vmax_km/h) de cada segmento elemental de vía.
    segs: Vec<([f64; 2], [f64; 2], f64)>,
    grid: std::collections::HashMap<(i32, i32), Vec<u32>>,
}

fn cell_key(lat: f64, lon: f64) -> (i32, i32) {
    ((lat / CELL).floor() as i32, (lon / CELL).floor() as i32)
}

/// Distancia (m) punto→segmento en un marco local equirectangular, y punto proyectado.
fn pt_seg_dist_m(lat: f64, lon: f64, a: [f64; 2], b: [f64; 2], coslat: f64) -> (f64, [f64; 2]) {
    const K: f64 = 111_320.0;
    let (px, py) = (lon * coslat * K, lat * K);
    let (ax, ay) = (a[1] * coslat * K, a[0] * K);
    let (bx, by) = (b[1] * coslat * K, b[0] * K);
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    let t = if len2 <= 1e-9 { 0.0 } else { (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0) };
    let (cx, cy) = (ax + t * dx, ay + t * dy);
    let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
    (d, [cy / K, cx / (coslat * K)])
}

impl AdifNet {
    /// Carga el JSON procesado. Devuelve `None` si falta o está corrupto (→ fallback).
    pub fn load(path: &Path) -> Option<AdifNet> {
        let txt = std::fs::read_to_string(path).ok()?;
        let v: serde_json::Value = serde_json::from_str(&txt).ok()?;
        let arr = v.as_array()?;
        let mut segs: Vec<([f64; 2], [f64; 2], f64)> = Vec::new();
        for f in arr {
            let sp = f.get("speed_kmh").and_then(|x| x.as_f64()).unwrap_or(0.0);
            if sp <= 0.0 {
                continue;
            }
            let Some(coords) = f.get("coords").and_then(|c| c.as_array()) else { continue };
            let mut prev: Option<[f64; 2]> = None;
            for c in coords {
                let Some(cc) = c.as_array() else { continue };
                let (Some(lat), Some(lon)) = (cc.first().and_then(|x| x.as_f64()), cc.get(1).and_then(|x| x.as_f64())) else { continue };
                let p = [lat, lon];
                if let Some(q) = prev {
                    segs.push((q, p, sp));
                }
                prev = Some(p);
            }
        }
        if segs.is_empty() {
            return None;
        }
        let mut grid: std::collections::HashMap<(i32, i32), Vec<u32>> = std::collections::HashMap::new();
        for (i, (a, b, _)) in segs.iter().enumerate() {
            let (mlat, mlon) = ((a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0);
            grid.entry(cell_key(mlat, mlon)).or_default().push(i as u32);
        }
        Some(AdifNet { segs, grid })
    }

    pub fn n_segments(&self) -> usize {
        self.segs.len()
    }

    /// Velocidad ADIF (km/h) y punto proyectado del enlace más cercano dentro de `tol_m`.
    fn nearest(&self, lat: f64, lon: f64, tol_m: f64) -> Option<(f64, [f64; 2])> {
        let (ci, cj) = cell_key(lat, lon);
        let coslat = lat.to_radians().cos();
        let mut best: Option<(f64, f64, [f64; 2])> = None; // (dist, speed, proj)
        for di in -1..=1 {
            for dj in -1..=1 {
                if let Some(v) = self.grid.get(&(ci + di, cj + dj)) {
                    for &idx in v {
                        let (a, b, sp) = self.segs[idx as usize];
                        let (d, proj) = pt_seg_dist_m(lat, lon, a, b, coslat);
                        if d <= tol_m && best.map_or(true, |(bd, _, _)| d < bd) {
                            best = Some((d, sp, proj));
                        }
                    }
                }
            }
        }
        best.map(|(_, sp, proj)| (sp, proj))
    }
}

/// Una zona de velocidad de infraestructura a lo largo de la ruta.
#[derive(Debug, Clone, Copy)]
pub struct AdifZone {
    pub from_m: f64,
    pub to_m: f64,
    pub vmax_kmh: f64,
}

/// Perfil ADIF de una ruta: zonas de Vmax, cobertura y distancia por geometría ADIF.
#[derive(Debug, Clone)]
pub struct AdifProfile {
    pub zones: Vec<AdifZone>,
    /// Fracción [0,1] de la ruta con enlace ADIF cercano.
    pub coverage: f64,
    pub adif_distance_m: f64,
    /// Vmax mínima encontrada en la ruta (km/h), para avisos.
    pub min_vmax_kmh: Option<f64>,
    /// Nº de LTV distintas aplicadas en la ruta.
    pub ltv_applied: usize,
    /// Velocidad LTV mínima aplicada (km/h), si alguna.
    pub min_ltv_kmh: Option<f64>,
}

/// Interpola el punto a distancia `s` (m) a lo largo de la polilínea `pts` con
/// distancias acumuladas `cum`.
fn point_at(pts: &[(f64, f64)], cum: &[f64], s: f64) -> (f64, f64) {
    if s <= 0.0 {
        return pts[0];
    }
    let total = *cum.last().unwrap();
    if s >= total {
        return *pts.last().unwrap();
    }
    let mut i = 0;
    while i + 1 < cum.len() && cum[i + 1] < s {
        i += 1;
    }
    let seg = (cum[i + 1] - cum[i]).max(1e-9);
    let t = (s - cum[i]) / seg;
    (
        pts[i].0 + t * (pts[i + 1].0 - pts[i].0),
        pts[i].1 + t * (pts[i + 1].1 - pts[i].1),
    )
}

/// Construye el perfil de infraestructura de una ruta combinando la velocidad de diseño
/// de ADIF (DesignSpeed) con las LTV (si se pasan): `Vmax(x)=min(DesignSpeed, LTV)`.
/// Las LTV se activan al pasar cerca de su punto de inicio y se mantienen a lo largo de
/// la ruta durante su longitud por PK (`span_m`).
/// `step_m` = paso de muestreo; `tol_m` = tolerancia de proximidad al enlace.
pub fn adif_profile(
    adif: &AdifNet,
    ltv: Option<&crate::calculator::ltv::LtvSet>,
    pts: &[(f64, f64)],
    step_m: f64,
    tol_m: f64,
) -> Option<AdifProfile> {
    if pts.len() < 2 {
        return None;
    }
    // Distancias acumuladas de la polilínea de estaciones (GTFS).
    let mut cum = vec![0.0f64];
    for w in pts.windows(2) {
        let d = haversine_m(w[0].0, w[0].1, w[1].0, w[1].1);
        cum.push(cum.last().unwrap() + d);
    }
    let total = *cum.last().unwrap();
    if total <= 0.0 {
        return None;
    }

    // Muestreo.
    let n = ((total / step_m).ceil() as usize).max(1);
    let mut zones: Vec<AdifZone> = Vec::new();
    let mut matched = 0usize;
    let mut samples = 0usize;
    let mut min_v: Option<f64> = None;
    let mut adif_dist = 0.0f64;
    let mut prev_proj: Option<(f64, f64)> = None;
    // LTV activas: idx -> (metros restantes, velocidad km/h).
    let mut active: HashMap<usize, (f64, f64)> = HashMap::new();
    let mut applied: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut min_ltv: Option<f64> = None;

    for k in 0..=n {
        let s = (k as f64 * step_m).min(total);
        let (lat, lon) = point_at(pts, &cum, s);
        samples += 1;

        // Velocidad de diseño (DesignSpeed).
        let (design, proj_pt) = match adif.nearest(lat, lon, tol_m) {
            Some((sp, proj)) => {
                matched += 1;
                (Some(sp), (proj[0], proj[1]))
            }
            None => (None, (lat, lon)),
        };
        if let Some(pp) = prev_proj {
            adif_dist += haversine_m(pp.0, pp.1, proj_pt.0, proj_pt.1);
        }
        prev_proj = Some(proj_pt);

        // Activar LTV cuyo inicio está cerca de esta muestra.
        if let Some(lset) = ltv {
            for (idx, sp, span) in lset.nearby(lat, lon, 250.0) {
                let cover = span.max(step_m);
                let e = active.entry(idx).or_insert((0.0, sp));
                e.0 = e.0.max(cover);
                e.1 = sp;
                applied.insert(idx);
                min_ltv = Some(min_ltv.map_or(sp, |m: f64| m.min(sp)));
            }
        }
        // Cap LTV vigente = mínima velocidad de las LTV activas.
        let ltv_cap = active.values().map(|(_, sp)| *sp).fold(f64::INFINITY, f64::min);
        let ltv_cap = if ltv_cap.is_finite() { Some(ltv_cap) } else { None };

        // Combinar min(design, ltv).
        let combined = match (design, ltv_cap) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };
        if let Some(sp) = combined {
            min_v = Some(min_v.map_or(sp, |m: f64| m.min(sp)));
            match zones.last_mut() {
                Some(z) if (z.vmax_kmh - sp).abs() < 0.5 => z.to_m = s,
                _ => zones.push(AdifZone { from_m: s, to_m: s, vmax_kmh: sp }),
            }
        }

        // Consumir la longitud de las LTV activas (avanza la ruta un paso).
        active.retain(|_, (rem, _)| {
            *rem -= step_m;
            *rem > 0.0
        });
    }

    Some(AdifProfile {
        zones,
        coverage: matched as f64 / samples.max(1) as f64,
        adif_distance_m: adif_dist,
        min_vmax_kmh: min_v,
        ltv_applied: applied.len(),
        min_ltv_kmh: min_ltv,
    })
}

/// Índice auxiliar: lista de estaciones (stop_id, nombre) que participan en algún
/// servicio, deduplicadas por nombre y ordenadas alfabéticamente. Alimenta los
/// desplegables de la UI.
pub fn station_list(net: &Network) -> Vec<(String, String)> {
    let mut by_name: HashMap<String, String> = HashMap::new();
    for svc in &net.services {
        for s in &svc.schedule {
            let name = net.stop_name(&s.stop_id).to_string();
            by_name.entry(name).or_insert_with(|| s.stop_id.clone());
        }
    }
    let mut list: Vec<(String, String)> =
        by_name.into_iter().map(|(name, id)| (id, name)).collect();
    list.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
    list
}
