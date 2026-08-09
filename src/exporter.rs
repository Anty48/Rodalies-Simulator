//! Exportación de horarios optimizados a CSV y comparativa final en consola.

use std::error::Error;
use std::path::{Path, PathBuf};

use crate::gtfs_loader::{fmt_hms, Network};
use crate::optimizer::LineOptResult;

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
