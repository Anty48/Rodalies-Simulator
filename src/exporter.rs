//! Exportación de horarios optimizados a CSV y comparativa final en consola.

use std::error::Error;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use printpdf::{BuiltinFont, Mm, PdfDocument};

use crate::gtfs_loader::{fmt_hms, Network, TrainService};
use crate::optimizer::LineOptResult;

/// hora `HH:MM` a partir de segundos.
fn hm(secs: u32) -> String {
    format!("{:02}:{:02}", secs / 3600, (secs % 3600) / 60)
}

/// Escribe `<LINEA>_optimized.csv` con la hora exacta de salida/llegada por estación y
/// tren (`trip_short_name`) tras aplicar los desplazamientos óptimos.
pub fn export_line_csv(
    net: &Network,
    result: &LineOptResult,
    dir: &Path,
) -> Result<PathBuf, Box<dyn Error>> {
    std::fs::create_dir_all(dir)?;
    let safe: String = result
        .line
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let path = dir.join(format!("{}_optimized.csv", safe));
    let mut wtr = csv::Writer::from_path(&path)?;

    wtr.write_record([
        "trip_short_name",
        "trip_id",
        "stop_sequence",
        "stop_id",
        "stop_name",
        "arrival_optimized",
        "departure_optimized",
        "offset_min",
    ])?;

    // Índice trip_id → &TrainService para búsqueda rápida.
    for trip_id in &result.trip_ids {
        let Some(svc) = net.services.iter().find(|s| &s.trip_id == trip_id) else {
            continue;
        };
        let off = result.offsets.get(trip_id).copied().unwrap_or(0);
        for st in &svc.schedule {
            let arr = (st.arrival_sec as i64 + off).max(0) as u32;
            let dep = (st.departure_sec as i64 + off).max(0) as u32;
            wtr.write_record([
                &svc.train_number,
                &svc.trip_id,
                &st.seq.to_string(),
                &st.stop_id,
                net.stop_name(&st.stop_id),
                &fmt_hms(arr),
                &fmt_hms(dep),
                &format!("{:+}", off / 60),
            ])?;
        }
    }
    wtr.flush()?;
    Ok(path)
}

/// Exporta un PDF por línea con la tabla de horarios optimizada (estaciones en filas,
/// trenes en columnas). Formato tipo horario oficial de Rodalies. A4 apaisado.
pub fn export_line_pdf(
    net: &Network,
    result: &LineOptResult,
    dir: &Path,
) -> Result<PathBuf, Box<dyn Error>> {
    std::fs::create_dir_all(dir)?;
    let safe: String = result
        .line
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let path = dir.join(format!("{}_horari.pdf", safe));

    // Trenes (rail) ordenados por hora de salida optimizada.
    let mut trains: Vec<&TrainService> = result
        .trip_ids
        .iter()
        .filter_map(|id| net.services.iter().find(|s| &s.trip_id == id))
        .collect();
    let off = |t: &TrainService| result.offsets.get(&t.trip_id).copied().unwrap_or(0);
    trains.sort_by_key(|t| t.schedule[0].departure_sec as i64 + off(t));

    // Secuencia canónica de estaciones = el trayecto más largo.
    let canon = trains.iter().max_by_key(|t| t.schedule.len()).copied();
    let Some(canon) = canon else {
        return Err("línia sense trens".into());
    };
    let stations: Vec<(String, String)> = canon
        .schedule
        .iter()
        .map(|st| (st.stop_id.clone(), net.stop_name(&st.stop_id).to_string()))
        .collect();

    // Documento A4 apaisado.
    let (w, h) = (297.0, 210.0);
    let (doc, page1, layer1) =
        PdfDocument::new(format!("Rodalies {} · horari optimitzat", result.line), Mm(w), Mm(h), "capa");
    let font = doc.add_builtin_font(BuiltinFont::Helvetica)?;
    let bold = doc.add_builtin_font(BuiltinFont::HelveticaBold)?;

    let per_page = 15usize;
    let chunks: Vec<&[&TrainService]> = trains.chunks(per_page).collect();
    let n_pages = chunks.len().max(1);

    for (pi, chunk) in chunks.iter().enumerate() {
        let layer = if pi == 0 {
            doc.get_page(page1).get_layer(layer1)
        } else {
            let (p, l) = doc.add_page(Mm(w), Mm(h), "capa");
            doc.get_page(p).get_layer(l)
        };

        // Cabecera.
        layer.use_text(
            format!("Rodalies · Línia {} — horari optimitzat  (pàg. {}/{})", result.line, pi + 1, n_pages),
            13.0,
            Mm(12.0),
            Mm(h - 12.0),
            &bold,
        );
        layer.use_text(
            "Estacions en files · trens en columnes (hora de sortida). Generat per rodalies-sim.",
            8.0,
            Mm(12.0),
            Mm(h - 17.0),
            &font,
        );

        // Geometría de la rejilla.
        let x_station = 12.0;
        let x_first = 62.0;
        let top = h - 24.0;
        let bottom = 12.0;
        let rows = stations.len().max(1);
        let rowh = ((top - bottom) / rows as f32).clamp(3.6, 7.0);
        let colw = (w - x_first - 6.0) / chunk.len().max(1) as f32;
        let fs = if rowh < 4.5 { 6.0 } else { 7.0 };

        // Cabecera de columnas: hora de salida de cada tren.
        for (ci, t) in chunk.iter().enumerate() {
            let x = x_first + ci as f32 * colw;
            let dep0 = (t.schedule[0].departure_sec as i64 + off(t)).max(0) as u32;
            layer.use_text(hm(dep0), fs, Mm(x), Mm(top + 1.0), &bold);
        }
        layer.use_text("Estació", 8.0, Mm(x_station), Mm(top + 1.0), &bold);

        // Filas de estaciones.
        for (ri, (sid, name)) in stations.iter().enumerate() {
            let y = top - (ri as f32 + 1.0) * rowh;
            let short: String = name.chars().take(26).collect();
            layer.use_text(short, fs, Mm(x_station), Mm(y), &font);
            for (ci, t) in chunk.iter().enumerate() {
                if let Some(st) = t.schedule.iter().find(|s| &s.stop_id == sid) {
                    let dep = (st.departure_sec as i64 + off(t)).max(0) as u32;
                    let x = x_first + ci as f32 * colw;
                    layer.use_text(hm(dep), fs, Mm(x), Mm(y), &font);
                }
            }
        }
    }

    doc.save(&mut BufWriter::new(std::fs::File::create(&path)?))?;
    Ok(path)
}

/// Comparativa final en consola: potencial base vs optimizado, recuperación y
/// reducción del retraso por pasajero.
pub fn print_comparison(results: &[LineOptResult]) {
    println!("\n┌─ COMPARATIVA D'OPTIMITZACIÓ ({} línies) ──────────────────────────────┐", results.len());
    println!(
        "  {:<5} {:>5} {:>10} {:>10} {:>8} {:>10} {:>10} {:>12}",
        "línia", "trens", "V base", "V òptim", "ΔV %", "recup base", "recup òpt", "conflictes"
    );
    println!("  {}", "─".repeat(78));

    let mut sum_dv = 0.0;
    let mut sum_ddelay = 0.0;
    let mut cnt = 0.0;
    for r in results {
        let dv = pct(r.base_v, r.best_v);
        let ddelay = pct(r.base_peak, r.best_peak);
        sum_dv += dv;
        sum_ddelay += ddelay;
        cnt += 1.0;
        println!(
            "  {:<5} {:>5} {:>10.1} {:>10.1} {:>7.1}% {:>8.1} m {:>8.1} m {:>5.1}→{:<5.1}",
            r.line, r.n_trips, r.base_v, r.best_v, dv, r.base_recovery_min, r.best_recovery_min,
            r.base_held, r.best_held
        );
    }
    println!("  {}", "─".repeat(78));
    if cnt > 0.0 {
        println!(
            "  Mitjana: reducció de V {:.1}%  ·  reducció de retard/passatger {:.1}%",
            sum_dv / cnt,
            sum_ddelay / cnt
        );
    }
    println!("└───────────────────────────────────────────────────────────────────────┘");
}

/// Reducción porcentual de `base` a `opt` (positivo = mejora).
fn pct(base: f64, opt: f64) -> f64 {
    if base.abs() < 1e-9 {
        0.0
    } else {
        (base - opt) / base * 100.0
    }
}
