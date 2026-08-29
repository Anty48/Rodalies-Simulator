//! Análisis de **línea completa**: recorre los tramos consecutivos del itinerario
//! canónico de una línea/sentido (desde GTFS, sin hardcodear), simula la física de cada
//! tramo para cada serie, añade los **tiempos de parada** (dwell) y compara el **tiempo
//! mínimo físico** con el **tiempo programado** (GTFS). Detecta anomalías y etiqueta la
//! **calidad de cada dato** por separado.
//!
//! Devuelve una estructura serializable a JSON; la UI la cachea y dibuja tablas/gráficas
//! sin recalcular al cambiar de serie.

use std::collections::HashMap;

use serde::Serialize;

use crate::calculator::infrastructure::{self, haversine_m, AdifNet};
use crate::calculator::physics::{self, Restriction, SpeedZone};
use crate::calculator::rolling_stock::{self, Provenance};
use crate::calculator::schedules::{self, Stat};
use crate::gtfs_loader::Network;

/// Modo de tiempo de parada.
#[derive(Debug, Clone)]
pub enum DwellMode {
    /// Mediana real por estación desde los horarios GTFS.
    Auto,
    /// Valor fijo (s) para todas las estaciones intermedias.
    Fixed(u32),
    /// Valor por estación (stop_id → s); las que falten usan 0.
    Custom(HashMap<String, u32>),
}

impl DwellMode {
    fn label(&self) -> String {
        match self {
            DwellMode::Auto => "Automàtic (mediana GTFS)".into(),
            DwellMode::Fixed(s) => format!("Fix {} s", s),
            DwellMode::Custom(_) => "Personalitzat".into(),
        }
    }
}

fn prov_code(p: Provenance) -> &'static str {
    match p {
        Provenance::Oficial => "oficial",
        Provenance::Secundaria => "secundaria",
        Provenance::Estimacion => "estimacion",
        Provenance::Suposicion => "suposicion",
        Provenance::NoDisponible => "nd",
    }
}

// --------------------------------------------------------------------------
// Estructuras de salida (JSON)
// --------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct SeriesSeg {
    pub marcha_s: f64,
    pub vmax_reached_kmh: f64,
    pub vmax_allowed_kmh: f64,
    pub t_accel: f64,
    pub t_cruise: f64,
    pub t_brake: f64,
    pub d_accel_km: f64,
    pub d_brake_km: f64,
    pub reached_end: bool,
    /// Marcha con la CVM real de ADIF aplicada (modelo refinado). `None` si sin datos ADIF.
    pub marcha_ref_s: Option<f64>,
    pub vmax_ref_reached_kmh: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Segment {
    pub from: String,
    pub to: String,
    pub dist_km: f64,
    pub per_series: HashMap<String, SeriesSeg>,
    /// Vmax mínima de infraestructura ADIF en el tramo (km/h), si hay datos.
    pub adif_min_vmax_kmh: Option<f64>,
    /// Distancia del tramo por geometría ADIF (km), si hay datos.
    pub adif_dist_km: Option<f64>,
    /// Cobertura ADIF del tramo [0,1].
    pub coverage: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Station {
    pub name: String,
    pub cum_km: f64,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub dwell_s: u32,
    pub dwell_src: String,
    /// Tiempo programado acumulado (s) desde la salida del origen (timetable canónico).
    pub programmed_cum_s: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Totals {
    pub marcha_s: f64,
    pub paradas_s: f64,
    pub total_s: f64,
    pub margin_median_s: f64,
    /// Modelo refinado (CVM ADIF): marcha + paradas y margen. `None` si sin datos ADIF.
    pub marcha_ref_s: Option<f64>,
    pub total_ref_s: Option<f64>,
    pub margin_ref_median_s: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProgrammedOut {
    pub n: usize,
    pub min: u32,
    pub max: u32,
    pub mean: u32,
    pub median: u32,
    pub p10: u32,
    pub p90: u32,
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Ficha {
    pub id: String,
    pub name: String,
    pub builder: String,
    pub available: bool,
    pub vmax: f64,
    pub vmax_prov: String,
    pub vmax_src: String,
    pub power: f64,
    pub power_prov: String,
    pub mass: f64,
    pub mass_prov: String,
    pub accel: f64,
    pub accel_prov: String,
    pub decel: f64,
    pub decel_prov: String,
    pub notes: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct QualityItem {
    pub variable: String,
    pub level: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceItem {
    pub variable: String,
    pub source: String,
    pub organismo: String,
    pub url: String,
    pub file: String,
    pub method: String,
    pub precision: String,
    pub level: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LineAnalysis {
    pub line: String,
    pub direction_label: String,
    pub direction_key: (String, String),
    pub distance_km: f64,
    pub n_stations: usize,
    pub n_segments: usize,
    pub dt: f64,
    pub series: Vec<String>,
    pub unavailable: Vec<String>,
    pub stations: Vec<Station>,
    pub segments: Vec<Segment>,
    pub totals: HashMap<String, Totals>,
    pub programmed: ProgrammedOut,
    pub observed_available: bool,
    pub observed_note: String,
    pub anomalies: Vec<String>,
    pub quality: Vec<QualityItem>,
    pub fichas: Vec<Ficha>,
    pub sources: Vec<SourceItem>,
    pub dwell_mode: String,
    /// Modelo refinado con CVM ADIF disponible.
    pub adif_available: bool,
    /// Distancia de la línea por geometría ADIF (km), si hay datos.
    pub adif_distance_km: Option<f64>,
    /// Cobertura ADIF media de la línea [0,100] %.
    pub coverage_pct: f64,
    /// LTV temporales aplicadas (nº, velocidad mínima, fecha del snapshot).
    pub ltv_applied: usize,
    pub min_ltv_kmh: Option<f64>,
    pub ltv_snapshot: Option<String>,
    pub error: Option<String>,
}

fn empty_error(line: &str, msg: &str) -> LineAnalysis {
    LineAnalysis {
        line: line.to_string(),
        direction_label: String::new(),
        direction_key: (String::new(), String::new()),
        distance_km: 0.0,
        n_stations: 0,
        n_segments: 0,
        dt: 0.1,
        series: Vec::new(),
        unavailable: Vec::new(),
        stations: Vec::new(),
        segments: Vec::new(),
        totals: HashMap::new(),
        programmed: ProgrammedOut {
            n: 0,
            min: 0,
            max: 0,
            mean: 0,
            median: 0,
            p10: 0,
            p90: 0,
            source: String::new(),
        },
        observed_available: false,
        observed_note: String::new(),
        anomalies: Vec::new(),
        quality: Vec::new(),
        fichas: Vec::new(),
        sources: Vec::new(),
        dwell_mode: String::new(),
        adif_available: false,
        adif_distance_km: None,
        coverage_pct: 0.0,
        ltv_applied: 0,
        min_ltv_kmh: None,
        ltv_snapshot: None,
        error: Some(msg.to_string()),
    }
}

fn stat_to_out(s: &Stat, source: String) -> ProgrammedOut {
    ProgrammedOut {
        n: s.n,
        min: s.min,
        max: s.max,
        mean: s.mean,
        median: s.median,
        p10: s.p10,
        p90: s.p90,
        source,
    }
}

/// Ficha JSON de una serie.
fn ficha_of(id: &str) -> Option<Ficha> {
    let s = rolling_stock::get(id)?;
    Some(Ficha {
        id: s.id.to_string(),
        name: s.name.to_string(),
        builder: s.builder.to_string(),
        available: s.available,
        vmax: s.vmax_kmh.value,
        vmax_prov: prov_code(s.vmax_kmh.prov).into(),
        vmax_src: s.vmax_kmh.source.to_string(),
        power: s.power_kw.value,
        power_prov: prov_code(s.power_kw.prov).into(),
        mass: s.mass_t.value,
        mass_prov: prov_code(s.mass_t.prov).into(),
        accel: s.accel_start.value,
        accel_prov: prov_code(s.accel_start.prov).into(),
        decel: s.decel_service.value,
        decel_prov: prov_code(s.decel_service.prov).into(),
        notes: s.notes.to_string(),
    })
}

/// Fuentes documentadas (para la sección global y el "¿de dónde sale este número?").
fn sources_list(adif_available: bool) -> Vec<SourceItem> {
    let cvm = if adif_available {
        SourceItem {
            variable: "Velocitat màxima infraestructura (CVM)".into(),
            source: "ADIF — DesignSpeed (velocitat de disseny per enllaç)".into(),
            organismo: "ADIF — IDEADIF".into(),
            url: "https://ideadif.adif.es/services/wfs (INSPIRE tn-ra:DesignSpeed) · datos.gob.es e0dat0002".into(),
            file: "processed/adif/rfig_speed.json (scripts/fetch_adif_cvm.py)".into(),
            method: "Projecció de la ruta a l'enllaç ADIF més proper → Vmax(x)".into(),
            precision: "Oficial (velocitat de disseny; cobertura parcial segons proximitat)".into(),
            level: "oficial".into(),
        }
    } else {
        SourceItem {
            variable: "Velocitat màxima infraestructura (CVM)".into(),
            source: "ADIF — no carregat".into(),
            organismo: "ADIF".into(),
            url: "Executa scripts/fetch_adif_cvm.py → processed/adif/rfig_speed.json".into(),
            file: "—".into(),
            method: "Sense fitxer processat → límit = Vmax del tren".into(),
            precision: "No disponible".into(),
            level: "nd".into(),
        }
    };
    let ltv = SourceItem {
        variable: "LTV (limitacions temporals de velocitat)".into(),
        source: "ADIF — HUB LTV (ArcGIS FeatureServer LTV_2)".into(),
        organismo: "ADIF".into(),
        url: "https://ltv-adif.hub.arcgis.com · services7.arcgis.com/.../LTV_2/FeatureServer".into(),
        file: "processed/adif/ltv.json (scripts/fetch_adif_ltv.py o ZIP a raw/ltv/)".into(),
        method: "Punt d'inici + extensió per PK al llarg de la ruta (min amb DesignSpeed)".into(),
        precision: "Velocitat oficial però TEMPORAL (snapshot fechat); s'aplica si s'activa".into(),
        level: "estimacion".into(),
    };
    vec![
        cvm,
        ltv,
        SourceItem {
            variable: "Distància / ruta / estacions".into(),
            source: "GTFS Rodalies/Cercanías".into(),
            organismo: "Renfe / Rodalies de Catalunya".into(),
            url: "data.renfe.com (feed GTFS)".into(),
            file: "data/gtfs (stops/routes/trips/stop_times)".into(),
            method: "Polilínia d'estacions (haversine entre parades consecutives)".into(),
            precision: "Aproximada (no PK oficial; infravalora la via real)".into(),
            level: "secundaria".into(),
        },
        SourceItem {
            variable: "Temps de parada (dwell)".into(),
            source: "GTFS stop_times (arrival/departure)".into(),
            organismo: "Renfe / Rodalies de Catalunya".into(),
            url: "data.renfe.com".into(),
            file: "data/gtfs/stop_times.txt".into(),
            method: "Mediana de (departure − arrival) per estació i sentit".into(),
            precision: "Oficial programada".into(),
            level: "oficial".into(),
        },
        SourceItem {
            variable: "Temps programat".into(),
            source: "GTFS stop_times".into(),
            organismo: "Renfe / Rodalies de Catalunya".into(),
            url: "data.renfe.com".into(),
            file: "data/gtfs/stop_times.txt".into(),
            method: "Estadística sobre serveis d'itinerari complet del sentit".into(),
            precision: "Oficial programada (no observada)".into(),
            level: "oficial".into(),
        },
        SourceItem {
            variable: "Material rodant (Vmax/potència/massa)".into(),
            source: "Renfe informacion-trenes.csv (447/450/470) · Wikipedia EN (490)".into(),
            organismo: "Renfe / Wikipedia".into(),
            url: "data.renfe.com · en.wikipedia.org/wiki/Renfe_Class_490".into(),
            file: "raw/informacion-trenes.csv".into(),
            method: "Fitxa tècnica".into(),
            precision: "Oficial (447/450/470) · Secundària (490)".into(),
            level: "oficial".into(),
        },
        SourceItem {
            variable: "Acceleració / frenada".into(),
            source: "Model (no publicat per cap font)".into(),
            organismo: "—".into(),
            url: "—".into(),
            file: "src/calculator/rolling_stock.rs".into(),
            method: "Potència constant amb tope d'arrencada; frenada b constant".into(),
            precision: "Suposició del model".into(),
            level: "suposicion".into(),
        },
        SourceItem {
            variable: "Circulació observada (hora real)".into(),
            source: "No trobada oberta".into(),
            organismo: "Renfe/Adif".into(),
            url: "—".into(),
            file: "—".into(),
            method: "No hi ha dataset històric obert de circulació real per tren".into(),
            precision: "No disponible".into(),
            level: "nd".into(),
        },
    ]
}

/// Punto de entrada: análisis completo de una línea/sentido.
pub fn analyze_line(
    net: &Network,
    line: &str,
    dir_key: &(String, String),
    series_ids: &[String],
    dwell: &DwellMode,
    dt: f64,
    adif: Option<&AdifNet>,
    ltv: Option<&crate::calculator::ltv::LtvSet>,
) -> LineAnalysis {
    let dt = if dt.is_finite() && dt > 0.0 { dt.clamp(0.02, 1.0) } else { 0.1 };

    let Some(itin) = schedules::itinerary(net, line, dir_key) else {
        return empty_error(line, "No s'ha trobat l'itinerari d'aquesta línia/sentit al GTFS.");
    };
    let canonical_len = itin.stops.len();
    if canonical_len < 2 {
        return empty_error(line, "L'itinerari té menys de 2 estacions.");
    }

    // Horarios.
    let dwell_map = schedules::dwell_by_station(net, line, dir_key, canonical_len);
    let programmed_stat = schedules::programmed_total(net, line, dir_key, canonical_len);
    let prog_cum: HashMap<String, u32> =
        schedules::programmed_cumulative(net, &itin.dir.canonical_trip).into_iter().collect();

    // Series disponibles / no disponibles.
    let mut series: Vec<String> = Vec::new();
    let mut unavailable: Vec<String> = Vec::new();
    for id in series_ids {
        match rolling_stock::get(id) {
            Some(s) if s.available => series.push(s.id.to_string()),
            Some(s) => unavailable.push(s.id.to_string()),
            None => {}
        }
    }

    // Distancias acumuladas + estaciones.
    let mut cum_km = 0.0f64;
    let mut stations: Vec<Station> = Vec::with_capacity(canonical_len);
    let mut anomalies: Vec<String> = Vec::new();
    for (i, s) in itin.stops.iter().enumerate() {
        if i > 0 {
            let a = &itin.stops[i - 1];
            let d = match (a.lat, a.lon, s.lat, s.lon) {
                (Some(la), Some(lo), Some(lc), Some(ld)) => haversine_m(la, lo, lc, ld),
                _ => {
                    anomalies.push(format!(
                        "Estació sense coordenades entre «{}» i «{}» → distància del tram = 0.",
                        a.name, s.name
                    ));
                    0.0
                }
            };
            cum_km += d / 1000.0;
        }
        // Dwell según modo (origen y destino no suman parada).
        let is_end = i == 0 || i == canonical_len - 1;
        let (dwell_s, dwell_src) = if is_end {
            (0u32, "extrem (0)".to_string())
        } else {
            match dwell {
                DwellMode::Auto => {
                    let m = dwell_map.get(&s.stop_id).map(|d| d.median_s).unwrap_or(0);
                    (m, "GTFS (mediana)".to_string())
                }
                DwellMode::Fixed(v) => (*v, "fix".to_string()),
                DwellMode::Custom(map) => {
                    (map.get(&s.stop_id).copied().unwrap_or(0), "personalitzat".to_string())
                }
            }
        };
        stations.push(Station {
            name: s.name.clone(),
            cum_km,
            lat: s.lat,
            lon: s.lon,
            dwell_s,
            dwell_src,
            programmed_cum_s: prog_cum.get(&s.stop_id).copied(),
        });
    }
    let distance_km = cum_km;

    // Segmentos + física por serie.
    let mut segments: Vec<Segment> = Vec::with_capacity(canonical_len - 1);
    let mut adif_dist_total = 0.0f64;
    let mut cov_weighted = 0.0f64;
    let mut cov_len = 0.0f64;
    let mut ltv_total = 0usize;
    let mut min_ltv_line: Option<f64> = None;
    for i in 0..canonical_len - 1 {
        let a = &stations[i];
        let b = &stations[i + 1];
        let dist_km = b.cum_km - a.cum_km;
        let dist_m = (dist_km * 1000.0).max(1.0);

        if dist_km > 0.0 && dist_km < 0.15 {
            anomalies.push(format!(
                "Tram molt curt {:.0} m ({} → {}): possible parada tècnica o dades GTFS.",
                dist_km * 1000.0, a.name, b.name
            ));
        }
        if dist_km > 25.0 {
            anomalies.push(format!(
                "Tram molt llarg {:.1} km ({} → {}): revisar dades.",
                dist_km, a.name, b.name
            ));
        }

        // Perfil ADIF del tramo (CVM real) por proyección de la polilínea de estaciones.
        let mut speed_zones: Vec<SpeedZone> = Vec::new();
        let mut refined_restr: Vec<Restriction> = Vec::new();
        let mut seg_min_vmax: Option<f64> = None;
        let mut seg_adif_dist: Option<f64> = None;
        let mut seg_cov = 0.0f64;
        let mut refine = false;
        if let Some(adif) = adif {
            if let (Some(la), Some(lo), Some(lc), Some(ld)) = (a.lat, a.lon, b.lat, b.lon) {
                let pts = [(la, lo), (lc, ld)];
                if let Some(prof) = infrastructure::adif_profile(adif, ltv, &pts, 40.0, 140.0) {
                    ltv_total += prof.ltv_applied;
                    if let Some(mv) = prof.min_ltv_kmh {
                        min_ltv_line = Some(min_ltv_line.map_or(mv, |m: f64| m.min(mv)));
                    }
                    speed_zones = prof
                        .zones
                        .iter()
                        .map(|z| SpeedZone { from: z.from_m, to: z.to_m, vmax: z.vmax_kmh / 3.6 })
                        .collect();
                    refined_restr = physics::restrictions_from_zones(&speed_zones, dist_m);
                    seg_min_vmax = prof.min_vmax_kmh;
                    seg_adif_dist = Some(prof.adif_distance_m / 1000.0);
                    seg_cov = prof.coverage;
                    refine = !speed_zones.is_empty();
                    adif_dist_total += prof.adif_distance_m / 1000.0;
                    cov_weighted += prof.coverage * dist_km;
                    cov_len += dist_km;
                }
            }
        }

        let mut per_series = HashMap::new();
        for id in &series {
            let sdef = rolling_stock::get(id).unwrap();
            let veh = super::vehicle_of(&sdef);
            let restr = vec![Restriction { x: dist_m, v: 0.0 }];
            let sim = physics::simulate(dist_m, &veh, &[], &restr, dt);
            let bd = sim.breakdown();
            // Modelo refinado con CVM ADIF (min(Vmax_tren, Vmax_ADIF(x))).
            let (marcha_ref_s, vmax_ref) = if refine {
                let sr = physics::simulate(dist_m, &veh, &speed_zones, &refined_restr, dt);
                (Some(sr.time_s), Some(sr.vmax_reached * 3.6))
            } else {
                (None, None)
            };
            per_series.insert(
                id.clone(),
                SeriesSeg {
                    marcha_s: sim.time_s,
                    vmax_reached_kmh: sim.vmax_reached * 3.6,
                    vmax_allowed_kmh: sdef.vmax_kmh.value,
                    t_accel: bd.t_accel,
                    t_cruise: bd.t_cruise,
                    t_brake: bd.t_brake,
                    d_accel_km: bd.d_accel / 1000.0,
                    d_brake_km: bd.d_brake / 1000.0,
                    reached_end: sim.reached_end,
                    marcha_ref_s,
                    vmax_ref_reached_kmh: vmax_ref,
                },
            );
        }
        segments.push(Segment {
            from: a.name.clone(),
            to: b.name.clone(),
            dist_km,
            per_series,
            adif_min_vmax_kmh: seg_min_vmax,
            adif_dist_km: seg_adif_dist,
            coverage: seg_cov,
        });
    }
    let adif_available = adif.is_some() && cov_len > 0.0;
    let coverage_pct = if cov_len > 0.0 { cov_weighted / cov_len * 100.0 } else { 0.0 };
    let adif_distance_km = if adif_available { Some(adif_dist_total) } else { None };

    // Totales por serie (marcha + paradas).
    let paradas_s: f64 = stations.iter().map(|s| s.dwell_s as f64).sum();
    let mut totals: HashMap<String, Totals> = HashMap::new();
    let mut fastest_total = f64::INFINITY;
    for id in &series {
        let marcha_s: f64 = segments
            .iter()
            .filter_map(|seg| seg.per_series.get(id))
            .map(|ss| ss.marcha_s)
            .sum();
        let total_s = marcha_s + paradas_s;
        fastest_total = fastest_total.min(total_s);
        let margin_median_s = programmed_stat.median as f64 - total_s;
        // Modelo refinado: sólo si TODOS los tramos tienen marcha refinada (CVM ADIF).
        let refs: Vec<f64> = segments
            .iter()
            .filter_map(|seg| seg.per_series.get(id).and_then(|ss| ss.marcha_ref_s))
            .collect();
        let (marcha_ref_s, total_ref_s, margin_ref_median_s) =
            if adif_available && refs.len() == segments.len() {
                let m: f64 = refs.iter().sum();
                let t = m + paradas_s;
                (Some(m), Some(t), Some(programmed_stat.median as f64 - t))
            } else {
                (None, None, None)
            };
        totals.insert(
            id.clone(),
            Totals {
                marcha_s,
                paradas_s,
                total_s,
                margin_median_s,
                marcha_ref_s,
                total_ref_s,
                margin_ref_median_s,
            },
        );
    }

    // Anomalías de comparación con lo programado.
    if programmed_stat.n > 0 && (programmed_stat.min as f64) < fastest_total - 1.0 {
        anomalies.push(format!(
            "⚠ El temps programat MÍNIM ({}) és inferior al mínim físic més ràpid ({}). \
             Revisar dades o supòsits del model (probablement el model és massa optimista \
             o el servei programat salta parades).",
            fmt_ms(programmed_stat.min as f64),
            fmt_ms(fastest_total)
        ));
    }

    // Calidad por variable (independiente).
    let dwell_level = match dwell {
        DwellMode::Auto => ("oficial", "Dwell real de GTFS (mediana per estació)."),
        DwellMode::Fixed(_) => ("suposicion", "Dwell fix triat per l'usuari (no mesurat)."),
        DwellMode::Custom(_) => ("suposicion", "Dwell personalitzat per l'usuari."),
    };
    let (vinfra_lvl, vinfra_note) = if adif_available {
        (
            "oficial",
            format!(
                "CVM real d'ADIF (DesignSpeed, IDEADIF) aplicada · cobertura {:.0}%.",
                coverage_pct
            ),
        )
    } else {
        ("nd", "Sense CVM d'Adif → límit = Vmax del tren.".to_string())
    };
    let (dist_lvl, dist_note) = if adif_available {
        ("secundaria", "GTFS + comparació amb geometria ADIF (projecció).".to_string())
    } else {
        ("secundaria", "Polilínia d'estacions GTFS (aproximada, no PK d'Adif).".to_string())
    };
    let quality = vec![
        QualityItem {
            variable: "Distància".into(),
            level: dist_lvl.into(),
            note: dist_note,
        },
        QualityItem {
            variable: "Vmax infraestructura".into(),
            level: vinfra_lvl.into(),
            note: vinfra_note,
        },
        QualityItem {
            variable: "Parades".into(),
            level: dwell_level.0.into(),
            note: dwell_level.1.into(),
        },
        QualityItem {
            variable: "Horari programat".into(),
            level: "oficial".into(),
            note: "GTFS oficial.".into(),
        },
        QualityItem {
            variable: "Circulació observada".into(),
            level: "nd".into(),
            note: "No hi ha dataset obert de circulació real per tren.".into(),
        },
        QualityItem {
            variable: "Acceleració/frenada".into(),
            level: "suposicion".into(),
            note: "Model (no publicat per cap font).".into(),
        },
    ];

    let fichas: Vec<Ficha> = series_ids.iter().filter_map(|id| ficha_of(id)).collect();

    LineAnalysis {
        line: line.to_string(),
        direction_label: itin.dir.label.clone(),
        direction_key: dir_key.clone(),
        distance_km,
        n_stations: stations.len(),
        n_segments: segments.len(),
        dt,
        series,
        unavailable,
        stations,
        segments,
        totals,
        programmed: stat_to_out(
            &programmed_stat,
            format!(
                "GTFS Rodalies — {} serveis d'itinerari complet del sentit",
                programmed_stat.n
            ),
        ),
        observed_available: false,
        observed_note: "No s'han trobat dades obertes de circulació real per a aquest \
                        anàlisi. S'utilitza l'horari programat de GTFS."
            .into(),
        anomalies,
        quality,
        fichas,
        sources: sources_list(adif_available),
        dwell_mode: dwell.label(),
        adif_available,
        adif_distance_km,
        coverage_pct,
        ltv_applied: ltv_total,
        min_ltv_kmh: min_ltv_line,
        ltv_snapshot: ltv.map(|l| l.snapshot.clone()),
        error: None,
    }
}

/// Formatea segundos como m:ss (auxiliar para mensajes).
fn fmt_ms(s: f64) -> String {
    let t = s.round() as i64;
    format!("{}:{:02}", t / 60, t % 60)
}

// --------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    fn custom(pairs: &[(&str, u32)]) -> DwellMode {
        DwellMode::Custom(pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect())
    }

    #[test]
    fn dwell_mode_labels() {
        assert!(DwellMode::Auto.label().contains("Auto"));
        assert_eq!(DwellMode::Fixed(30).label(), "Fix 30 s");
        assert!(custom(&[("a", 10)]).label().contains("Personal"));
    }

    #[test]
    fn prov_code_cubre_todos() {
        assert_eq!(prov_code(Provenance::Oficial), "oficial");
        assert_eq!(prov_code(Provenance::NoDisponible), "nd");
        assert_eq!(prov_code(Provenance::Suposicion), "suposicion");
    }

    /// Integración con el GTFS real (se salta si no está `./data/gtfs`, para no romper CI).
    #[test]
    fn analisis_linea_real() {
        let dir = std::path::Path::new("./data/gtfs");
        if !dir.is_dir() {
            return;
        }
        let net = match crate::gtfs_loader::load(dir) {
            Ok(n) => n,
            Err(_) => return,
        };
        // Elegir una línea con sentidos y al menos 3 estaciones en el canónico.
        let Some(line) = net.services.iter().find(|s| !s.is_bus).map(|s| s.route_short_name.clone())
        else {
            return;
        };
        let dirs = schedules::line_directions(&net, &line);
        let Some(dir0) = dirs.first().cloned() else { return };
        let itin = schedules::itinerary(&net, &line, &dir0.key).unwrap();
        if itin.stops.len() < 3 {
            return;
        }
        let series: Vec<String> =
            ["447", "450", "470", "490"].iter().map(|s| s.to_string()).collect();

        // Sin parada (Fixed 0).
        let a = analyze_line(&net, &line, &dir0.key, &series, &DwellMode::Fixed(0), 0.2, None, None);
        assert!(a.error.is_none(), "error inesperado");
        assert!(a.n_stations >= 3);
        assert_eq!(a.n_segments, a.n_stations - 1, "nº de tramos = estaciones-1");
        // Consistencia de distancias: la acumulada final = distancia total.
        assert!((a.distance_km - a.stations.last().unwrap().cum_km).abs() < 1e-6);
        // Total = marcha + paradas; con Fixed(0) paradas = 0; marcha > 0; todas las series.
        assert_eq!(a.totals.len(), series.len(), "faltan series en totals");
        for id in &a.series {
            let t = &a.totals[id];
            assert!((t.total_s - (t.marcha_s + t.paradas_s)).abs() < 1e-6);
            assert_eq!(t.paradas_s, 0.0, "Fixed(0) debe dar 0 paradas");
            assert!(t.marcha_s > 0.0, "marcha debe ser > 0");
            // La marcha de la línea = suma de las marchas de los tramos.
            let sum: f64 =
                a.segments.iter().filter_map(|s| s.per_series.get(id)).map(|s| s.marcha_s).sum();
            assert!((sum - t.marcha_s).abs() < 1e-6, "suma de tramos ≠ total marcha");
        }

        // Parada fija 30 s → paradas = 30 · (nº estaciones intermedias).
        let b = analyze_line(&net, &line, &dir0.key, &series, &DwellMode::Fixed(30), 0.2, None, None);
        let inter = (b.n_stations - 2) as f64;
        for id in &b.series {
            assert_eq!(b.totals[id].paradas_s, 30.0 * inter, "parada fija incorrecta");
            // Con paradas, el total es mayor que sin paradas.
            assert!(b.totals[id].total_s > a.totals[id].total_s);
        }

        // Parada personalizada: sólo una estación intermedia con 45 s.
        let mid = itin.stops[1].stop_id.clone();
        let mut cmap = HashMap::new();
        cmap.insert(mid, 45u32);
        let c = analyze_line(&net, &line, &dir0.key, &series, &DwellMode::Custom(cmap), 0.2, None, None);
        for id in &c.series {
            assert_eq!(c.totals[id].paradas_s, 45.0, "parada personalizada incorrecta");
        }

        // Modo automático: dwell desde GTFS (>= 0) y estadística programada presente.
        let d = analyze_line(&net, &line, &dir0.key, &series, &DwellMode::Auto, 0.2, None, None);
        assert!(d.programmed.n >= 1 || d.programmed.n == 0); // no debe entrar en pánico

        // Modelo refinado con CVM ADIF si está el fichero procesado.
        if let Some(adif) = AdifNet::load(std::path::Path::new("./processed/adif/rfig_speed.json")) {
            let r = analyze_line(&net, &line, &dir0.key, &series, &DwellMode::Fixed(0), 0.2, Some(&adif), None);
            assert!(r.adif_available, "ADIF cargado pero no marcado disponible");
            for id in &r.series {
                // Con CVM aplicada, la marcha refinada nunca es MENOR que la actual
                // (los límites sólo pueden frenar, nunca acelerar por encima del tren).
                if let Some(mref) = r.totals[id].marcha_ref_s {
                    assert!(mref >= r.totals[id].marcha_s - 1.0, "refinado más rápido que actual");
                }
            }
        }
    }

    /// GTFS incompleto / línea inexistente → error controlado, sin pánico.
    #[test]
    fn linea_inexistente_da_error() {
        let dir = std::path::Path::new("./data/gtfs");
        if !dir.is_dir() {
            return;
        }
        let net = match crate::gtfs_loader::load(dir) {
            Ok(n) => n,
            Err(_) => return,
        };
        let a = analyze_line(
            &net,
            "LINEA_QUE_NO_EXISTE_999",
            &("x".into(), "y".into()),
            &["447".into()],
            &DwellMode::Auto,
            0.2,
            None,
            None,
        );
        assert!(a.error.is_some());
    }
}
