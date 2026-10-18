//! **Demanda de viajeros por estación** a partir de datos reales de Renfe.
//!
//! Fuente: Renfe Data · «Volumen de viajeros por franja horaria (Barcelona)» (2018): viajeros que
//! SUBEN y que BAJAN en cada estación del núcleo de Rodalies de Barcelona, por franjas de 30 min
//! (123 estaciones, 369.477 viajeros/día). El `CODIGO_ESTACION` coincide con el `stop_id` del GTFS.
//! Fichero: `raw/renfe/barcelona_viajeros_por_franja.csv` (lo descarga `scripts/fetch_renfe_demand.py`).
//!
//! El motor usa la demanda como **curva acumulada**: los viajeros que han llegado al andén entre
//! dos instantes = `C(t1) − C(t0)`. Así el número de viajeros que recoge un tren depende de cuánto
//! hace que pasó el anterior (si va tarde, hay más gente → parada más larga → más retraso).

use std::collections::HashMap;
use std::path::Path;

pub const DEMAND_PATH: &str = "raw/renfe/barcelona_viajeros_por_franja.csv";
/// Franjas de 30 min en un día.
pub const BANDS: usize = 48;
const BAND_S: f64 = 1800.0;

/// Viajeros por franja de una estación (subidos y bajados).
#[derive(Clone)]
pub struct StationDemand {
    pub board: [f64; BANDS],
    pub alight: [f64; BANDS],
}

impl StationDemand {
    pub fn board_day(&self) -> f64 {
        self.board.iter().sum()
    }
    pub fn alight_day(&self) -> f64 {
        self.alight.iter().sum()
    }
}

pub struct Demand {
    pub stations: HashMap<String, StationDemand>,
    /// Perfil horario medio de SUBIDAS del núcleo (suma 1): para estaciones sin datos.
    pub profile: [f64; BANDS],
}

/// Demanda cargada UNA vez por proceso (el optimizador construye miles de motores: no se relee el CSV).
pub fn global() -> Option<&'static Demand> {
    static D: std::sync::OnceLock<Option<Demand>> = std::sync::OnceLock::new();
    D.get_or_init(Demand::load_default).as_ref()
}

impl Demand {
    pub fn load_default() -> Option<Demand> {
        Self::load(Path::new(DEMAND_PATH))
    }

    pub fn load(path: &Path) -> Option<Demand> {
        let text = std::fs::read_to_string(path).ok()?;
        let mut stations: HashMap<String, StationDemand> = HashMap::new();
        for line in text.lines().skip(1) {
            // CODIGO_ESTACION;NOMBRE_ESTACION;NUCLEO_CERCANIAS;TRAMO_HORARIO;VIAJEROS_SUBIDOS;VIAJEROS_BAJADOS
            let f: Vec<&str> = line.split(';').collect();
            if f.len() < 6 {
                continue;
            }
            let Some(band) = parse_band(f[3]) else { continue };
            let up: f64 = f[4].trim().parse().unwrap_or(0.0);
            let down: f64 = f[5].trim().parse().unwrap_or(0.0);
            let e = stations
                .entry(f[0].trim().trim_start_matches('\u{feff}').to_string())
                .or_insert(StationDemand { board: [0.0; BANDS], alight: [0.0; BANDS] });
            e.board[band] += up;
            e.alight[band] += down;
        }
        if stations.is_empty() {
            return None;
        }
        let mut profile = [0.0; BANDS];
        for s in stations.values() {
            for (p, b) in profile.iter_mut().zip(s.board.iter()) {
                *p += b;
            }
        }
        let tot: f64 = profile.iter().sum::<f64>().max(1.0);
        for p in &mut profile {
            *p /= tot;
        }
        Some(Demand { stations, profile })
    }

    pub fn total_board_day(&self) -> f64 {
        self.stations.values().map(StationDemand::board_day).sum()
    }
}

/// "07:30 - 08:00" → 15.
fn parse_band(s: &str) -> Option<usize> {
    let hm = s.trim().split('-').next()?.trim();
    let mut it = hm.split(':');
    let h: usize = it.next()?.trim().parse().ok()?;
    let m: usize = it.next()?.trim().parse().ok()?;
    let b = h * 2 + m / 30;
    (b < BANDS).then_some(b)
}

/// Curva ACUMULADA de viajeros de un día a partir de viajeros por franja: `cum[i]` = viajeros
/// llegados antes del inicio de la franja `i` (`cum[48]` = total del día).
pub fn cumulative(per_band: &[f64; BANDS]) -> [f64; BANDS + 1] {
    let mut c = [0.0; BANDS + 1];
    for i in 0..BANDS {
        c[i + 1] = c[i] + per_band[i];
    }
    c
}

/// Viajeros acumulados hasta el instante `t` (s desde medianoche; admite >24 h del GTFS: el día
/// siguiente repite la misma curva). Interpolación lineal dentro de la franja (llegadas uniformes).
pub fn cum_at(cum: &[f64; BANDS + 1], t: f64) -> f64 {
    let day = cum[BANDS];
    let t = t.max(0.0);
    let k = (t / 86400.0).floor();
    let r = t - k * 86400.0;
    let i = ((r / BAND_S) as usize).min(BANDS - 1);
    let frac = ((r - i as f64 * BAND_S) / BAND_S).clamp(0.0, 1.0);
    k * day + cum[i] + (cum[i + 1] - cum[i]) * frac
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn franja_se_parsea() {
        assert_eq!(parse_band("00:00 - 00:30"), Some(0));
        assert_eq!(parse_band("07:30 - 08:00"), Some(15));
        assert_eq!(parse_band("23:30 - 00:00"), Some(47));
    }

    #[test]
    fn acumulada_es_monotona_y_cruza_medianoche() {
        let mut b = [0.0; BANDS];
        b[14] = 100.0; // 07:00-07:30
        let c = cumulative(&b);
        assert_eq!(cum_at(&c, 7.0 * 3600.0), 0.0);
        assert!((cum_at(&c, 7.25 * 3600.0) - 50.0).abs() < 1e-9);
        assert_eq!(cum_at(&c, 8.0 * 3600.0), 100.0);
        // Día siguiente (GTFS 24:00+): se suma otro día completo.
        assert!((cum_at(&c, 86400.0 + 7.25 * 3600.0) - 150.0).abs() < 1e-9);
    }
}
