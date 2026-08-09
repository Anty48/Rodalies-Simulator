//! Exportación de horarios optimizados a CSV y PDF, y comparativas en consola.

use std::error::Error;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use printpdf::{BuiltinFont, Mm, PdfDocument};

use crate::gtfs_loader::{fmt_hms, Network, TrainService};
use crate::optimizer::{LineOptResult, SystemResult};

/// hora `HH:MM` a partir de segundos.
fn hm(secs: u32) -> String {
    format!("{:02}:{:02}", secs / 3600, (secs % 3600) / 60)
}

fn safe(line: &str) -> String {
    line.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// Viajes de tren (no bus) de una línea en la ventana, ordenados por salida de origen.
fn line_trips<'a>(
    net: &'a Network,
    line: &str,
    service_id: &str,
    window: (u32, u32),
) -> Vec<&'a TrainService> {
    let mut v: Vec<&TrainService> = net
        .services
        .iter()
        .filter(|s| {
            s.route_short_name == line
                && s.service_id == service_id
                && !s.is_bus
                && matches!(s.first_time(), Some(t) if t >= window.0 && t <= window.1)
        })
        .collect();
    v.sort_by_key(|t| t.schedule[0].departure_sec);
    v
}

// --------------------------------------------------------------------------
// Escritores genéricos (comparten núcleo entre optimización por línea y de sistema)
// --------------------------------------------------------------------------

/// CSV: una fila por (tren, parada) con las horas ya desplazadas.
fn write_csv(
    net: &Network,
    line: &str,
    trains: &[&TrainService],
    off: &dyn Fn(&TrainService) -> i64,
    dir: &Path,
) -> Result<PathBuf, Box<dyn Error>> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}_optimized.csv", safe(line)));
    let mut wtr = csv::Writer::from_path(&path)?;
    wtr.write_record([
        "trip_short_name", "trip_id", "stop_sequence", "stop_id", "stop_name",
        "arribada", "sortida", "offset_min",
    ])?;
    for t in trains {
        let o = off(t);
        for st in &t.schedule {
            let arr = (st.arrival_sec as i64 + o).max(0) as u32;
            let dep = (st.departure_sec as i64 + o).max(0) as u32;
            wtr.write_record([
                &t.train_number, &t.trip_id, &st.seq.to_string(), &st.stop_id,
                net.stop_name(&st.stop_id), &fmt_hms(arr), &fmt_hms(dep), &format!("{:+}", o / 60),
            ])?;
        }
    }
    wtr.flush()?;
    Ok(path)
}

/// PDF: tabla de horarios con **estaciones en COLUMNAS** y **trenes en FILAS**
/// (cada fila es un horario). A4 apaisado, paginado por trenes.
fn write_pdf(
    net: &Network,
    line: &str,
    trains: &[&TrainService],
    off: &dyn Fn(&TrainService) -> i64,
    dir: &Path,
) -> Result<PathBuf, Box<dyn Error>> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}_horari.pdf", safe(line)));

    // Estaciones (columnas) = trayecto más largo.
    let canon = trains.iter().max_by_key(|t| t.schedule.len()).copied();
    let Some(canon) = canon else {
        return Err("línia sense trens".into());
    };
    let stations: Vec<(String, String)> = canon
        .schedule
        .iter()
        .map(|st| (st.stop_id.clone(), net.stop_name(&st.stop_id).to_string()))
        .collect();

    let (w, h) = (297.0f32, 210.0f32);
    let (doc, page1, layer1) =
        PdfDocument::new(format!("Rodalies {} · horari optimitzat", line), Mm(w), Mm(h), "capa");
    let font = doc.add_builtin_font(BuiltinFont::Helvetica)?;
    let bold = doc.add_builtin_font(BuiltinFont::HelveticaBold)?;

    // Geometría: columna de etiqueta de tren a la izquierda, estaciones a la derecha.
    let x_label = 10.0;
    let x_first = 34.0;
    let top = h - 30.0;
    let bottom = 12.0;
    let colw = ((w - x_first - 4.0) / stations.len().max(1) as f32).max(3.0);
    let fs: f32 = if colw < 7.0 { 4.5 } else { 6.0 };

    let per_page = (((top - bottom) / 4.6).floor() as usize).max(1);
    let chunks: Vec<&[&TrainService]> = trains.chunks(per_page).collect();
    let n_pages = chunks.len().max(1);

    for (pi, chunk) in chunks.iter().enumerate() {
        let layer = if pi == 0 {
            doc.get_page(page1).get_layer(layer1)
        } else {
            let (p, l) = doc.add_page(Mm(w), Mm(h), "capa");
            doc.get_page(p).get_layer(l)
        };

        layer.use_text(
            format!("Rodalies · Línia {} — horari optimitzat  (pàg. {}/{})", line, pi + 1, n_pages),
            12.0, Mm(x_label), Mm(h - 12.0), &bold,
        );
        layer.use_text(
            "Files = trens (horaris) · columnes = estacions. Generat per rodalies-sim.",
            7.5, Mm(x_label), Mm(h - 17.0), &font,
        );

        // Cabecera de columnas: nombre de estación abreviado.
        layer.use_text("Tren \\ Estació", 6.5, Mm(x_label), Mm(top + 4.0), &bold);
        for (ci, (_, name)) in stations.iter().enumerate() {
            let x = x_first + ci as f32 * colw;
            let ab = abbrev(name);
            layer.use_text(ab, (fs - 0.5).max(4.0), Mm(x), Mm(top + 4.0), &bold);
        }

        // Filas: un tren por fila.
        for (ri, t) in chunk.iter().enumerate() {
            let y = top - (ri as f32 + 1.0) * 4.6;
            let o = off(t);
            let dep0 = (t.schedule[0].departure_sec as i64 + o).max(0) as u32;
            let lbl = format!("{} {}", hm(dep0), short_num(&t.train_number));
            layer.use_text(lbl, fs, Mm(x_label), Mm(y), &font);
            for (ci, (sid, _)) in stations.iter().enumerate() {
                if let Some(st) = t.schedule.iter().find(|s| &s.stop_id == sid) {
                    let dep = (st.departure_sec as i64 + o).max(0) as u32;
                    let x = x_first + ci as f32 * colw;
                    layer.use_text(hm(dep), fs, Mm(x), Mm(y), &font);
                }
            }
        }
    }

    doc.save(&mut BufWriter::new(std::fs::File::create(&path)?))?;
    Ok(path)
}

fn abbrev(name: &str) -> String {
    let n = name.trim_start_matches("Barcelona").trim_start_matches('-').trim();
    n.chars().take(7).collect()
}
fn short_num(tn: &str) -> String {
    // Últimos dígitos del número de circulación (más legible en la etiqueta).
    let digits: String = tn.chars().filter(|c| c.is_ascii_digit()).collect();
    let n = digits.len();
    if n > 5 {
        digits[n - 5..].to_string()
    } else {
        digits
    }
}

// --------------------------------------------------------------------------
// Exportación por LÍNEA (optimización individual)
// --------------------------------------------------------------------------

pub fn export_line_csv(net: &Network, r: &LineOptResult, dir: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let trains: Vec<&TrainService> = r
        .trip_ids
        .iter()
        .filter_map(|id| net.services.iter().find(|s| &s.trip_id == id))
        .collect();
    let off = |t: &TrainService| r.offsets.get(&t.trip_id).copied().unwrap_or(0);
    write_csv(net, &r.line, &trains, &off, dir)
}

pub fn export_line_pdf(net: &Network, r: &LineOptResult, dir: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let mut trains: Vec<&TrainService> = r
        .trip_ids
        .iter()
        .filter_map(|id| net.services.iter().find(|s| &s.trip_id == id))
        .collect();
    let off = |t: &TrainService| r.offsets.get(&t.trip_id).copied().unwrap_or(0);
    trains.sort_by_key(|t| t.schedule[0].departure_sec as i64 + off(t));
    write_pdf(net, &r.line, &trains, &off, dir)
}

// --------------------------------------------------------------------------
// Exportación del SISTEMA (un desfase por línea)
// --------------------------------------------------------------------------

/// Exporta CSV + PDF por cada línea aplicando su desfase óptimo del sistema.
/// Devuelve `[(línea, csv, pdf)]`.
pub fn export_system(
    net: &Network,
    result: &SystemResult,
    service_id: &str,
    window: (u32, u32),
    dir: &Path,
) -> Vec<(String, PathBuf, PathBuf)> {
    let mut out = Vec::new();
    for line in &result.lines {
        let trains = line_trips(net, line, service_id, window);
        if trains.is_empty() {
            continue;
        }
        let o = result.offsets.get(line).copied().unwrap_or(0);
        let off = move |_: &TrainService| o;
        let csv = write_csv(net, line, &trains, &off, dir);
        let pdf = write_pdf(net, line, &trains, &off, dir);
        if let (Ok(c), Ok(p)) = (csv, pdf) {
            out.push((line.clone(), c, p));
        }
    }
    out
}

// --------------------------------------------------------------------------
// Comparativas en consola
// --------------------------------------------------------------------------

pub fn print_comparison(results: &[LineOptResult]) {
    println!("\n┌─ COMPARATIVA D'OPTIMITZACIÓ PER LÍNIA ({} línies) ────────────────────┐", results.len());
    println!(
        "  {:<5} {:>5} {:>9} {:>9} {:>7} {:>9} {:>9} {:>11}",
        "línia", "trens", "V base", "V òptim", "ΔV %", "pic base", "pic òpt", "conflictes"
    );
    println!("  {}", "─".repeat(72));
    for r in results {
        println!(
            "  {:<5} {:>5} {:>9.1} {:>9.1} {:>6.1}% {:>7.0} s {:>7.0} s {:>4.0}→{:<4.0} ({:.0}→{:.0}m)",
            r.line, r.n_trips, r.base_v, r.best_v, pct(r.base_v, r.best_v),
            r.base_peak, r.best_peak, r.base_held, r.best_held,
            r.base_recovery_min, r.best_recovery_min
        );
    }
    println!("  └{}", "─".repeat(72));
}

pub fn print_system_comparison(r: &SystemResult) {
    println!("\n┌─ RESULTAT · OPTIMITZACIÓ DEL SISTEMA (dia laborable) ─────────────────┐");
    println!("  Trens (tren, no bus) coordinats .... {}", r.trips);
    println!("  Línies amb desfàs ajustat .......... {} de {}", r.offsets.len(), r.lines.len());
    println!("  Potencial V .............. {:.1}  →  {:.1}   ({:+.1}%)", r.base_v, r.best_v, -r.delta_pct);
    println!("  Pic de retard acumulat mitjà  {:.0} s  →  {:.0} s   ({:+.1}%)",
        r.base_delay, r.best_delay, -pct(r.base_delay, r.best_delay));
    println!("  Retencions per senyal (mitjana) {:.0}  →  {:.0}", r.base_held, r.best_held);
    println!("  Temps de recuperació mitjà  {:.1} min  →  {:.1} min", r.base_recovery_min, r.best_recovery_min);
    println!("  {}", "─".repeat(60));
    println!("  Desfàs òptim per línia (min):");
    let mut offs: Vec<(&String, &i64)> = r.offsets.iter().collect();
    offs.sort_by_key(|(l, _)| l.to_string());
    let row: Vec<String> = offs.iter().map(|(l, o)| format!("{}={:+}", l, **o / 60)).collect();
    if row.is_empty() {
        println!("    (cap ajust: el sistema base ja era òptim per a aquests escenaris)");
    } else {
        println!("    {}", row.join("  "));
    }
    println!("└───────────────────────────────────────────────────────────────────────┘");
}

fn pct(base: f64, opt: f64) -> f64 {
    if base.abs() < 1e-9 {
        0.0
    } else {
        (base - opt) / base * 100.0
    }
}
