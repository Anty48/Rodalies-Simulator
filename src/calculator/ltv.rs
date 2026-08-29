//! Limitaciones Temporales de Velocidad (LTV) de ADIF — capa **temporal** (obras,
//! deterioro, seguridad), distinta de la velocidad de diseño permanente (DesignSpeed).
//!
//! Fuente OFICIAL: ADIF — HUB LTV (ArcGIS FeatureServer `LTV_2`, portal
//! `ltv-adif.hub.arcgis.com`). Cada LTV es un **punto** (inicio, WGS84) con
//! `RESTRICCIONVELOCIDAD` (km/h) y un tramo por PK (`PKINI→PKFIN`).
//!
//! Al ser un dato **que cambia a diario**, se trata como un *snapshot fechado* y se
//! aplica sólo cuando el usuario activa la opción. Flujo de actualización diaria:
//!   1. `scripts/fetch_adif_ltv.py` (descarga directa del FeatureServer), **o**
//!   2. dejar el ZIP diario del HUB en `raw/ltv/` → el programa lo **auto-ingiere** al
//!      arrancar (o vía `/api/ltv/reload`), detectando GeoJSON o CSV dentro del ZIP.
//!
//! Procedencia: 🟠 el valor de velocidad es oficial, pero es **temporal/fechado** y su
//! aplicación espacial usa el punto de inicio + extensión por PK a lo largo de la ruta
//! (aproximación documentada: no se dispone de la geometría de línea de la LTV).

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const CELL: f64 = 0.01;
const PROCESSED: &str = "processed/adif/ltv.json";

/// Una LTV: punto de inicio + velocidad + longitud del tramo (por PK).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ltv {
    pub lat: f64,
    pub lon: f64,
    pub speed_kmh: f64,
    #[serde(default)]
    pub span_m: f64,
    #[serde(default)]
    pub line: Option<String>,
    #[serde(default)]
    pub desc: Option<String>,
    #[serde(default)]
    pub motivo: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LtvDoc {
    snapshot: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    ltvs: Vec<Ltv>,
}

/// Conjunto de LTV con índice espacial y metadatos del snapshot.
pub struct LtvSet {
    pub ltvs: Vec<Ltv>,
    grid: HashMap<(i32, i32), Vec<u32>>,
    pub snapshot: String,
    pub source: String,
}

fn cell_key(lat: f64, lon: f64) -> (i32, i32) {
    ((lat / CELL).floor() as i32, (lon / CELL).floor() as i32)
}

impl LtvSet {
    fn build(ltvs: Vec<Ltv>, snapshot: String, source: String) -> LtvSet {
        let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, l) in ltvs.iter().enumerate() {
            grid.entry(cell_key(l.lat, l.lon)).or_default().push(i as u32);
        }
        LtvSet { ltvs, grid, snapshot, source }
    }

    pub fn count(&self) -> usize {
        self.ltvs.len()
    }

    /// LTV cuyo punto de inicio está a ≤ `tol_m` del punto dado. Devuelve
    /// `(índice, velocidad km/h, longitud m)`.
    pub fn nearby(&self, lat: f64, lon: f64, tol_m: f64) -> Vec<(usize, f64, f64)> {
        const K: f64 = 111_320.0;
        let coslat = lat.to_radians().cos();
        let (ci, cj) = cell_key(lat, lon);
        let mut out = Vec::new();
        for di in -1..=1 {
            for dj in -1..=1 {
                if let Some(v) = self.grid.get(&(ci + di, cj + dj)) {
                    for &idx in v {
                        let l = &self.ltvs[idx as usize];
                        let dx = (l.lon - lon) * coslat * K;
                        let dy = (l.lat - lat) * K;
                        if (dx * dx + dy * dy).sqrt() <= tol_m {
                            out.push((idx as usize, l.speed_kmh, l.span_m));
                        }
                    }
                }
            }
        }
        out
    }

    /// Carga el JSON procesado (`processed/adif/ltv.json`).
    pub fn load_processed(path: &Path) -> Option<LtvSet> {
        let txt = std::fs::read_to_string(path).ok()?;
        let doc: LtvDoc = serde_json::from_str(&txt).ok()?;
        if doc.ltvs.is_empty() {
            return None;
        }
        Some(LtvSet::build(doc.ltvs, doc.snapshot, doc.source))
    }
}

// --------------------------------------------------------------------------
// Ingesta de un ZIP / GeoJSON / CSV dejado por el usuario (dato diario)
// --------------------------------------------------------------------------

/// Reproyección inversa UTM (ETRS89/UTM 30N, EPSG:25830) → lon/lat (grados). ETRS89 usa
/// el elipsoide GRS80, prácticamente idéntico a WGS84 para este uso.
fn utm30n_to_lonlat(easting: f64, northing: f64) -> (f64, f64) {
    let a = 6_378_137.0_f64; // GRS80 semieje mayor
    let f: f64 = 1.0 / 298.257_222_101;
    let e2: f64 = f * (2.0 - f);
    let e1 = (1.0 - (1.0 - e2).sqrt()) / (1.0 + (1.0 - e2).sqrt());
    let k0 = 0.9996;
    let x = easting - 500_000.0;
    let y = northing;
    let m = y / k0;
    let mu = m
        / (a * (1.0 - e2 / 4.0 - 3.0 * e2 * e2 / 64.0 - 5.0 * e2 * e2 * e2 / 256.0));
    let phi1 = mu
        + (3.0 * e1 / 2.0 - 27.0 * e1.powi(3) / 32.0) * (2.0 * mu).sin()
        + (21.0 * e1 * e1 / 16.0 - 55.0 * e1.powi(4) / 32.0) * (4.0 * mu).sin()
        + (151.0 * e1.powi(3) / 96.0) * (6.0 * mu).sin();
    let ep2 = e2 / (1.0 - e2);
    let c1 = ep2 * phi1.cos().powi(2);
    let t1 = phi1.tan().powi(2);
    let n1 = a / (1.0 - e2 * phi1.sin().powi(2)).sqrt();
    let r1 = a * (1.0 - e2) / (1.0 - e2 * phi1.sin().powi(2)).powf(1.5);
    let d = x / (n1 * k0);
    let lat = phi1
        - (n1 * phi1.tan() / r1)
            * (d * d / 2.0
                - (5.0 + 3.0 * t1 + 10.0 * c1 - 4.0 * c1 * c1 - 9.0 * ep2) * d.powi(4) / 24.0
                + (61.0 + 90.0 * t1 + 298.0 * c1 + 45.0 * t1 * t1 - 252.0 * ep2
                    - 3.0 * c1 * c1)
                    * d.powi(6)
                    / 720.0);
    let lon_rad = (d - (1.0 + 2.0 * t1 + c1) * d.powi(3) / 6.0
        + (5.0 - 2.0 * c1 + 28.0 * t1 - 3.0 * c1 * c1 + 8.0 * ep2 + 24.0 * t1 * t1)
            * d.powi(5)
            / 120.0)
        / phi1.cos();
    let lon0 = -3.0_f64.to_radians(); // meridiano central zona 30
    (lon0.to_degrees() + lon_rad.to_degrees(), lat.to_degrees())
}

fn parse_geojson(text: &str) -> Vec<Ltv> {
    let Ok(v): Result<serde_json::Value, _> = serde_json::from_str(text) else { return Vec::new() };
    let Some(feats) = v.get("features").and_then(|f| f.as_array()) else { return Vec::new() };
    let mut out = Vec::new();
    for f in feats {
        let p = f.get("properties");
        let sp = p
            .and_then(|p| p.get("RESTRICCIONVELOCIDAD"))
            .and_then(|x| x.as_f64())
            .unwrap_or(0.0);
        let coords = f.get("geometry").and_then(|g| g.get("coordinates")).and_then(|c| c.as_array());
        let (Some(c), true) = (coords, sp > 0.0) else { continue };
        let (Some(lon), Some(lat)) = (c.first().and_then(|x| x.as_f64()), c.get(1).and_then(|x| x.as_f64())) else { continue };
        let pki = p.and_then(|p| p.get("PKINI")).and_then(|x| x.as_f64());
        let pkf = p.and_then(|p| p.get("PKFIN")).and_then(|x| x.as_f64());
        let span = match (pki, pkf) {
            (Some(i), Some(j)) => (j - i).abs() * 1000.0,
            _ => 0.0,
        };
        let strf = |k: &str| p.and_then(|p| p.get(k)).and_then(|x| x.as_str()).map(|s| s.to_string());
        out.push(Ltv {
            lat,
            lon,
            speed_kmh: sp,
            span_m: span,
            line: strf("CODLINEA"),
            desc: strf("DESCLINEA"),
            motivo: strf("MOTIVO"),
        });
    }
    out
}

fn parse_csv(bytes: &[u8]) -> Vec<Ltv> {
    let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(bytes);
    let headers = match rdr.headers() {
        Ok(h) => h.clone(),
        Err(_) => return Vec::new(),
    };
    let col = |name: &str| headers.iter().position(|h| h.eq_ignore_ascii_case(name));
    let (Some(cx), Some(cy), Some(cv)) = (col("X"), col("Y"), col("RESTRICCIONVELOCIDAD")) else {
        return Vec::new();
    };
    let cpki = col("PKINI");
    let cpkf = col("PKFIN");
    let cline = col("CODLINEA");
    let cdesc = col("DESCLINEA");
    let cmot = col("MOTIVO");
    let mut out = Vec::new();
    for rec in rdr.records().flatten() {
        let get = |i: Option<usize>| i.and_then(|i| rec.get(i)).map(|s| s.trim().to_string());
        let (Some(x), Some(y), Some(v)) = (
            rec.get(cx).and_then(|s| s.trim().parse::<f64>().ok()),
            rec.get(cy).and_then(|s| s.trim().parse::<f64>().ok()),
            rec.get(cv).and_then(|s| s.trim().parse::<f64>().ok()),
        ) else {
            continue;
        };
        if v <= 0.0 {
            continue;
        }
        // X,Y en EPSG:25830 (UTM 30N) → lon/lat.
        let (lon, lat) = utm30n_to_lonlat(x, y);
        let pki = cpki.and_then(|i| rec.get(i)).and_then(|s| s.trim().parse::<f64>().ok());
        let pkf = cpkf.and_then(|i| rec.get(i)).and_then(|s| s.trim().parse::<f64>().ok());
        let span = match (pki, pkf) {
            (Some(i), Some(j)) => (j - i).abs() * 1000.0,
            _ => 0.0,
        };
        out.push(Ltv {
            lat,
            lon,
            speed_kmh: v,
            span_m: span,
            line: get(cline),
            desc: get(cdesc),
            motivo: get(cmot),
        });
    }
    out
}

/// Extrae LTV de un fichero (`.zip` con GeoJSON/CSV dentro, o `.geojson`/`.json`/`.csv`).
fn ingest_file(path: &Path) -> Vec<Ltv> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    match ext.as_str() {
        "zip" => {
            let Ok(file) = std::fs::File::open(path) else { return Vec::new() };
            let Ok(mut zip) = zip::ZipArchive::new(file) else { return Vec::new() };
            // Preferir GeoJSON; si no, CSV.
            let mut geo_idx = None;
            let mut csv_idx = None;
            for i in 0..zip.len() {
                if let Ok(f) = zip.by_index(i) {
                    let n = f.name().to_lowercase();
                    if n.ends_with(".geojson") || n.ends_with(".json") {
                        geo_idx = Some(i);
                    } else if n.ends_with(".csv") {
                        csv_idx = Some(i);
                    }
                }
            }
            if let Some(i) = geo_idx {
                let mut s = String::new();
                if let Ok(mut f) = zip.by_index(i) {
                    if f.read_to_string(&mut s).is_ok() {
                        return parse_geojson(&s);
                    }
                }
            }
            if let Some(i) = csv_idx {
                let mut b = Vec::new();
                if let Ok(mut f) = zip.by_index(i) {
                    if f.read_to_end(&mut b).is_ok() {
                        return parse_csv(&b);
                    }
                }
            }
            Vec::new()
        }
        "geojson" | "json" => std::fs::read_to_string(path).map(|s| parse_geojson(&s)).unwrap_or_default(),
        "csv" => std::fs::read(path).map(|b| parse_csv(&b)).unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Fichero de datos LTV más reciente en `raw/ltv/` (zip/geojson/json/csv).
fn newest_raw(dir: &Path) -> Option<(PathBuf, std::time::SystemTime)> {
    let mut best: Option<(PathBuf, std::time::SystemTime)> = None;
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("").to_lowercase();
        if !matches!(ext.as_str(), "zip" | "geojson" | "json" | "csv") {
            continue;
        }
        let mtime = e.metadata().and_then(|m| m.modified()).ok()?;
        if best.as_ref().map_or(true, |(_, t)| mtime > *t) {
            best = Some((p, mtime));
        }
    }
    best
}

fn mtime_date(t: std::time::SystemTime) -> String {
    let secs = t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    // Días desde epoch → fecha civil (Howard Hinnant).
    let z = (secs / 86400) as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02}", y, m, d)
}

/// Auto-ingesta: si hay un fichero LTV en `raw/ltv/` más nuevo que el procesado (o no
/// hay procesado), lo procesa, escribe `processed/adif/ltv.json` y devuelve el conjunto.
/// Si no, carga el procesado existente. Si no hay nada, `None`.
pub fn auto_ingest(raw_dir: &Path, processed: &Path) -> Option<LtvSet> {
    let processed_mtime = std::fs::metadata(processed).and_then(|m| m.modified()).ok();
    if let Some((raw_path, raw_mtime)) = newest_raw(raw_dir) {
        let need = processed_mtime.map_or(true, |pm| raw_mtime > pm);
        if need {
            let ltvs = ingest_file(&raw_path);
            if !ltvs.is_empty() {
                let snapshot = mtime_date(raw_mtime);
                let source = format!(
                    "ADIF HUB LTV (ingerit de {})",
                    raw_path.file_name().and_then(|n| n.to_str()).unwrap_or("?")
                );
                write_processed(processed, &ltvs, &snapshot, &source);
                return Some(LtvSet::build(ltvs, snapshot, source));
            }
        }
    }
    LtvSet::load_processed(processed)
}

fn write_processed(path: &Path, ltvs: &[Ltv], snapshot: &str, source: &str) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let doc = serde_json::json!({
        "snapshot": snapshot, "source": source, "count": ltvs.len(), "ltvs": ltvs,
    });
    let _ = std::fs::write(path, doc.to_string());
}

/// Punto de entrada por defecto (rutas estándar del proyecto).
pub fn load_default() -> Option<LtvSet> {
    auto_ingest(Path::new("raw/ltv"), Path::new(PROCESSED))
}

// --------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"type":"FeatureCollection","features":[
      {"type":"Feature","properties":{"RESTRICCIONVELOCIDAD":30,"PKINI":6.0,"PKFIN":6.3,"CODLINEA":"026","DESCLINEA":"L","MOTIVO":"obres"},"geometry":{"type":"Point","coordinates":[-6.14,39.99]}},
      {"type":"Feature","properties":{"RESTRICCIONVELOCIDAD":0,"PKINI":1.0,"PKFIN":1.0},"geometry":{"type":"Point","coordinates":[2.0,41.5]}}
    ]}"#;

    #[test]
    fn parse_geojson_basico() {
        let v = parse_geojson(SAMPLE);
        assert_eq!(v.len(), 1, "debe descartar la de velocidad 0");
        assert_eq!(v[0].speed_kmh, 30.0);
        assert!((v[0].span_m - 300.0).abs() < 1.0);
        assert_eq!(v[0].line.as_deref(), Some("026"));
    }

    #[test]
    fn indice_espacial_encuentra_por_proximidad() {
        let set = LtvSet::build(parse_geojson(SAMPLE), "2026-08-30".into(), "test".into());
        // Muy cerca del punto (-6.14, 39.99).
        let hit = set.nearby(39.9902, -6.1401, 300.0);
        assert_eq!(hit.len(), 1);
        assert_eq!(hit[0].1, 30.0);
        // Lejos → nada.
        assert!(set.nearby(41.5, 2.0, 300.0).is_empty());
    }

    #[test]
    fn utm_reproyeccion_razonable() {
        // Un punto UTM 30N cerca de Madrid → lon≈-3.7, lat≈40.4.
        let (lon, lat) = utm30n_to_lonlat(440_000.0, 4_474_000.0);
        assert!((-4.0..-3.4).contains(&lon), "lon={}", lon);
        assert!((40.0..40.8).contains(&lat), "lat={}", lat);
    }
}
