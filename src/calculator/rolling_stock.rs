//! Base de datos INTERNA del material rodante (series de tren) para el calculador
//! de tiempo mínimo. Los datos van **separados del algoritmo** (requisito) y cada
//! valor lleva su **procedencia** (`Provenance`) para poder distinguir, tal y como
//! pide el encargo:
//!
//!   * `Oficial`      — dato de fuente oficial (Renfe/Adif/fabricante).
//!   * `Secundaria`   — dato técnico fiable pero no oficial (p.ej. Wikipedia con ficha).
//!   * `Estimacion`   — estimación a partir de otros datos.
//!   * `Suposicion`   — parámetro del modelo (no medido; asunción de ingeniería).
//!   * `NoDisponible` — no se ha encontrado el dato; NO se inventa.
//!
//! IMPORTANTE: no se inventa ningún dato. Donde no hay fuente, se marca como tal.
//! La serie 456 no se ha encontrado documentada en ninguna fuente accesible, así que
//! se declara `available = false` y no se le asignan cifras.

/// Nivel de confianza / origen de un dato concreto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    Oficial,
    Secundaria,
    /// Estimación a partir de otros datos (categoría de la taxonomía; hoy sin uso porque
    /// ningún dato del calculador se deriva por estimación — se prefiere marcar como
    /// oficial/secundaria/suposición o no disponible). Se conserva para futuras series.
    #[allow(dead_code)]
    Estimacion,
    Suposicion,
    NoDisponible,
}

impl Provenance {
    pub fn label(self) -> &'static str {
        match self {
            Provenance::Oficial => "Oficial",
            Provenance::Secundaria => "Secundaria (técnica)",
            Provenance::Estimacion => "Estimación",
            Provenance::Suposicion => "Suposición del modelo",
            Provenance::NoDisponible => "No disponible",
        }
    }
    /// Color CSS para la insignia de procedencia en la UI.
    pub fn css(self) -> &'static str {
        match self {
            Provenance::Oficial => "#1b7f3b",
            Provenance::Secundaria => "#1f6feb",
            Provenance::Estimacion => "#9a6700",
            Provenance::Suposicion => "#9a6700",
            Provenance::NoDisponible => "#c5221f",
        }
    }
    /// Rango de incertidumbre (mayor = peor); usado para el aviso global por serie.
    pub fn rank(self) -> u8 {
        match self {
            Provenance::Oficial => 0,
            Provenance::Secundaria => 1,
            Provenance::Estimacion => 2,
            Provenance::Suposicion => 3,
            Provenance::NoDisponible => 4,
        }
    }
}

/// Un valor numérico con su procedencia y la referencia (documento/URL) de la fuente.
#[derive(Debug, Clone)]
pub struct Sourced {
    pub value: f64,
    pub prov: Provenance,
    /// Referencia corta de la fuente (nombre exacto del documento/dataset o URL).
    pub source: &'static str,
}

impl Sourced {
    const fn new(value: f64, prov: Provenance, source: &'static str) -> Self {
        Sourced { value, prov, source }
    }
}

/// Ficha técnica de una serie de tren.
#[derive(Debug, Clone)]
pub struct TrainSeries {
    pub id: &'static str,
    pub name: &'static str,
    pub builder: &'static str,
    /// `false` cuando la serie no está documentada (no se le asignan cifras).
    pub available: bool,

    // --- Datos con fuente ---
    pub vmax_kmh: Sourced,
    pub power_kw: Sourced,
    pub mass_t: Sourced,
    /// Aceleración de arranque máxima (tope adhesión/confort). NO publicada por Renfe.
    pub accel_start: Sourced,
    /// Deceleración de servicio (freno normal). NO publicada por Renfe.
    pub decel_service: Sourced,
    /// Rendimiento de la transmisión (tracción → rueda). Parámetro del modelo.
    pub efficiency: Sourced,
    /// Factor de masa rotativa (inercia de ejes/motores). Parámetro del modelo.
    pub rotary_mass: Sourced,

    pub notes: &'static str,
}

// --------------------------------------------------------------------------
// Fuentes (referencias exactas)
// --------------------------------------------------------------------------
//
// [RENFE_CSV] Renfe — "informacion-trenes.csv" (data.renfe.com, dataset "Información
//             de trenes"), incluido en el repositorio en ./raw. Campos: VELOCIDAD
//             MAXIMA, POTENCIA TOTAL, MASA SIN CARGA.
// [WIKI_447]  Wikipedia (EN) "Renfe Class 447" — https://en.wikipedia.org/wiki/Renfe_Class_447
// [WIKI_490]  Wikipedia (EN) "Renfe Class 490" — https://en.wikipedia.org/wiki/Renfe_Class_490
// [MODELO]    Asunción de ingeniería del propio modelo de este calculador (ver README
//             / sección "Fuentes i metodologia"). No es un dato medido de la serie.

const RENFE_CSV: &str = "Renfe — informacion-trenes.csv (data.renfe.com)";
const WIKI_490: &str = "Wikipedia EN — Renfe Class 490";
const MODELO: &str = "Suposición del modelo (ver Metodología)";

// Parámetros del modelo comunes (mismos supuestos para todas las series porque
// Renfe/Adif no publican curvas de aceleración ni deceleración por serie).
const ACCEL_START_MODEL: Sourced = Sourced::new(1.0, Provenance::Suposicion, MODELO);
const DECEL_MODEL: Sourced = Sourced::new(0.9, Provenance::Suposicion, MODELO);
const EFF_MODEL: Sourced = Sourced::new(0.85, Provenance::Suposicion, MODELO);
const ROTARY_MODEL: Sourced = Sourced::new(0.10, Provenance::Suposicion, MODELO);

/// Devuelve la base de datos completa de series soportadas (orden estable).
pub fn all() -> Vec<TrainSeries> {
    vec![
        // ---- Serie 447 (Cercanías, workhorse) ----
        TrainSeries {
            id: "447",
            name: "UT 447 — Cercanías",
            builder: "CAF / Alstom / Siemens / ABB / Adtranz",
            available: true,
            vmax_kmh: Sourced::new(120.0, Provenance::Oficial, RENFE_CSV),
            power_kw: Sourced::new(2400.0, Provenance::Oficial, RENFE_CSV),
            mass_t: Sourced::new(162.5, Provenance::Oficial, RENFE_CSV),
            accel_start: ACCEL_START_MODEL,
            decel_service: DECEL_MODEL,
            efficiency: EFF_MODEL,
            rotary_mass: ROTARY_MODEL,
            notes: "Masa 162,5 t (Renfe CSV); Wikipedia da 157 t. Vmax 120 km/h. \
                    Aceleración/frenado NO publicados → modelados.",
        },
        // ---- Serie 450 (Cercanías, doble piso) ----
        TrainSeries {
            id: "450",
            name: "UT 450 — Cercanías (doble piso)",
            builder: "CAF / Alstom",
            available: true,
            vmax_kmh: Sourced::new(140.0, Provenance::Oficial, RENFE_CSV),
            power_kw: Sourced::new(2960.0, Provenance::Oficial, RENFE_CSV),
            mass_t: Sourced::new(350.8, Provenance::Oficial, RENFE_CSV),
            accel_start: ACCEL_START_MODEL,
            decel_service: DECEL_MODEL,
            efficiency: EFF_MODEL,
            rotary_mass: ROTARY_MODEL,
            notes: "Composición de doble piso (masa elevada). Vmax 140 km/h. \
                    Aceleración/frenado NO publicados → modelados.",
        },
        // ---- Serie 470 (Media Distancia; reconstrucción de la 440) ----
        TrainSeries {
            id: "470",
            name: "UT 470 — Media Distancia",
            builder: "Renfe (reconstrucción serie 440)",
            available: true,
            vmax_kmh: Sourced::new(140.0, Provenance::Oficial, RENFE_CSV),
            power_kw: Sourced::new(1160.0, Provenance::Oficial, RENFE_CSV),
            mass_t: Sourced::new(156.0, Provenance::Oficial, RENFE_CSV),
            accel_start: ACCEL_START_MODEL,
            decel_service: DECEL_MODEL,
            efficiency: EFF_MODEL,
            rotary_mass: ROTARY_MODEL,
            notes: "En el CSV como \"Media Distancia R-470\". Potencia baja (1160 kW) \
                    respecto a su masa. Aceleración/frenado NO publicados → modelados.",
        },
        // ---- Serie 490 (Alstom/Fiat; ex "pendular", hoy limitada) ----
        TrainSeries {
            id: "490",
            name: "UT 490 — Alstom/Fiat",
            builder: "Alstom / Fiat Ferroviaria",
            available: true,
            vmax_kmh: Sourced::new(160.0, Provenance::Secundaria, WIKI_490),
            power_kw: Sourced::new(2040.0, Provenance::Secundaria, WIKI_490),
            mass_t: Sourced::new(159.0, Provenance::Secundaria, WIKI_490),
            accel_start: ACCEL_START_MODEL,
            decel_service: DECEL_MODEL,
            efficiency: EFF_MODEL,
            rotary_mass: ROTARY_MODEL,
            notes: "Diseño original 220 km/h; limitada a 160 km/h desde 2022 (se usa 160). \
                    No aparece en el CSV de Renfe → datos de Wikipedia. \
                    Aceleración/frenado NO publicados → modelados.",
        },
        // ---- Serie 456 (NO DOCUMENTADA) ----
        TrainSeries {
            id: "456",
            name: "Serie 456 — no documentada",
            builder: "—",
            available: false,
            vmax_kmh: Sourced::new(0.0, Provenance::NoDisponible, "—"),
            power_kw: Sourced::new(0.0, Provenance::NoDisponible, "—"),
            mass_t: Sourced::new(0.0, Provenance::NoDisponible, "—"),
            accel_start: Sourced::new(0.0, Provenance::NoDisponible, "—"),
            decel_service: Sourced::new(0.0, Provenance::NoDisponible, "—"),
            efficiency: Sourced::new(0.0, Provenance::NoDisponible, "—"),
            rotary_mass: Sourced::new(0.0, Provenance::NoDisponible, "—"),
            notes: "No se ha encontrado ninguna serie 456 de Renfe en fuentes accesibles \
                    (la numeración salta 453→460; la 449 es la CAF de Media Distancia). \
                    NO se inventan datos: la serie se muestra como no disponible.",
        },
    ]
}

/// Busca una serie por su id ("447", "450", …).
pub fn get(id: &str) -> Option<TrainSeries> {
    all().into_iter().find(|s| s.id == id)
}
