//! Trazado REAL de vía por cantón (polilínea sobre la red ferroviaria de OpenStreetMap) y
//! BIFURCACIONES fuera de estación.
//!
//! Sin esto, cada cantón medía la LÍNEA RECTA entre estaciones. Donde la vía da un rodeo —Estació
//! de França → Passeig de Gràcia sale hacia el Clot y vuelve por el túnel de Aragó: 4,6 km de vía
//! frente a 1,9 km de recta— el tren físico llegaba en 2 min a un tramo que el horario recorre en
//! 10, y se quedaba ocupando el andén de destino esperando su hora. Con la polilínea real:
//!   * la longitud del cantón (`cum_l` del motor) es la de la vía,
//!   * el perfil de Vmax (ADIF/OSM) se muestrea SOBRE la curva, no sobre una recta que puede pasar
//!     por encima de otras vías,
//!   * la densidad de señales de tramo sale de la longitud real,
//!   * y el visor dibuja las curvas y mueve los trenes por ellas.
//!
//! BIFURCACIONES: dos cantones que salen de una estación por la MISMA vía y se separan más allá
//! (El Prat → Aeroport / Viladecans, Arc de Triomf → Clot / La Sagrera en Glòries…) comparten un
//! tramo común. Cada bifurcación es un NODO de paso ("J<n>") con coordenadas: el cantón A→B pasa a
//! ser A→J→B, así el tramo A→J es el MISMO cantón físico para los dos ramales (se bloquean entre
//! sí, como en la realidad) y los trenes se ven sobre la vía común hasta la aguja.
//!
//! APARTADEROS DE CRUCE: en vía única, las vías de cruce sin parada de viajeros (estaciones cerradas,
//! puestos de cruce: Chiprana, Val de Pilas…) también son nodos de paso ("A<n>", `passing_loop`). El
//! tren no para en ellos por horario, pero el motor puede detenerlo allí para CRUZARSE con uno de
//! frente en vez de retenerlo en la estación anterior todo el tramo (Samper → Caspe son 30 km).
//!
//! Datos: `processed/adif/track_geometry.json`, generado por `scripts/fetch_osm_track.py`. Si falta
//! el fichero o un cantón, se usa la recta (comportamiento anterior) — nunca es obligatorio.

use crate::calculator::infrastructure::haversine_m;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

pub const DEFAULT_PATH: &str = "processed/adif/track_geometry.json";

/// Polilínea de un cantón dirigido (lat, lon) y su longitud (m) medida sobre ESA polilínea (así
/// coincide con las distancias que usa `adif_profile` al muestrearla).
pub struct CantonGeom {
    pub pts: Vec<(f64, f64)>,
    pub len_m: f64,
}

/// Nodo de paso fuera de estación: BIFURCACIÓN (punto de divergencia de dos o más ramales) o
/// APARTADERO DE CRUCE de vía única sin parada de viajeros.
pub struct Junction {
    pub lat: f64,
    pub lon: f64,
    /// Nombre legible: "Bif. <estación de la que sale el tramo común>" o
    /// "Apartadero <estación>–<estación> (km x)".
    pub name: String,
    /// Apartadero de cruce (vía principal + vía de apartado): dos trenes se pueden cruzar en él.
    pub passing_loop: bool,
}

pub struct TrackGeometry {
    cantons: HashMap<(String, String), CantonGeom>,
    junctions: HashMap<String, Junction>,
    /// Bifurcaciones por las que pasa cada cantón ORIGINAL (estación→estación) y su distancia (m)
    /// desde el origen, en orden de paso.
    via: HashMap<(String, String), Vec<(String, f64)>>,
}

#[derive(Deserialize)]
struct RawFile {
    cantons: HashMap<String, RawCanton>,
    #[serde(default)]
    junctions: HashMap<String, RawJunction>,
}
#[derive(Deserialize)]
struct RawCanton {
    coords: Vec<[f64; 2]>,
    #[serde(default)]
    via: Vec<(String, f64)>,
}
#[derive(Deserialize)]
struct RawJunction {
    lat: f64,
    lon: f64,
    near: String,
    /// Apartadero de cruce: estación del otro extremo del cantón y km desde `near`.
    #[serde(default, rename = "loop")]
    passing_loop: bool,
    #[serde(default)]
    far: String,
    #[serde(default)]
    km: f64,
}

impl TrackGeometry {
    /// `stop_names`: stop_id → nombre, para rotular las bifurcaciones.
    pub fn load(path: &str, stop_names: &HashMap<String, String>) -> Option<Self> {
        let raw: RawFile = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
        let junctions: HashMap<String, Junction> = raw
            .junctions
            .into_iter()
            .map(|(id, j)| {
                let near = stop_names.get(&j.near).cloned().unwrap_or(j.near);
                let name = if j.passing_loop {
                    let far = stop_names.get(&j.far).cloned().unwrap_or(j.far);
                    format!("Apartadero {near}–{far} (km {:.1})", j.km)
                } else {
                    format!("Bif. {near}")
                };
                (id, Junction { lat: j.lat, lon: j.lon, name, passing_loop: j.passing_loop })
            })
            .collect();
        let mut cantons = HashMap::with_capacity(raw.cantons.len());
        let mut via = HashMap::new();
        let mut pieces: Vec<((String, String), Vec<(f64, f64)>)> = Vec::new();
        // Orden FIJO por clave: el tramo común de una bifurcación aparece una vez por ramal (con
        // puntos de corte algo distintos) y abajo «se queda el primero». Recorriendo el `HashMap` del
        // JSON, el primero cambiaba en cada ejecución → la longitud del tramo común variaba unos
        // metros → la misma semilla daba jornadas distintas (un R15 llegaba a Sants con 3 o 22 min).
        let mut raw_cantons: Vec<(String, RawCanton)> = raw.cantons.into_iter().collect();
        raw_cantons.sort_by(|x, y| x.0.cmp(&y.0));
        for (k, c) in raw_cantons {
            let Some((a, b)) = k.split_once('>') else { continue };
            if c.coords.len() < 2 {
                continue;
            }
            let pts: Vec<(f64, f64)> = c.coords.iter().map(|p| (p[0], p[1])).collect();
            let v: Vec<(String, f64)> = c.via.into_iter().filter(|(j, _)| junctions.contains_key(j)).collect();
            if !v.is_empty() {
                // Trozos A→J1, J1→J2, …, Jn→B de la polilínea (los comparten los ramales).
                let dists: Vec<f64> = v.iter().map(|x| x.1).collect();
                let parts = split_polyline(&pts, &dists);
                let mut nodes = vec![a.to_string()];
                nodes.extend(v.iter().map(|x| x.0.clone()));
                nodes.push(b.to_string());
                for (w, part) in nodes.windows(2).zip(parts) {
                    pieces.push(((w[0].clone(), w[1].clone()), part));
                }
                via.insert((a.to_string(), b.to_string()), v);
            }
            let len_m = polyline_len(&pts);
            cantons.insert((a.to_string(), b.to_string()), CantonGeom { pts, len_m });
        }
        // Un tramo común aparece una vez por ramal: se queda el primero (son la misma vía).
        for (k, part) in pieces {
            if part.len() >= 2 {
                let len_m = polyline_len(&part);
                cantons.entry(k).or_insert(CantonGeom { pts: part, len_m });
            }
        }
        Some(TrackGeometry { cantons, junctions, via })
    }

    /// Trazado del cantón `a → b`. Si solo existe el sentido contrario (misma vía física) se
    /// devuelve invertido.
    pub fn get(&self, a: &str, b: &str) -> Option<CantonGeom> {
        if let Some(g) = self.cantons.get(&(a.to_string(), b.to_string())) {
            return Some(CantonGeom { pts: g.pts.clone(), len_m: g.len_m });
        }
        self.cantons.get(&(b.to_string(), a.to_string())).map(|g| CantonGeom {
            pts: g.pts.iter().rev().copied().collect(),
            len_m: g.len_m,
        })
    }

    /// Longitud de vía (m) del cantón en cualquier sentido, sin copiar la polilínea (se consulta
    /// por cada cantón de cada tren al construir el motor).
    pub fn len_m(&self, a: &str, b: &str) -> Option<f64> {
        self.cantons
            .get(&(a.to_string(), b.to_string()))
            .or_else(|| self.cantons.get(&(b.to_string(), a.to_string())))
            .map(|g| g.len_m)
    }

    /// Bifurcaciones y apartaderos de cruce por los que pasa el cantón estación→estación `a → b`,
    /// con su distancia (m) desde `a`, en orden de paso. Vacío si va directo.
    pub fn via(&self, a: &str, b: &str) -> &[(String, f64)] {
        self.via.get(&(a.to_string(), b.to_string())).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn junction(&self, id: &str) -> Option<&Junction> {
        self.junctions.get(id)
    }

    pub fn junction_ids(&self) -> impl Iterator<Item = (&String, &Junction)> {
        self.junctions.iter()
    }

    pub fn len(&self) -> usize {
        self.cantons.len()
    }
}

/// Geometría cargada UNA vez por proceso (dato de infraestructura inmutable, como la CVM de ADIF).
/// Los nombres de estación (para rotular bifurcaciones) se leen del GTFS filtrado.
pub fn global() -> Option<&'static TrackGeometry> {
    static G: OnceLock<Option<TrackGeometry>> = OnceLock::new();
    G.get_or_init(|| TrackGeometry::load(DEFAULT_PATH, &stop_names_from_gtfs())).as_ref()
}

fn stop_names_from_gtfs() -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Ok(mut rdr) = csv::ReaderBuilder::new().trim(csv::Trim::All).from_path("data/gtfs/stops.txt") else {
        return out;
    };
    let headers = rdr.headers().cloned().unwrap_or_default();
    let (Some(ci), Some(cn)) = (
        headers.iter().position(|h| h.trim_start_matches('\u{feff}') == "stop_id"),
        headers.iter().position(|h| h == "stop_name"),
    ) else {
        return out;
    };
    for rec in rdr.records().flatten() {
        if let (Some(i), Some(n)) = (rec.get(ci), rec.get(cn)) {
            out.insert(i.to_string(), n.to_string());
        }
    }
    out
}

pub fn polyline_len(pts: &[(f64, f64)]) -> f64 {
    pts.windows(2).map(|w| haversine_m(w[0].0, w[0].1, w[1].0, w[1].1)).sum()
}

/// Parte la polilínea en los puntos a las distancias `dists` (m, crecientes) desde su origen:
/// devuelve `dists.len() + 1` trozos que comparten los puntos de corte.
fn split_polyline(pts: &[(f64, f64)], dists: &[f64]) -> Vec<Vec<(f64, f64)>> {
    let mut parts = Vec::with_capacity(dists.len() + 1);
    let mut cur = vec![pts[0]];
    let mut acc = 0.0;
    let mut k = 0;
    for w in pts.windows(2) {
        let seg = haversine_m(w[0].0, w[0].1, w[1].0, w[1].1);
        while k < dists.len() && acc + seg >= dists[k] {
            let t = if seg > 0.0 { ((dists[k] - acc) / seg).clamp(0.0, 1.0) } else { 0.0 };
            let cut = (w[0].0 + (w[1].0 - w[0].0) * t, w[0].1 + (w[1].1 - w[0].1) * t);
            cur.push(cut);
            parts.push(std::mem::replace(&mut cur, vec![cut]));
            k += 1;
        }
        cur.push(w[1]);
        acc += seg;
    }
    while parts.len() < dists.len() {
        // Distancia más allá del final (no debería pasar): trozo degenerado en el último punto.
        let last = *cur.last().unwrap();
        parts.push(std::mem::replace(&mut cur, vec![last]));
    }
    parts.push(cur);
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverse_lookup_inverts_polyline() {
        let mut cantons = HashMap::new();
        let pts = vec![(41.0, 2.0), (41.001, 2.0), (41.001, 2.001)];
        let len_m = polyline_len(&pts);
        cantons.insert(("A".to_string(), "B".to_string()), CantonGeom { pts, len_m });
        let g = TrackGeometry { cantons, junctions: HashMap::new(), via: HashMap::new() };
        let r = g.get("B", "A").unwrap();
        assert_eq!(r.pts.first(), Some(&(41.001, 2.001)));
        assert!((r.len_m - len_m).abs() < 1e-9);
        // La polilínea con codo es más larga que la recta entre extremos.
        assert!(len_m > haversine_m(41.0, 2.0, 41.001, 2.001));
        assert!(g.get("A", "C").is_none());
    }

    #[test]
    fn split_keeps_total_length_and_cut_points() {
        let pts = vec![(41.0, 2.0), (41.01, 2.0), (41.02, 2.0)];
        let total = polyline_len(&pts);
        let parts = split_polyline(&pts, &[total * 0.25, total * 0.75]);
        assert_eq!(parts.len(), 3);
        let sum: f64 = parts.iter().map(|p| polyline_len(p)).sum();
        assert!((sum - total).abs() < 1e-6);
        assert!((polyline_len(&parts[0]) - total * 0.25).abs() < 1e-6);
        // Los trozos se tocan en el punto de corte.
        assert_eq!(parts[0].last(), parts[1].first());
    }
}
