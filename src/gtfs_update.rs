//! Actualización del feed GTFS nacional (Cercanías/Rodalies) desde el portal de datos
//! abiertos de Renfe, y filtrado a Rodalies de Catalunya — la versión Rust de
//! `scripts/prep_gtfs.sh`, reutilizable desde el botón "Actualizar GTFS" de la web.
//!
//! Fuente oficial: <https://data.renfe.com/dataset/horarios-cercanias> →
//! `https://ssl.renfe.com/ftransit/Fichero_CER_FOMENTO/fomento_transit.zip` (confirmado en
//! sesión 2026-09-10; mismo fichero `fomento_transit.zip` que ya documentaba el README para
//! colocar a mano en `raw/fomento_transit/`).

use std::collections::HashSet;
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

const FEED_URL: &str = "https://ssl.renfe.com/ftransit/Fichero_CER_FOMENTO/fomento_transit.zip";

#[derive(Debug, Clone, serde::Serialize)]
pub struct UpdateStats {
    pub n_routes: usize,
    pub n_trips: usize,
    pub n_stop_times: usize,
    pub n_stops: usize,
}

/// Descarga el feed nacional con `curl` (incluido en Windows 10/11 y en cualquier entorno
/// Unix) a `dest`. No se añade una dependencia HTTP/TLS nueva solo para esto.
pub fn download_national_feed(dest: &Path) -> Result<(), String> {
    let out = std::process::Command::new("curl")
        .args(["-sL", "--fail", "--retry", "3", "-o"])
        .arg(dest)
        .arg(FEED_URL)
        .output()
        .map_err(|e| format!("no se pudo ejecutar curl: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "curl falló ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    match std::fs::metadata(dest) {
        Ok(m) if m.len() > 1_000_000 => Ok(()),
        Ok(m) => Err(format!("descarga sospechosamente pequeña ({} bytes)", m.len())),
        Err(e) => Err(format!("no se pudo leer el fichero descargado: {e}")),
    }
}

/// Extrae un ZIP de GTFS (agency/calendar/routes/shapes/stops/stop_times/transfers/trips) a
/// `out_dir`, sobreescribiendo lo que hubiera.
pub fn extract_zip(zip_path: &Path, out_dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let file = File::open(zip_path).map_err(|e| format!("no se pudo abrir el ZIP: {e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("ZIP inválido: {e}"))?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().to_string();
        // Solo ficheros planos con nombre saneado (el feed no trae subcarpetas reales).
        if name.contains('/') || name.contains("..") {
            continue;
        }
        let dest = out_dir.join(&name);
        let mut w = BufWriter::new(File::create(&dest).map_err(|e| e.to_string())?);
        std::io::copy(&mut entry, &mut w).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Filtra el feed nacional (en `raw_dir`) a Rodalies de Catalunya (route_short_name que
/// empieza por 'R') y escribe routes/trips/stop_times/stops en `out_dir`. Misma lógica que
/// `scripts/prep_gtfs.sh`, en Rust para poder invocarla desde el servidor sin depender de bash.
pub fn filter_to_rodalies(raw_dir: &Path, out_dir: &Path) -> Result<UpdateStats, String> {
    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;

    // 1. routes.txt -> conserva route_short_name que empieza por 'R'; recuerda route_id.
    let mut route_ids: HashSet<String> = HashSet::new();
    let n_routes = {
        let mut rdr = reader(&raw_dir.join("routes.txt"))?;
        let headers = rdr.headers().map_err(|e| e.to_string())?.clone();
        let mut wtr = csv::Writer::from_path(out_dir.join("routes.txt")).map_err(|e| e.to_string())?;
        wtr.write_record(&headers).map_err(|e| e.to_string())?;
        let short_idx = col_index(&headers, "route_short_name")?;
        let id_idx = col_index(&headers, "route_id")?;
        let mut n = 0usize;
        for rec in rdr.records() {
            let rec = rec.map_err(|e| e.to_string())?;
            let short = rec.get(short_idx).unwrap_or("").trim();
            if short.starts_with('R') {
                route_ids.insert(rec.get(id_idx).unwrap_or("").trim().to_string());
                wtr.write_record(&rec).map_err(|e| e.to_string())?;
                n += 1;
            }
        }
        wtr.flush().map_err(|e| e.to_string())?;
        n
    };

    // 2. trips.txt -> conserva route_id in route_ids; recuerda trip_id.
    let mut trip_ids: HashSet<String> = HashSet::new();
    let n_trips = {
        let mut rdr = reader(&raw_dir.join("trips.txt"))?;
        let headers = rdr.headers().map_err(|e| e.to_string())?.clone();
        let mut wtr = csv::Writer::from_path(out_dir.join("trips.txt")).map_err(|e| e.to_string())?;
        wtr.write_record(&headers).map_err(|e| e.to_string())?;
        let route_idx = col_index(&headers, "route_id")?;
        let trip_idx = col_index(&headers, "trip_id")?;
        let mut n = 0usize;
        for rec in rdr.records() {
            let rec = rec.map_err(|e| e.to_string())?;
            if route_ids.contains(rec.get(route_idx).unwrap_or("").trim()) {
                trip_ids.insert(rec.get(trip_idx).unwrap_or("").trim().to_string());
                wtr.write_record(&rec).map_err(|e| e.to_string())?;
                n += 1;
            }
        }
        wtr.flush().map_err(|e| e.to_string())?;
        n
    };

    // 3. stop_times.txt (el fichero grande) -> conserva trip_id in trip_ids; recuerda stop_id.
    let mut stop_ids: HashSet<String> = HashSet::new();
    let n_stop_times = {
        let mut rdr = reader(&raw_dir.join("stop_times.txt"))?;
        let headers = rdr.headers().map_err(|e| e.to_string())?.clone();
        let mut wtr = csv::Writer::from_path(out_dir.join("stop_times.txt")).map_err(|e| e.to_string())?;
        wtr.write_record(&headers).map_err(|e| e.to_string())?;
        let trip_idx = col_index(&headers, "trip_id")?;
        let stop_idx = col_index(&headers, "stop_id")?;
        let mut n = 0usize;
        for rec in rdr.records() {
            let rec = rec.map_err(|e| e.to_string())?;
            if trip_ids.contains(rec.get(trip_idx).unwrap_or("").trim()) {
                stop_ids.insert(rec.get(stop_idx).unwrap_or("").trim().to_string());
                wtr.write_record(&rec).map_err(|e| e.to_string())?;
                n += 1;
            }
        }
        wtr.flush().map_err(|e| e.to_string())?;
        n
    };

    // 4. stops.txt -> conserva stop_id in stop_ids.
    let n_stops = {
        let mut rdr = reader(&raw_dir.join("stops.txt"))?;
        let headers = rdr.headers().map_err(|e| e.to_string())?.clone();
        let mut wtr = csv::Writer::from_path(out_dir.join("stops.txt")).map_err(|e| e.to_string())?;
        wtr.write_record(&headers).map_err(|e| e.to_string())?;
        let id_idx = col_index(&headers, "stop_id")?;
        let mut n = 0usize;
        for rec in rdr.records() {
            let rec = rec.map_err(|e| e.to_string())?;
            if stop_ids.contains(rec.get(id_idx).unwrap_or("").trim()) {
                wtr.write_record(&rec).map_err(|e| e.to_string())?;
                n += 1;
            }
        }
        wtr.flush().map_err(|e| e.to_string())?;
        n
    };

    Ok(UpdateStats { n_routes, n_trips, n_stop_times, n_stops })
}

fn reader(path: &Path) -> Result<csv::Reader<File>, String> {
    csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .flexible(true)
        .from_path(path)
        .map_err(|e| format!("no se pudo leer {}: {e}", path.display()))
}

fn col_index(headers: &csv::StringRecord, name: &str) -> Result<usize, String> {
    headers
        .iter()
        .position(|h| h.trim() == name)
        .ok_or_else(|| format!("columna '{name}' no encontrada en la cabecera"))
}

/// Orquesta la actualización completa desde el feed oficial de Renfe: descarga, extrae a
/// `raw/fomento_transit/`, filtra a `data/gtfs/`. `progress` recibe mensajes de estado.
pub fn update_from_renfe(progress: impl Fn(&str)) -> Result<UpdateStats, String> {
    let tmp_zip = std::env::temp_dir().join("fomento_transit_nuevo.zip");
    progress("Descargando el feed nacional de Renfe…");
    download_national_feed(&tmp_zip)?;
    update_from_zip(&tmp_zip, progress)
}

/// Igual que `update_from_renfe` pero partiendo de bytes ya subidos (botón "Subir GTFS").
pub fn update_from_bytes(bytes: &[u8], progress: impl Fn(&str)) -> Result<UpdateStats, String> {
    let tmp_zip = std::env::temp_dir().join("fomento_transit_subido.zip");
    std::fs::write(&tmp_zip, bytes).map_err(|e| format!("no se pudo guardar el ZIP subido: {e}"))?;
    update_from_zip(&tmp_zip, progress)
}

fn update_from_zip(zip_path: &Path, progress: impl Fn(&str)) -> Result<UpdateStats, String> {
    progress("Extrayendo el ZIP…");
    let raw_dir = Path::new("raw/fomento_transit");
    extract_zip(zip_path, raw_dir)?;
    let _ = std::fs::remove_file(zip_path);
    progress("Filtrando a Rodalies de Catalunya…");
    let stats = filter_to_rodalies(raw_dir, Path::new("data/gtfs"))?;
    progress("Filtrado completo.");
    Ok(stats)
}
