//! **Parámetros de realismo** del motor unificado (todo lo que el usuario puede ajustar) y la lista
//! de **supuestos** del modelo (lo que se asume sin dato o con un dato aproximado).
//!
//! Los valores por defecto son los que se proponen como razonables, cada uno con su motivo/fuente.
//! Se guardan en `report/realism.json` (lo edita la página `/configuracion`); si el fichero falta o
//! le faltan campos, se completan con los de por defecto (`#[serde(default)]`): así añadir un
//! parámetro nuevo nunca rompe un fichero guardado.

use serde::{Deserialize, Serialize};

pub const REALISM_PATH: &str = "report/realism.json";

/// Ficha operativa de una serie de material (lo que la física de `rolling_stock` no tiene):
/// capacidad, puertas y longitud de UNA unidad. Valores aproximados (ver supuestos).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SeriesOps {
    pub id: String,
    /// Plazas totales (sentadas + de pie a 4 pers./m²) de una unidad.
    pub capacity: f64,
    /// Puertas por costado de una unidad (cada puerta = un flujo de subida/bajada).
    pub doors: f64,
    /// Longitud de una unidad (m).
    pub length_m: f64,
    /// Factor de tiempo por viajero (dos pisos = escaleras interiores → más lento que 1).
    pub door_factor: f64,
    /// ¿Circula en doble composición en hora punta? (unidades acoplables de cercanías).
    pub double_in_peak: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct RealismCfg {
    // ---- Pasajeros y tiempo de parada ----
    /// Parada calculada por los viajeros que suben/bajan (si no, el modelo antiguo fijo 20–60 s).
    pub pax_enabled: bool,
    /// Multiplicador de la demanda real (Renfe 2018: 369.477 viajeros/día en el núcleo).
    pub demand_scale: f64,
    /// Variabilidad de la demanda de cada estación en cada run (σ de un factor log-normal).
    pub demand_noise: f64,
    /// Demanda diaria de una estación SIN datos de Renfe (fuera del núcleo de Barcelona), por cada
    /// tren que para allí. Se escala con el perfil horario medio del núcleo.
    pub fallback_pax_per_stop: f64,
    /// Parte fija de toda parada: abrir puertas, avisos, cerrar puertas y orden de salida (s).
    pub door_fixed_s: f64,
    /// Segundos por viajero que SUBE por cada puerta.
    pub board_s_per_pax: f64,
    /// Segundos por viajero que BAJA por cada puerta.
    pub alight_s_per_pax: f64,
    /// Ocupación (0–1) a partir de la cual el tren lleno frena el intercambio (gente en las puertas).
    pub crowd_threshold: f64,
    /// Penalización del intercambio con el tren lleno: ×(1 + este valor) al 100 % de ocupación.
    pub crowd_penalty: f64,
    /// Parada mínima en cualquier estación con viajeros (s).
    pub dwell_min_s: f64,
    /// Probabilidad de que una parada concreta se alargue por un imprevisto menor (puertas
    /// retenidas, viajero con rampa, bicicleta…).
    pub extra_dwell_prob: f64,
    /// Duración máxima de ese alargamiento (s); se sortea uniforme entre 10 s y este valor.
    pub extra_dwell_max_s: f64,
    /// Peso medio de un viajero (kg) — suma a la masa del tren y le quita aceleración.
    pub pax_mass_kg: f64,
    /// Franjas de hora punta (h desde medianoche) para la doble composición.
    pub peak_windows: Vec<(f64, f64)>,

    // ---- Rotación de material en cabecera ----
    /// La unidad que llega a cabecera es la que hace el siguiente servicio (el retraso se propaga).
    pub rotation_enabled: bool,
    /// Tiempo mínimo de cambio de cabina del maquinista y preparación (s).
    pub turnaround_min_s: f64,
    /// Espera máxima programada entre llegada y siguiente salida para encadenar (min). Más allá la
    /// unidad se va a cocheras y el siguiente servicio sale con otra.
    pub rotation_max_wait_min: f64,
    /// Si la unidad entrante no llega, a los X min de la hora de salida sale una unidad de reserva.
    pub reserve_unit_after_min: f64,

    // ---- Conducción real ----
    /// ASFA: al pasar una señal en anuncio de parada (ámbar) el tren debe bajar a `asfa_amber_kmh`
    /// y no puede volver a acelerar hasta la señal siguiente aunque esta se abra.
    pub asfa_enabled: bool,
    pub asfa_amber_kmh: f64,
    /// Deceleración de servicio que usa el maquinista (m/s²); la del material es la máxima.
    pub service_decel: f64,
    /// Fracción de la aceleración máxima que aplica el maquinista.
    pub accel_fraction: f64,
    /// Frenar ANTES de cada reducción de velocidad de la vía (si no, el límite cae de golpe).
    pub anticipate_limits: bool,
    /// Tiempo de reacción del maquinista al abrirse una señal estando parado (s, uniforme min–max).
    pub reaction_min_s: f64,
    pub reaction_max_s: f64,
    /// Margen bajo la Vmax en los túneles de Barcelona (factor sobre la Vmax de la vía).
    pub tunnel_speed_factor: f64,
    /// Estaciones subterráneas: un tramo entre dos de ellas se considera en túnel.
    pub tunnel_stations: Vec<String>,
    /// Resistencia al avance (Davis) estimada: N/kN de peso a velocidad 0 y a 100 km/h.
    pub davis_enabled: bool,
    pub davis_r0: f64,
    pub davis_r100: f64,
    /// Variabilidad de velocidad del maquinista por defecto (nadie circula clavado a Vmax): valor inicial
    /// del modo visual, las simulaciones encadenadas y la comparación (cada run puede cambiarlo).
    pub speed_var_enabled: bool,
    /// Factor de aleatoriedad a 120 km/h (%): media Vmax − p %, entre Vmax − 2p % y Vmax.
    pub speed_var_pct: f64,

    // ---- Vía ----
    /// Rellenar los huecos sin dato de velocidad de vía (OSM/ADIF) dentro de cada tramo.
    pub fill_track_gaps: bool,
    /// Vmax de vía (km/h) donde no hay NINGÚN dato en todo el tramo.
    pub default_track_kmh: f64,

    // ---- Cabeceras ----
    /// El tren que sale de una vía de estacionamiento cruza la vía contraria: las dos señales del
    /// cruce se ponen en rojo mientras se incorpora.
    pub terminal_crossing: bool,
    /// Tiempo que el cruce queda ocupado por la maniobra de incorporación (s).
    pub crossing_time_s: f64,

    // ---- Mercancías ----
    /// Los mercancías se apartan en estaciones con vías secundarias para dejar pasar al de atrás.
    pub freight_yield: bool,
    /// Distancia hacia atrás (m) en la que se mira si viene un tren de viajeros.
    pub freight_yield_lookback_m: f64,
    /// Distancia (m) a la estación con apartadero a partir de la cual se decide apartarse.
    pub freight_yield_decide_m: f64,
    /// Espera máxima en un apartadero (s) antes de salir igualmente.
    pub freight_siding_max_s: f64,
    /// Ventana (s) en la que un mercancías cede el paso en una bifurcación o vía única a un tren
    /// de viajeros que se acerca.
    pub freight_yield_window_s: f64,

    // ---- Prioridad de los regionales sobre los cercanías ----
    /// En cabecera, un cercanías espera a salir si detrás viene un regional RÁPIDO (que se salta paradas
    /// que él hace) que pasa por esa estación en la misma dirección: así no sale delante y lo frena.
    pub commuter_wait_regional: bool,
    /// Espera máxima (min) sobre la hora de salida del cercanías: pasado ese margen sale igualmente.
    pub commuter_wait_margin_min: f64,
    /// Un regional es «rápido» si se salta MÁS de estas paradas en todo su recorrido…
    pub fast_skips_total_gt: u32,
    /// …y al menos estas coinciden con paradas que sí hace el cercanías (después de la cabecera).
    pub fast_common_min: u32,
    /// Los cercanías se apartan (como los mercancías) en estaciones con más de 2 vías para dejar pasar
    /// al regional que les sigue.
    pub commuter_yield: bool,
    /// Distancia hacia atrás (m) en la que el cercanías mira si viene un regional.
    pub commuter_yield_lookback_m: f64,
    /// Distancia (m) a su próxima parada a partir de la cual decide apartarse en ella.
    pub commuter_yield_decide_m: f64,
    /// Espera máxima apartado (s) antes de salir igualmente.
    pub commuter_siding_max_s: f64,
    /// Solo se aparta si el regional se salta al menos estas de sus próximas paradas (si las hace
    /// todas, adelantarlo no ahorra nada).
    pub commuter_yield_min_skips: u32,
    /// Solo se aparta si el regional va FRENADO por él (ámbar del ASFA o retenido detrás).
    pub commuter_yield_only_hindered: bool,

    // ---- Material ----
    pub series: Vec<SeriesOps>,
}

impl Default for RealismCfg {
    fn default() -> Self {
        RealismCfg {
            pax_enabled: true,
            demand_scale: 1.0,
            demand_noise: 0.15,
            fallback_pax_per_stop: 12.0,
            door_fixed_s: 18.0,
            board_s_per_pax: 1.5,
            alight_s_per_pax: 1.1,
            crowd_threshold: 0.75,
            crowd_penalty: 1.0,
            dwell_min_s: 20.0,
            extra_dwell_prob: 0.03,
            extra_dwell_max_s: 75.0,
            pax_mass_kg: 75.0,
            peak_windows: vec![(6.5, 9.5), (17.0, 20.0)],

            rotation_enabled: true,
            turnaround_min_s: 180.0,
            rotation_max_wait_min: 90.0,
            reserve_unit_after_min: 45.0,

            asfa_enabled: true,
            asfa_amber_kmh: 60.0,
            service_decel: 0.6,
            accel_fraction: 0.75,
            anticipate_limits: true,
            reaction_min_s: 3.0,
            reaction_max_s: 8.0,
            tunnel_speed_factor: 0.9,
            tunnel_stations: [
                "Barcelona-Sants",
                "Barcelona Plaça de Catalunya",
                "Barcelona Arc de Triomf",
                "Barcelona-Passeig de Gràcia",
                "Barcelona El Clot",
                "Barcelona La Sagrera-Meridiana",
                "Barcelona Fabra i Puig",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            davis_enabled: true,
            davis_r0: 1.6,
            davis_r100: 4.0,

            speed_var_enabled: true,
            speed_var_pct: 2.0,
            fill_track_gaps: true,
            default_track_kmh: 100.0,

            terminal_crossing: true,
            crossing_time_s: 40.0,

            freight_yield: true,
            freight_yield_lookback_m: 6000.0,
            freight_yield_decide_m: 2500.0,
            freight_siding_max_s: 1200.0,
            freight_yield_window_s: 150.0,

            commuter_wait_regional: true,
            commuter_wait_margin_min: 5.0,
            fast_skips_total_gt: 4,
            fast_common_min: 3,
            commuter_yield: true,
            commuter_yield_lookback_m: 3000.0,
            commuter_yield_decide_m: 2500.0,
            commuter_siding_max_s: 300.0,
            commuter_yield_min_skips: 2,
            commuter_yield_only_hindered: true,

            series: default_series(),
        }
    }
}

fn default_series() -> Vec<SeriesOps> {
    let s = |id: &str, capacity: f64, doors: f64, length_m: f64, door_factor: f64, double_in_peak: bool| SeriesOps {
        id: id.into(),
        capacity,
        doors,
        length_m,
        door_factor,
        double_in_peak,
    };
    vec![
        // 447: 3 coches, 3 puertas dobles por costado y coche (diseñada para cercanías de alta carga).
        s("447", 650.0, 9.0, 76.0, 1.0, true),
        // 450: 6 coches de dos pisos, 2 puertas por costado y coche; el doble piso ralentiza.
        s("450", 1450.0, 12.0, 161.0, 1.3, false),
        // 465 Civia (5 coches): 2 puertas por costado y coche, piso bajo.
        s("465", 900.0, 10.0, 98.0, 0.9, true),
        // 470: regional de 3 coches, 1 puerta por costado y coche.
        s("470", 300.0, 3.0, 75.0, 1.2, false),
        // 490/456/otros: respaldo genérico.
        s("490", 350.0, 4.0, 80.0, 1.1, false),
    ]
}

impl RealismCfg {
    /// Lee `report/realism.json`; si falta o no se puede leer, los valores por defecto.
    pub fn load() -> Self {
        std::fs::read_to_string(REALISM_PATH)
            .ok()
            .and_then(|s| serde_json::from_str::<RealismCfg>(&s).ok())
            .map(|c| c.sanitized())
            .unwrap_or_default()
    }

    pub fn save(&self) -> bool {
        std::fs::create_dir_all("report").ok();
        std::fs::write(REALISM_PATH, serde_json::to_string_pretty(self).unwrap_or_default()).is_ok()
    }

    /// Acota valores imposibles que vengan del formulario (negativos, factores fuera de rango).
    pub fn sanitized(mut self) -> Self {
        let pos = |v: f64, d: f64| if v.is_finite() && v >= 0.0 { v } else { d };
        let d = RealismCfg::default();
        self.demand_scale = pos(self.demand_scale, d.demand_scale);
        self.demand_noise = pos(self.demand_noise, d.demand_noise).min(1.0);
        self.fallback_pax_per_stop = pos(self.fallback_pax_per_stop, d.fallback_pax_per_stop);
        self.door_fixed_s = pos(self.door_fixed_s, d.door_fixed_s);
        self.board_s_per_pax = pos(self.board_s_per_pax, d.board_s_per_pax);
        self.alight_s_per_pax = pos(self.alight_s_per_pax, d.alight_s_per_pax);
        self.crowd_threshold = pos(self.crowd_threshold, d.crowd_threshold).clamp(0.05, 1.0);
        self.crowd_penalty = pos(self.crowd_penalty, d.crowd_penalty);
        self.dwell_min_s = pos(self.dwell_min_s, d.dwell_min_s);
        self.extra_dwell_prob = pos(self.extra_dwell_prob, d.extra_dwell_prob).min(1.0);
        self.extra_dwell_max_s = pos(self.extra_dwell_max_s, d.extra_dwell_max_s).max(10.0);
        self.pax_mass_kg = pos(self.pax_mass_kg, d.pax_mass_kg);
        self.turnaround_min_s = pos(self.turnaround_min_s, d.turnaround_min_s);
        self.rotation_max_wait_min = pos(self.rotation_max_wait_min, d.rotation_max_wait_min);
        self.reserve_unit_after_min = pos(self.reserve_unit_after_min, d.reserve_unit_after_min);
        self.asfa_amber_kmh = pos(self.asfa_amber_kmh, d.asfa_amber_kmh).max(10.0);
        self.service_decel = pos(self.service_decel, d.service_decel).clamp(0.1, 1.5);
        self.accel_fraction = pos(self.accel_fraction, d.accel_fraction).clamp(0.1, 1.0);
        self.reaction_min_s = pos(self.reaction_min_s, d.reaction_min_s);
        self.reaction_max_s = pos(self.reaction_max_s, d.reaction_max_s).max(self.reaction_min_s);
        self.tunnel_speed_factor = pos(self.tunnel_speed_factor, d.tunnel_speed_factor).clamp(0.3, 1.0);
        self.davis_r0 = pos(self.davis_r0, d.davis_r0);
        self.davis_r100 = pos(self.davis_r100, d.davis_r100);
        self.speed_var_pct = pos(self.speed_var_pct, d.speed_var_pct).clamp(0.5, 25.0);
        self.default_track_kmh = pos(self.default_track_kmh, d.default_track_kmh).max(10.0);
        self.crossing_time_s = pos(self.crossing_time_s, d.crossing_time_s);
        self.freight_yield_lookback_m = pos(self.freight_yield_lookback_m, d.freight_yield_lookback_m);
        self.freight_yield_decide_m = pos(self.freight_yield_decide_m, d.freight_yield_decide_m);
        self.freight_siding_max_s = pos(self.freight_siding_max_s, d.freight_siding_max_s);
        self.freight_yield_window_s = pos(self.freight_yield_window_s, d.freight_yield_window_s);
        self.commuter_wait_margin_min = pos(self.commuter_wait_margin_min, d.commuter_wait_margin_min).min(30.0);
        self.commuter_yield_lookback_m = pos(self.commuter_yield_lookback_m, d.commuter_yield_lookback_m);
        self.commuter_yield_decide_m = pos(self.commuter_yield_decide_m, d.commuter_yield_decide_m);
        self.commuter_siding_max_s = pos(self.commuter_siding_max_s, d.commuter_siding_max_s);
        self.fast_common_min = self.fast_common_min.max(1);
        self.commuter_yield_min_skips = self.commuter_yield_min_skips.max(1);
        if self.series.is_empty() {
            self.series = d.series;
        }
        self
    }

    /// Ficha operativa de una serie (con respaldo genérico).
    pub fn series_ops(&self, id: &str) -> SeriesOps {
        self.series
            .iter()
            .find(|s| s.id == id)
            .or_else(|| self.series.iter().find(|s| s.id == "490"))
            .cloned()
            .unwrap_or_else(|| default_series().pop().unwrap())
    }

    /// ¿Es hora punta en `t` (s desde medianoche)?
    pub fn is_peak(&self, t: f64) -> bool {
        let h = t / 3600.0;
        self.peak_windows.iter().any(|&(a, b)| h >= a && h < b)
    }
}

/// Un supuesto del modelo, para la página `/supuestos`.
#[derive(Serialize)]
pub struct Assumption {
    pub area: &'static str,
    pub text: &'static str,
    /// "dato" (dato real) · "aprox" (dato aproximado/derivado) · "supuesto" (sin dato).
    pub level: &'static str,
    pub source: &'static str,
    /// Enlaces a la fuente: `(texto, URL)`. En un PDF, el texto lleva artículo y página y la URL
    /// abre esa página (`#page=N`).
    pub links: &'static [(&'static str, &'static str)],
}

// ---- Fuentes de los datos reales (enlaces de `/supuestos`) -------------------------------------
// Las páginas de PDF se comprobaron sobre la versión consolidada del BOE descargada el 19/10/2026;
// si el BOE publica otra consolidación, la paginación puede moverse.

const SRC_GTFS: &[(&str, &str)] = &[
    ("Renfe Data · Horarios de Cercanías (GTFS)", "https://data.renfe.com/dataset/horarios-cercanias"),
    ("Descarga directa: fomento_transit.zip", "https://ssl.renfe.com/ftransit/Fichero_CER_FOMENTO/fomento_transit.zip"),
];
const SRC_OSM: &[(&str, &str)] = &[
    ("OpenStreetMap (© colaboradores de OSM, licencia ODbL)", "https://www.openstreetmap.org/copyright"),
    ("API Overpass (consulta railway=rail)", "https://overpass-api.de/"),
];
const SRC_VMAX: &[(&str, &str)] = &[
    ("OpenStreetMap (maxspeed, © colaboradores de OSM, ODbL)", "https://www.openstreetmap.org/copyright"),
    ("Adif · Red de Transporte Ferroviario (INSPIRE), ficha en data.europa.eu", "https://data.europa.eu/data/datasets/62c487b1-6816-4594-a800-218abe896994?locale=es"),
    (
        "Adif · WFS de la Red de Transporte Ferroviario (IDEADIF, capa tn-ra:DesignSpeed), ficha en el catálogo INSPIRE del IGN",
        "https://www.idee.es/csw-codsi-idee/srv/api/records/8d185f60-78f3-4355-8aa3-153757cdb663?language=spa",
    ),
];
const SRC_LTV: &[(&str, &str)] = &[
    ("Adif · Limitaciones Temporales de Velocidad (portal)", "https://ltv-adif.hub.arcgis.com"),
    ("Servicio LTV_2 (ArcGIS FeatureServer)", "https://services7.arcgis.com/XTupIrLX53AjaJqO/arcgis/rest/services/LTV_2/FeatureServer/0"),
];
const SRC_RCF_BLOQUEO: &[(&str, &str)] = &[(
    "BOE · Reglamento de Circulación Ferroviaria (RD 664/2015, consolidado), art. 4.2.1 «Bloqueo automático de vía única, vía doble y vías banalizadas», p. 125",
    "https://www.boe.es/buscar/pdf/2015/BOE-A-2015-8042-consolidado.pdf#page=125",
)];
const SRC_RCF_ASFA: &[(&str, &str)] = &[
    (
        "BOE · Reglamento de Circulación Ferroviaria (RD 664/2015, consolidado), art. 2.1.2.5 «Anuncio de parada», p. 37",
        "https://www.boe.es/buscar/pdf/2015/BOE-A-2015-8042-consolidado.pdf#page=37",
    ),
    (
        "Ídem, Libro Quinto, Apéndice 1 (ASFA Digital), art. 5AP1.1.4.2, punto 3 «Anuncio de parada y anuncio de parada inmediata»: 60 km/h con Tipo ≤ 100 y 80 km/h con Tipo > 100 (modo convencional), p. 182",
        "https://www.boe.es/buscar/pdf/2015/BOE-A-2015-8042-consolidado.pdf#page=182",
    ),
];
const SRC_TRENES: &[(&str, &str)] = &[
    ("Renfe Data · Fichas de nuestros trenes", "https://data.renfe.com/dataset/fichas-nuestros-trenes"),
    (
        "informacion-trenes.csv (velocidad máxima, potencia total, masa sin carga)",
        "https://data.renfe.com/dataset/d5524151-d959-40a0-a3ba-4546e7533b11/resource/4877096a-160f-4fb2-925d-d261b51a065f/download/informacion-trenes.csv",
    ),
    ("Wikipedia (EN) · Renfe Class 447", "https://en.wikipedia.org/wiki/Renfe_Class_447"),
    ("Wikipedia (EN) · Renfe Class 490", "https://en.wikipedia.org/wiki/Renfe_Class_490"),
];
const SRC_DEMANDA: &[(&str, &str)] = &[
    ("Renfe Data · Volumen de viajeros por franja horaria (Barcelona)", "https://data.renfe.com/dataset/volumen-de-viajeros-por-franja-horaria-barcelona"),
    (
        "barcelona_viajeros_por_franja_csv.csv",
        "https://data.renfe.com/dataset/9190f983-e138-42da-901a-b37205562fe4/resource/1417396e-4d6a-466a-a987-03d07aa92bed/download/barcelona_viajeros_por_franja_csv.csv",
    ),
];
const SRC_VIAS_ESTACION: &[(&str, &str)] = &[("trenscat.com · fichas y esquemas de vías de las estaciones", "http://www.trenscat.com/renfe/")];
/// Apartaderos de cruce: el nodo de Adif en OSM que les da nombre y una de sus vías de apartado.
const SRC_APARTADEROS: &[(&str, &str)] = &[
    ("Escatrón (Samper–Caspe, km 8,1) · nodo OSM de Adif", "https://www.openstreetmap.org/node/13378853527"),
    ("Escatrón · vía de apartado (OSM)", "https://www.openstreetmap.org/way/1375194589"),
    ("Chiprana (Samper–Caspe, km 20,5) · nodo OSM de Adif", "https://www.openstreetmap.org/node/5843870851"),
    ("Chiprana · vía de apartado (OSM)", "https://www.openstreetmap.org/way/1375194597"),
    ("Val de Pilas (Caspe–Fabara, km 11,2) · nodo OSM de Adif", "https://www.openstreetmap.org/node/13379272986"),
    ("Val de Pilas · vía de apartado (OSM)", "https://www.openstreetmap.org/way/1375221551"),
    ("Els Guiamets (Móra la Nova–Capçanes, km 9,4) · nodo OSM de Adif", "https://www.openstreetmap.org/node/2899831261"),
    ("Els Guiamets · vía de apartado (OSM)", "https://www.openstreetmap.org/way/262738768"),
    ("Canal d'Urgell (La Floresta–Vinaixa, km 3,8) · nodo OSM de Adif", "https://www.openstreetmap.org/node/12156993927"),
    ("Canal d'Urgell · vía de apartado (OSM)", "https://www.openstreetmap.org/way/572817587"),
];
const SRC_TORTOSA: &[(&str, &str)] = &[
    ("Tortosa · vía de apartado 1 (OSM)", "https://www.openstreetmap.org/way/207502600"),
    ("Tortosa · vía de apartado 2 (OSM)", "https://www.openstreetmap.org/way/207502601"),
    ("Tortosa · vía de apartado 3 (OSM)", "https://www.openstreetmap.org/way/207502602"),
];
const SRC_DWELL: &[(&str, &str)] = &[
    (
        "TCRP Report 165 · Transit Capacity and Quality of Service Manual, 3.ª ed. (2013), cap. 8, Exhibit 8-12 «Selection of Rail Transit Door Flow Times», p. 8-25 (PDF de ~200 MB)",
        "https://onlinepubs.trb.org/onlinepubs/tcrp/tcrp_rpt_165ch-08.pdf#page=30",
    ),
    (
        "Ídem, Exhibit 8-13 «Summary of Rail Transit Average Door Flow Times», p. 8-26",
        "https://onlinepubs.trb.org/onlinepubs/tcrp/tcrp_rpt_165ch-08.pdf#page=31",
    ),
    ("Ficha del manual en National Academies Press", "https://www.nationalacademies.org/publications/24766"),
];
const NONE: &[(&str, &str)] = &[];

/// Todo lo que el modelo ASUME. Mantener al día al añadir física: si se simplifica algo, se
/// declara aquí (regla del proyecto: el realismo sacrificado se declara, no se esconde). Todo dato
/// real lleva su fuente enlazada (y la página, si es un PDF).
pub fn assumptions() -> Vec<Assumption> {
    let a = |area, text, level, source, links| Assumption { area, text, level, source, links };
    vec![
        // Red y horarios
        a("Red", "La red (estaciones, tramos y orden de paradas) se construye íntegramente desde el GTFS de Renfe; no hay mapa fijo.", "dato", "GTFS Renfe Cercanías (Rodalies)", SRC_GTFS),
        a("Red", "Se simula el día tipo dominante del GTFS (laborable); sábados y festivos no se distinguen.", "supuesto", "—", NONE),
        a("Red", "La geometría y longitud de cada tramo salen del trazado real de OSM con restricción de ángulo de giro; si falta, la recta entre estaciones.", "dato", "OpenStreetMap (scripts/fetch_osm_track.py)", SRC_OSM),
        a("Red", "Vía única solo donde está declarada en la topología (R3 norte de Montcada, etc.); el resto es vía doble.", "aprox", "topology.rs (datos públicos / referencia Godot)", NONE),
        a("Red", "En vía única, los trenes se cruzan en las estaciones y en los apartaderos de cruce sin parada de viajeros: Escatrón y Chiprana (Samper–Caspe), Val de Pilas (Caspe–Fabara), Els Guiamets (Móra la Nova–Capçanes) y Canal d'Urgell (La Floresta–Vinaixa), detectados como vías de apartado de OSM con los dos extremos sobre la vía principal.", "dato", "OpenStreetMap (scripts/fetch_osm_track.py)", SRC_APARTADEROS),
        a("Red", "Número de vías de andén por estación según tabla propia; 2 por defecto.", "aprox", "topology.rs · trenscat.com", SRC_VIAS_ESTACION),
        a("Red", "Los cercanías apartados no comprueban cuántas vías de apartado quedan libres en la estación (como los mercancías).", "supuesto", "Modelo", NONE),
        a("Red", "Tortosa (final del ramal desde L'Aldea) admite 2 trenes en la llegada: vía principal más 3 vías de apartado de 241-428 m.", "dato", "OpenStreetMap", SRC_TORTOSA),
        // Señalización
        a("Señalización", "Bloqueo automático con señales cada ~1,6 km (máx. 3-5 por tramo según longitud), 1 tren por bloque; no se usan las posiciones reales de las señales.", "supuesto", "Modelo (el concepto de bloqueo, del Reglamento de Circulación)", SRC_RCF_BLOQUEO),
        a("Señalización", "Aspectos de 3 indicaciones (verde, anuncio de parada, parada). El ASFA obliga a bajar a la velocidad de control al pasar un ámbar y a mantenerla hasta la señal siguiente. El reglamento fija 60 km/h para trenes de Tipo ≤ 100 y 80 km/h para Tipo > 100 (modo convencional); el modelo usa 60 km/h para todos (configurable).", "aprox", "Reglamento de Circulación Ferroviaria / ASFA Digital (simplificado)", SRC_RCF_ASFA),
        a("Señalización", "Los conflictos en bifurcaciones se resuelven por orden de llegada, salvo prioridad de regionales en vía única y de viajeros frente a mercancías.", "supuesto", "Modelo (sin regulador del CTC)", NONE),
        a("Red", "Reproducción visual de un día real: cada tren se sitúa sobre su recorrido del GTFS (emparejado por número y línea) proyectando su GPS sobre la vía real e interpolando entre posiciones. El GPS del feed solo cambia en ~1 de cada 4 refrescos: si tras una posición repetida el tren aparece más lejos de lo que puede recorrer arrancando, se adelanta su salida. Se descartan las coordenadas falsas («en camino a X» con las de la estación siguiente), las de más de 15 min antes de su salida y, como reubicación, los saltos de más de 200 km/h. Si el feed empieza o acaba el viaje a mitad de recorrido (≤ 30 km), se completa hasta el origen o el destino a su hora más el retraso. Los semáforos son solo visuales.", "aprox", "Renfe GTFS-RT (posiciones) · scripts/gtfsrt_track.py", SRC_GTFS),
        a("Señalización", "Prioridad de los regionales rápidos (los que se saltan más de 4 paradas, al menos 3 de ellas del cercanías): el cercanías espera en su cabecera hasta 5 min a que pase uno que viene detrás en su dirección, y se aparta en una parada con más de 2 vías si el regional va frenado detrás de él (hasta 5 min). Todo configurable; el CTC real regula caso a caso.", "supuesto", "Modelo (regla del usuario)", NONE),
        a("Señalización", "Si en vía única viene un tren de frente y hay un apartadero de cruce entre los dos, el tren entra y para en el último apartadero antes del cruce (parada técnica, sin viajeros). En la realidad el punto de cruce lo fijan el horario y el CTC.", "supuesto", "Modelo (sin regulador del CTC)", NONE),
        // Velocidades
        a("Velocidad", "Vmax de vía por tramo desde OSM (maxspeed); donde OSM no tiene dato (tramo aragonés de la R15, variante de Vandellòs de la R16), la velocidad de diseño de ADIF. Suelo de 30 km/h a los valores de maniobra de OSM.", "dato", "OSM / ADIF IDEADIF", SRC_VMAX),
        a("Velocidad", "Los huecos sin dato dentro de un tramo se rellenan con la menor velocidad de los vecinos; un tramo sin ningún dato usa la Vmax por defecto configurable.", "supuesto", "Modelo", NONE),
        a("Velocidad", "Limitaciones Temporales de Velocidad (LTV) de ADIF del día en que se descargaron.", "dato", "ADIF HUB LTV (ArcGIS)", SRC_LTV),
        a("Velocidad", "En los túneles de Barcelona el maquinista circula con un margen bajo la Vmax (factor configurable).", "supuesto", "Modelo", NONE),
        a("Velocidad", "No hay pendientes: la vía se considera horizontal (sin altimetría).", "supuesto", "—", NONE),
        // Material
        a("Material", "Material por línea: R2/R2N/R2S → 450; regionales → 470; R1/R3/R4 → 465 (Civia); resto → 447.", "aprox", "Asignación del usuario", NONE),
        a("Material", "Vmax, potencia y masa del material de la ficha oficial de Renfe; aceleración de arranque y deceleración NO publicadas (supuestas).", "aprox", "Renfe informacion-trenes.csv", SRC_TRENES),
        a("Material", "Tracción a potencia constante con tope de arranque; resistencia al avance Davis ESTIMADA (no publicada por serie).", "supuesto", "Modelo", NONE),
        a("Material", "Capacidad, puertas y longitud por serie aproximadas; doble composición en hora punta para 447 y 465.", "aprox", "Fichas públicas aproximadas", SRC_TRENES),
        a("Material", "Sin averías de material (fiabilidad no modelada).", "supuesto", "Decisión del usuario", NONE),
        // Pasajeros
        a("Pasajeros", "Viajeros que suben y bajan por estación y por franja de 30 min: datos reales de Renfe del núcleo de Barcelona (2018).", "dato", "Renfe Data · Volumen de viajeros por franja horaria (Barcelona)", SRC_DEMANDA),
        a("Pasajeros", "Estaciones fuera del núcleo de Barcelona: demanda estimada por nº de trenes que paran, con el perfil horario medio.", "supuesto", "Modelo", NONE),
        a("Pasajeros", "Cada viajero coge el primer tren de su línea y sentido; se reparten entre líneas según los trenes que pasan por la estación.", "supuesto", "Modelo", NONE),
        a("Pasajeros", "Los viajeros bajan en cada parada en proporción a los que bajan allí según Renfe (sin matriz origen-destino).", "aprox", "Renfe 2018 (bajadas por estación)", SRC_DEMANDA),
        a("Pasajeros", "Tiempo de parada = parte fija + viajeros × segundos por viajero / puertas, penalizado con el tren lleno, más un imprevisto menor ocasional. El modelo usa 1,5 s al subir y 1,1 s al bajar por puerta; el TCQSM mide 1,1–2,6 s al subir y 1,4–2,0 s al bajar por flujo de puerta con embarque a nivel.", "aprox", "TCQSM 3.ª ed. (TCRP Report 165), tiempos de flujo por puerta", SRC_DWELL),
        a("Pasajeros", "Si no caben, los viajeros se quedan en el andén esperando el siguiente tren.", "supuesto", "Modelo", NONE),
        // Operación
        a("Operación", "La unidad que llega a una cabecera hace el siguiente servicio de la misma línea (o de la misma serie) que sale de allí, con 3 min de cambio de cabina; el GTFS no trae la rotación real (block_id = viaje).", "supuesto", "Inferido del horario", NONE),
        a("Operación", "Un tren nunca sale de una estación antes de su hora; si llega tarde a cabecera, sale en cuanto puede (sin supresiones).", "aprox", "Decisión del usuario", NONE),
        a("Operación", "Al salir de una vía de estacionamiento el tren cruza la vía contraria: ambas señales del cruce en rojo durante la maniobra.", "aprox", "Modelo", NONE),
        a("Operación", "Capacidad de las vías de estacionamiento de cabecera ilimitada.", "supuesto", "—", NONE),
        a("Operación", "Sin relevos de maquinista ni falta de personal.", "supuesto", "Decisión del usuario", NONE),
        // Mercancías
        a("Mercancías", "No hay datos abiertos de surcos de mercancías: se generan semialeatoriamente por corredor con una cadencia diaria configurable, sobre todo de noche y fuera de las horas punta.", "supuesto", "Modelo", NONE),
        a("Mercancías", "Los mercancías solo circulan por tramos que también recorre Rodalies (el grafo del GTFS); aparecen y desaparecen en las mismas estaciones origen/destino de su corredor.", "aprox", "Modelo", NONE),
        a("Mercancías", "Se apartan en estaciones con más de 2 vías y en los apartaderos de cruce para dejar pasar al tren de viajeros que les sigue; ceden el paso en bifurcaciones y vía única.", "aprox", "Reglamento (prioridad de viajeros)", NONE),
        // Incidencias
        a("Incidencias", "Las incidencias se reducen a dos efectos: señal en rojo (uno o dos sentidos) o tramo doble que pasa a vía única; se ubican según pesos espaciales y la demanda horaria.", "supuesto", "Modelo", NONE),
    ]
}
