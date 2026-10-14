//! Núcleo del **motor de simulación unificado** (Rodalies) — FASE B: velocidades por FÍSICA.
//!
//! Objetivo (ver `docs/PLAN-motor-unificado.md`): fuente única de verdad de la física, para **ver**
//! (frontend reproductor) y **simular en masa** (headless, `rayon`). `step(dt)` = sub-paso puro;
//! `observe()` = estado observable; el driver decide (lote = `run()`; ver = `step`+`observe`).
//!
//! **Modelo (Fase B):** cada tren se mueve por CINEMÁTICA real, no por horario. Su velocidad la fija
//! `min(Vmax del material, Vmax de vía ADIF/CVM, LTV, frenada anticipada)`; acelera a **¾ de la
//! aceleración máxima** (no todos van a tope) e intenta ir siempre a la máxima permitida. Ante una
//! señal en **ÁMBAR** reduce un 20% para poder detenerse si el siguiente sigue en **ROJO**. El
//! **horario deja de dictar la velocidad**: solo marca que un tren **nunca sale de una estación
//! antes de su hora programada** de salida, y el **dwell mínimo** por densidad. Si la física o la
//! congestión lo retrasan, el retraso se propaga. Los atascos emergen solos (un tren clavado en un
//! rojo retiene a los de detrás).
//!
//! **Señalización de BLOQUES (portada de la Fase 1b):** 1 tren/bloque, sub-bloques por distancia,
//! andén = cola del último sub-bloque del cantón de llegada, vía única = testigo canónico compartido
//! (liberado al parar en el apartadero) con prioridad de cruce Regional. El gating ahora se dispara
//! al **cruzar la frontera de POSICIÓN** del bloque, no al alcanzar un tiempo.
//!
//! **Datos reales reutilizados** (no se reinventa): `src/calculator/` (física `physics.rs`, material
//! `rolling_stock.rs`, perfil ADIF `infrastructure::adif_profile`, LTV `ltv.rs`) + `processed/adif/`.
//!
//! **PENDIENTE:** Fase A (incidencias probabilísticas), enclavamiento de estación (inerte en lote),
//! y la deuda de rendimiento de las claves `String` en caliente (internar `stop_id`→`u32`).
//! Regla de oro: **verificar SIEMPRE a ×25** en el frontend (aquí se verifica headless).

use crate::calculator::physics::{self, Restriction, SpeedZone, Vehicle};
use crate::calculator::{infrastructure, ltv, rolling_stock, vehicle_of};
use crate::gtfs_loader::Network;
use crate::{game, topology, track_geometry};
use rand::{rngs::StdRng, Rng, SeedableRng};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

// ---- Constantes de dominio ----------------------------------------------------------------

const SIGNAL_SPACING_KM: f64 = 1.6;
const MARGIN_KM: f64 = 0.5;
const DWELL_MIN: f64 = 20.0;
const DWELL_MAX: f64 = 60.0;
const NOMINAL_PAX: f64 = 300.0;
const VENTANA_PRIORIDAD_SEC: f64 = 8.0 * 60.0;
/// Fracción de la aceleración máxima que aplica el maquinista (no todos van a tope).
const ACCEL_FRACTION: f64 = 0.75;
/// Reducción de velocidad ante señal en ámbar (para poder frenar si el siguiente está en rojo).
const AMBER_FACTOR: f64 = 0.8;
/// Variabilidad de velocidad (conducción real): truncado de la gaussiana en desviaciones típicas.
/// Con σ = p/2 el truncado simétrico a ±p alrededor de la media cae en ±2σ (se conserva ~95 % de
/// la campana). El usuario solo fija `p`; este σ es una elección de modelado declarada, no un dato.
const SPEED_VAR_TRUNC_Z: f64 = 2.0;
/// Velocidad de REFERENCIA (km/h) a la que se define el factor `p` del usuario (Vmax de la mayoría
/// del material de Rodalies: 447 / 465). A otra Vmax el factor escala LINEALMENTE: p(V) = p·V/120.
/// Motivo: a baja velocidad (agujas, curvas, LTV, entrada a estación) el maquinista clava el límite
/// —es fácil y el ASFA vigila—; a alta velocidad es más probable que lleve un margen por debajo.
const SPEED_VAR_REF_KMH: f64 = 120.0;
/// Tope de seguridad del factor efectivo `p(V)` (un 25 % implicaría circular hasta a la mitad de Vmax).
pub const SPEED_VAR_MAX: f64 = 0.25;
/// Sal del flujo aleatorio de velocidades: independiente del de incidencias, así activar/desactivar
/// la variabilidad NO cambia qué incidencias rueda una semilla.
const SPEED_VAR_SALT: u64 = 0x5EED_0F_5EED;

/// Vmax de vía por defecto donde ADIF no da cobertura (km/h → m/s), para no dejar el cantón libre.
const DEFAULT_TRACK_VMAX_MS: f64 = 100.0 / 3.6;
/// Paso de integración física acotado (s) dentro de cada `step` del driver.
const PHYS_SUBSTEP_S: f64 = 1.0;
/// Paso/tolerancia de muestreo del perfil ADIF por cantón (mismos que el calculador web).
const ADIF_STEP_M: f64 = 40.0;
const ADIF_TOL_M: f64 = 140.0;
/// Cadencia de muestreo de la línea temporal de retraso de red (s de tiempo simulado).
const SAMPLE_STEP_S: f64 = 60.0;
/// Umbral de equilibrio: la red se considera recuperada cuando el retraso ACUMULADO cae por debajo
/// (proxy de "vuelta a horario"; el pico suele ser 1-2 órdenes de magnitud mayor).
const EQUILIBRIUM_S: f64 = 60.0;

const PERFIL: &[(f64, f64)] = &[
    (5.0, 0.05), (6.0, 0.30), (7.0, 0.90), (8.0, 1.00), (9.0, 0.80), (9.5, 0.40),
    (12.0, 0.35), (15.0, 0.35), (17.0, 0.60), (18.0, 0.90), (18.5, 1.00), (19.5, 0.70),
    (21.0, 0.30), (22.5, 0.15), (24.0, 0.03),
];

fn demanda(t: f64) -> f64 {
    let h = t / 3600.0;
    if h <= PERFIL[0].0 {
        return PERFIL[0].1;
    }
    for i in 1..PERFIL.len() {
        if h <= PERFIL[i].0 {
            let (h0, v0) = PERFIL[i - 1];
            let (h1, v1) = PERFIL[i];
            return v0 + (v1 - v0) * (h - h0) / (h1 - h0);
        }
    }
    0.03
}

/// Distancia Haversine entre coords (km).
#[cfg(test)]
fn dist_km(a: (f64, f64), b: (f64, f64)) -> f64 {
    infrastructure::haversine_m(a.0, a.1, b.0, b.1) / 1000.0
}

/// Longitud de VÍA del cantón a→b (m): la del trazado real (OSM) si existe; si no, la recta.
fn canton_len_m(a: &str, b: &str, ca: (f64, f64), cb: (f64, f64)) -> f64 {
    track_geometry::global()
        .and_then(|g| g.len_m(a, b))
        .unwrap_or_else(|| infrastructure::haversine_m(ca.0, ca.1, cb.0, cb.1))
        .max(1.0)
}

/// Nº de sub-bloques del cantón a→b (≥1) por la distancia en línea recta (tests / sin trazado).
#[cfg(test)]
fn calc_sub_blocks(a: (f64, f64), b: (f64, f64), unica: bool) -> usize {
    calc_sub_blocks_len(dist_km(a, b), unica)
}

/// Nº de sub-bloques de un cantón de `d` km de VÍA (≥1). 1 en vía única; en doble = señales por
/// distancia + 1 (la densidad la fija la longitud real de vía, no la recta entre estaciones).
fn calc_sub_blocks_len(d: f64, unica: bool) -> usize {
    if unica {
        return 1;
    }
    let max_sig = if d > 7.0 { 5 } else { 3 };
    let signals = ((d / SIGNAL_SPACING_KM).round() as i64).clamp(0, max_sig);
    let mut m = (signals + 1) as usize;
    while m > 1 && d / (m as f64) < MARGIN_KM {
        m -= 1;
    }
    m
}

/// Serie de tren asignada a una línea (petición del usuario): R2→450; regional (no R11/R3)→470;
/// R1/R3/R4 (metropolitanas de alta frecuencia)→465 (Civia); resto→447 (por defecto).
fn fleet_for_line(line: &str) -> &'static str {
    if matches!(line, "R2" | "R2N" | "R2S") {
        return "450";
    }
    if topology::is_regional_line(line) && line != "R11" && line != "R3" {
        return "470";
    }
    if matches!(line, "R1" | "R3" | "R4") {
        return "465";
    }
    "447"
}

/// `Vehicle` (SI) del material asignado a la línea (con respaldo a la 447 si algo faltara).
fn vehicle_for_line(line: &str) -> Vehicle {
    let series = rolling_stock::get(fleet_for_line(line))
        .filter(|s| s.available)
        .or_else(|| rolling_stock::get("447"))
        .expect("serie 447 siempre disponible");
    vehicle_of(&series)
}

// ---- Clave canónica de cantón (solo en construcción, para casar la vía única de topología) -----

fn canon(a: &str, b: &str) -> String {
    if a < b { format!("{a}|{b}") } else { format!("{b}|{a}") }
}

// ---- Estado -------------------------------------------------------------------------------

/// Una parada del recorrido (tras la descomposición atómica de `game::build_schedule`).
struct Pt {
    id: String,
    a: f64, // llegada programada (s desde medianoche) — objetivo, no impone velocidad
    d: f64, // salida programada — un tren NUNCA sale antes de esta hora
    lat: f64,
    lon: f64,
    pass: bool, // punto de paso sintético (exprés): no para
}

/// Un BLOQUE de vía de la secuencia de un tren, situado por POSICIÓN de ruta (`x_enter`, m).
///
/// **Rendimiento (deuda saldada):** los recursos de ocupación se precalculan a índices `u32` en
/// construcción — el bucle caliente NO construye claves `String` por tick. `res` = recurso por
/// defecto (sub-bloque dirigido, o canónico si el cantón es vía única ESTÁTICA); `res_single` =
/// recurso canónico del cantón (el que se usa si una incidencia lo pone en vía única en caliente).
#[derive(Clone)]
struct Block {
    origin: bool,
    from: u32, // nid de la estación de origen del bloque
    to: u32,   // nid de la estación destino del bloque
    sb: usize,
    last: bool,
    dest: u32, // nid del destino del cantón (= to)
    arr_idx: usize,
    x_enter: f64,       // posición de ruta (m) de la señal de entrada del bloque
    res: u32,           // recurso de ocupación por defecto (índice en `occ`)
    res_single: u32,    // recurso canónico del cantón (vía única estática o por incidencia)
    stat_single: bool,  // el cantón es vía única ESTÁTICA (topología)
    cap_double: u32,    // capacidad cuando NO es vía única (platCap en el último sub-bloque; si no, 1)
    plat_tracks: u32,   // último sub-bloque: vías principales de andén de este acceso (1 = sin andén aparte)
}

/// Fase del tren en su recorrido.
#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Dwell,   // parado en `station_idx`: acumulando dwell / esperando su hora / esperando salida
    Running, // en marcha
    Done,    // ha finalizado
}

/// Estado de un tren.
struct Train {
    id: String,
    line: String,
    regional: bool,
    veh: Vehicle,
    pts: Vec<Pt>,
    cum_l: Vec<f64>,             // distancia de ruta acumulada en cada parada (m); len = pts.len()
    canton_zones: Vec<Vec<SpeedZone>>, // perfil de velocidad por cantón i (m/s)
    speed_p0: f64,               // factor de variabilidad a 120 km/h (0 = apagada, todos a Vmax)
    speed_z: Vec<f64>,           // sorteo N(0,1) truncado a ±2 por BLOQUE de `seq` (tramo entre señales)
    dwell_min: Vec<f64>,         // parada mínima por estación (s); 0 en puntos de paso
    seq: Vec<Block>,
    // física:
    phase: Phase,
    station_idx: usize, // estación actual (Dwell) o de partida del cantón en curso
    canton_idx: usize,  // cantón que recorre (índice de pts) cuando va en marcha
    target_stop: usize, // próxima parada REAL (pts index)
    arrived_at: f64,    // clock de llegada a la estación actual (para el dwell)
    s: f64,             // posición de ruta (m)
    v: f64,             // velocidad (m/s)
    cb: usize,          // bloque comprometido
    delay_s: f64,
    waiting: bool,      // retenido por señal/ocupación (no por dwell ni por su hora)
    held_since: f64,    // instante en que empezó la retención actual (NaN = no retenido)
    was_waiting: bool,  // estado `waiting` del step anterior (para contar conflictos distintos)
}

impl Train {
    /// Muestra observable para el trace: `(from_id, to_id, frac, v)`. En marcha = cantón actual +
    /// fracción; parado en una estación intermedia = cantón de LLEGADA con frac 1.0 (así ocupa el
    /// andén como en el motor). `None` si aún no ha salido de cabecera o ya ha finalizado.
    fn trace_sample(&self) -> Option<(&str, &str, f64, f64)> {
        match self.phase {
            Phase::Running => {
                let i = self.canton_idx;
                if i + 1 >= self.pts.len() {
                    return None;
                }
                let len = (self.cum_l[i + 1] - self.cum_l[i]).max(1.0);
                let frac = ((self.s - self.cum_l[i]) / len).clamp(0.0, 1.0);
                Some((&self.pts[i].id, &self.pts[i + 1].id, frac, self.v))
            }
            Phase::Dwell if self.station_idx > 0 => {
                let k = self.station_idx;
                Some((&self.pts[k - 1].id, &self.pts[k].id, 1.0, 0.0))
            }
            _ => None,
        }
    }

    /// ¿Está EN SERVICIO (ya ha salido de cabecera, o la hora de salida ya pasó y está retenido
    /// allí acumulando retraso)? Los trenes que aún esperan su hora en cabecera NO cuentan: si
    /// entraran en el denominador del retraso medio, la media se diluiría con trenes que ni existen
    /// todavía para el viajero.
    fn in_service(&self) -> bool {
        match self.phase {
            Phase::Running => true,
            Phase::Dwell => self.station_idx > 0 || self.delay_s > 0.0,
            Phase::Done => false,
        }
    }
}

/// Infraestructura precalculada UNA vez (fuera del bucle caliente). Todos los índices son `u32`
/// (stop_id internado a `nid`; recursos de ocupación a `res`), así el bucle caliente no hashea
/// cadenas. Los mapas `HashMap` de aquí solo los usa la ruta FRÍA (resolver incidencias, visor).
struct Infra {
    stop_index: HashMap<String, u32>,          // stop_id → nid (resolver incidencias)
    sub_blocks: HashMap<(u32, u32), u32>,      // (a_nid,b_nid) → nº sub-bloques
    single_track: HashSet<u32>,                // recursos canónicos que son vía única ESTÁTICA
    terminals: HashSet<u32>,                   // nids terminales
    edges_from: HashMap<u32, Vec<u32>>,        // a_nid → [b_nid] (visor: salida_ocupada)
    single_track_users: HashMap<u32, Vec<(usize, usize)>>, // recurso canónico → (tren, parada)
    res_dir: HashMap<(u32, u32, u32), u32>,    // (a,b,sb) → recurso dirigido (build/visor)
    res_canon: HashMap<(u32, u32), u32>,       // (min,max) → recurso canónico (build/visor)
    n_res: usize,
}

impl Infra {
    fn nid(&self, s: &str) -> Option<u32> {
        self.stop_index.get(s).copied()
    }
    fn canon_res(&self, a: u32, b: u32) -> Option<u32> {
        let k = if a < b { (a, b) } else { (b, a) };
        self.res_canon.get(&k).copied()
    }
    fn sub_m_id(&self, a: u32, b: u32) -> u32 {
        self.sub_blocks.get(&(a, b)).copied().unwrap_or(1)
    }
    /// Recurso dirigido (a,b,sb) para la ruta FRÍA (visor). Cae al canónico si no hay dirigido.
    fn dir_res(&self, a: u32, b: u32, sb: u32) -> Option<u32> {
        self.res_dir.get(&(a, b, sb)).copied().or_else(|| self.canon_res(a, b))
    }
}

// ---- Incidencias (Fase A) -----------------------------------------------------------------

/// Efecto físico de una incidencia. Toda incidencia se reduce a dos tipos (petición del usuario):
/// una señal en ROJO sin tren delante (en un sentido o en ambos), o un tramo de vía doble que pasa
/// a comportarse como VÍA ÚNICA.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum IncidentEffect {
    /// Señal de entrada del cantón en rojo. `both` = ambos sentidos (si no, solo el elegido).
    RedSignal { both: bool },
    /// El cantón (doble) pasa a vía única (testigo compartido) mientras dura la incidencia.
    SingleTrack,
}

/// Plantilla de incidencia definida por el usuario (creador del menú principal). Tasa: probabilidad
/// `prob` de que ocurra **cada `every_h` horas**. Antes del día se trocea la jornada en secciones de
/// `every_h` h y se tiran los dados en CADA sección → la misma plantilla puede aparecer varias veces
/// en un día sin tener que poner 100 %/día (proceso de Bernoulli por intervalo ≈ tasa constante).
#[derive(Clone)]
pub struct IncidentTemplate {
    pub motivo: String,
    pub effect: IncidentEffect,
    pub dur_min: f64,
    pub prob: f64,    // 0..1: probabilidad de aparecer en una sección de `every_h` horas
    pub every_h: f64, // longitud de la sección (h); 24 ≈ la antigua "probabilidad diaria"
    pub enabled: bool,
}

/// Sección mínima (h): evita trocear la jornada en miles de tiradas por un valor absurdo.
pub const MIN_EVERY_H: f64 = 0.25;

/// Una incidencia CONCRETA ya ubicada y fechada tras la tirada (lo que aplica el motor).
#[derive(Clone)]
pub struct ActiveIncident {
    pub from: String,
    pub to: String,
    pub effect: IncidentEffect,
    pub t0: f64,
    pub t1: f64,
    pub motivo: String,
}

/// Plantillas por defecto (razonables, ELIMINABLES desde el frontend). Cubren los dos tipos.
pub fn default_templates() -> Vec<IncidentTemplate> {
    vec![
        IncidentTemplate {
            motivo: "Avería de señalización".into(),
            effect: IncidentEffect::RedSignal { both: false },
            dur_min: 15.0,
            prob: 0.40,
            every_h: 24.0,
            enabled: true,
        },
        IncidentTemplate {
            motivo: "Persona en la vía".into(),
            effect: IncidentEffect::RedSignal { both: true },
            dur_min: 20.0,
            prob: 0.20,
            every_h: 24.0,
            enabled: true,
        },
        IncidentTemplate {
            motivo: "Obras: vía única provisional".into(),
            effect: IncidentEffect::SingleTrack,
            dur_min: 45.0,
            prob: 0.30,
            every_h: 24.0,
            enabled: true,
        },
        IncidentTemplate {
            motivo: "Falta de tensión en catenaria".into(),
            effect: IncidentEffect::RedSignal { both: true },
            dur_min: 30.0,
            prob: 0.15,
            every_h: 24.0,
            enabled: true,
        },
    ]
}

/// Muestra un instante del día (s) según la densidad temporal: más probable en hora punta (usa el
/// mismo `PERFIL` de demanda). Discretiza en bins y muestrea por peso.
fn sample_time(rng: &mut StdRng, start: f64, end: f64) -> f64 {
    let bins = 60usize;
    let w = (end - start) / bins as f64;
    let weights: Vec<f64> = (0..bins).map(|i| demanda(start + (i as f64 + 0.5) * w)).collect();
    let tot: f64 = weights.iter().sum::<f64>().max(1e-9);
    let mut r = rng.gen::<f64>() * tot;
    let mut bi = 0;
    for (i, &ww) in weights.iter().enumerate() {
        if r < ww {
            bi = i;
            break;
        }
        r -= ww;
    }
    start + (bi as f64 + rng.gen::<f64>()) * w
}

/// Muestra un cantón según los pesos espaciales (∝ nudos/BCN, estaciones grandes, tramos
/// frecuentados, terminales). `cantons` = (from, to, peso). Determinista con el `rng` sembrado.
fn sample_canton<'a>(rng: &mut StdRng, cantons: &'a [(String, String, f64)]) -> Option<&'a (String, String, f64)> {
    let tot: f64 = cantons.iter().map(|c| c.2).sum::<f64>();
    if tot <= 0.0 {
        return cantons.first();
    }
    let mut r = rng.gen::<f64>() * tot;
    for c in cantons {
        if r < c.2 {
            return Some(c);
        }
        r -= c.2;
    }
    cantons.last()
}

/// Nº de secciones ENTERAS en que se trocea la jornada para una plantilla "cada X horas": el entero
/// más cercano a `jornada / X` (mínimo 1). Secciones IGUALES (≈ X h) sin restos: así "cada 24 h"
/// en una jornada de ~19,6 h es exactamente UNA tirada = la antigua probabilidad diaria.
pub fn n_sections(day_s: f64, every_h: f64) -> usize {
    let x = every_h.max(MIN_EVERY_H) * 3600.0;
    ((day_s / x).round() as usize).max(1)
}

/// Tira los dados ANTES del día. Para cada plantilla activa, la jornada `[start,end)` se trocea en
/// `n_sections` secciones iguales y en CADA una se tira Bernoulli(`prob`); si sale, se ubica
/// (∝ pesos espaciales) y se fecha DENTRO de esa sección (∝ demanda horaria) una instancia. Media
/// de instancias/día = `prob × n`. Sin máximo. Determinista con `seed`.
fn roll_incidents(
    cantons: &[(String, String, f64)],
    templates: &[IncidentTemplate],
    start: f64,
    end: f64,
    seed: u64,
) -> Vec<ActiveIncident> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut out = Vec::new();
    for tpl in templates.iter().filter(|t| t.enabled) {
        let n = n_sections(end - start, tpl.every_h);
        let sec = (end - start) / n as f64;
        for k in 0..n {
            let s0 = start + k as f64 * sec;
            let s1 = if k + 1 == n { end } else { s0 + sec };
            if rng.gen::<f64>() < tpl.prob.clamp(0.0, 1.0) {
                if let Some((from, to, _)) = sample_canton(&mut rng, cantons) {
                    let t0 = sample_time(&mut rng, s0, s1);
                    out.push(ActiveIncident {
                        from: from.clone(),
                        to: to.clone(),
                        effect: tpl.effect,
                        t0,
                        t1: t0 + tpl.dur_min * 60.0,
                        motivo: tpl.motivo.clone(),
                    });
                }
            }
        }
    }
    out.sort_by(|a, b| a.t0.total_cmp(&b.t0));
    out
}

/// Efectos vigentes en el instante `now`, ya resueltos a índices `u32` (el bucle caliente no
/// hashea cadenas): señales en rojo por incidencia (pares de nid dirigidos `(a,b)`) y cantones que
/// pasan a vía única por incidencia (recursos canónicos). Las incidencias son poquísimas → resolver
/// sus stop_id → nid/recurso por `step` es despreciable.
fn dyn_effects(infra: &Infra, incidents: &[ActiveIncident], now: f64) -> (HashSet<(u32, u32)>, HashSet<u32>) {
    let mut red = HashSet::new();
    let mut single = HashSet::new();
    for inc in incidents {
        if now < inc.t0 || now >= inc.t1 {
            continue;
        }
        let (Some(fa), Some(tb)) = (infra.nid(&inc.from), infra.nid(&inc.to)) else { continue };
        match inc.effect {
            IncidentEffect::RedSignal { both } => {
                red.insert((fa, tb));
                if both {
                    red.insert((tb, fa));
                }
            }
            IncidentEffect::SingleTrack => {
                if let Some(c) = infra.canon_res(fa, tb) {
                    single.insert(c);
                }
            }
        }
    }
    (red, single)
}

/// Efectos dinámicos vigentes en un instante (recalculados por `step`), en índices `u32`.
struct Dyn<'a> {
    red: &'a HashSet<(u32, u32)>,
    single: &'a HashSet<u32>,
}
impl Dyn<'_> {
    /// ¿El cantón del bloque es vía única AHORA? (estática de topología o por incidencia vigente).
    #[inline]
    fn block_single(&self, b: &Block) -> bool {
        b.stat_single || self.single.contains(&b.res_single)
    }
    /// Recurso de ocupación del bloque AHORA (canónico si es vía única; si no, el dirigido).
    #[inline]
    fn block_res(&self, b: &Block) -> u32 {
        if self.block_single(b) {
            b.res_single
        } else {
            b.res
        }
    }
}

// ---- Configuración ------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub struct EngineCfg {
    pub start_sec: u32,
    pub end_sec: u32,
    pub dt_sec: f64,
    /// Semilla del Monte-Carlo (tirada de incidencias reproducible).
    pub seed: u64,
    /// Variabilidad de velocidad: `Some(p)` (p. ej. 0.03 = 3 %, definido A 120 km/h) hace que cada
    /// tren, en cada tramo entre señales, circule a `Vmax × f` con `f ~ N(1−p(V), (p(V)/2)²)` truncada
    /// a `[1−2p(V), 1]` y `p(V) = p·Vmax/120` (ver `speed_factor`). `None` = siempre a Vmax.
    pub speed_var: Option<f64>,
    /// MODO SIN HORARIOS («caos», experimentos de capacidad): cada tren conserva SOLO su hora de
    /// salida de cabecera; en el resto de paradas sale en cuanto cumple la parada mínima
    /// reglamentaria, y la vía única deja de reservar el paso a regionales según su horario (las
    /// señales solo miran si hay un tren delante). El retraso se sigue midiendo contra el horario.
    pub free_run: bool,
}
impl EngineCfg {
    /// Jornada completa. `start_sec` (05:00) es solo un FALLBACK: `Engine::new` lo sobrescribe con la
    /// salida del primer tren por horario (evita el artefacto de "nacer tarde"). `end_sec` = 24:00.
    pub fn day() -> Self {
        EngineCfg { start_sec: 5 * 3600, end_sec: 24 * 3600, dt_sec: 2.0, seed: 1, speed_var: None, free_run: false }
    }
}

#[derive(Clone, Copy)]
pub struct RunMetrics {
    pub n_trains: usize,
    pub sim_seconds: u32,
    pub train_min_delay: f64,
    pub max_delay_s: f64,
    pub peak_waiting: usize,
    pub final_waiting: usize,
    pub pax_min: f64,
    /// Nº de conflictos DISTINTOS (un tren que entra en retención por señal/ocupación cuenta 1).
    pub held_events: u64,
    /// Pico del retraso ACUMULADO de la red (suma de retrasos, s) y su instante.
    pub peak_total_delay_s: f64,
    pub peak_time: u32,
    /// Instante en que la red vuelve al equilibrio tras el pico (`None` si no recupera en la ventana).
    pub recovery_time: Option<u32>,
}

/// Foto observable de un tren en marcha (para el reproductor del frontend).
pub struct FrameTrain {
    pub id: String,
    pub line: String,
    pub from: String,
    pub to: String,
    pub frac: f64,
    pub lat: f64,
    pub lon: f64,
    pub speed_kmh: f64,
    pub delay_s: f64,
    pub waiting: bool,
    pub aspect: u8, // 0=verde, 1=ámbar, 2=rojo
}

/// Trayectoria de un tren para el REPRODUCTOR del frontend: arrays paralelos muestreados cada `dt`
/// desde `t0`. `a`/`b` = índices en la tabla `ids` del cantón (o del cantón de llegada si está
/// parado); `f` = fracción·1000 (0..1000; 1000 = parado en `b`); `v` = velocidad km/h; `d` =
/// retraso REAL del tren en ese instante (segundos, respecto a su llegada programada — el mismo
/// `delay_s` que el motor calcula por tren). Compacto (enteros) para que la jornada entera quepa
/// en un JSON razonable. `d` permite al visor mostrar el retraso honesto por tren y calcular el
/// KPI de pax·min con la MISMA fórmula que el motor (sin el proxy de "trenes retenidos").
#[derive(Serialize)]
pub struct TraceTrain {
    pub id: String,
    pub line: String,
    pub t0: f64,
    pub a: Vec<u32>,
    pub b: Vec<u32>,
    pub f: Vec<u16>,
    pub v: Vec<u16>,
    pub d: Vec<u16>,
}

/// Una LLEGADA TARDE de un tren a una estación (registro de CTC). `t` = instante de la llegada
/// (s desde medianoche); `stop` = stop_id (el frontend resuelve el nombre); `delay` = retraso en la
/// llegada respecto a la hora prevista GTFS (s). Solo se registran las llegadas de ≥ `LATE_LOG_THRESHOLD_S`.
#[derive(Serialize, Clone)]
pub struct LateArrival {
    pub t: u32,
    pub train: String,
    pub line: String,
    pub stop: String,
    pub delay: u16,
}

/// Trace completo de una jornada (lo que reproduce el frontend visual: velocidad variable REAL).
/// `arrivals` = todas las llegadas tarde a estación de la jornada (orden temporal), para el contador
/// de minutos de retraso y el registro de CTC del visor.
#[derive(Serialize)]
pub struct Trace {
    pub start: f64,
    pub end: f64,
    pub dt: f64,
    pub ids: Vec<String>,
    pub trains: Vec<TraceTrain>,
    pub arrivals: Vec<LateArrival>,
}

/// Tiempo de RETENCIÓN acumulado ante la señal de entrada de un tramo (cantón dirigido): cuánto
/// tráfico ha frenado ese tramo en la jornada. Solo se calcula en el modo VISUAL (`run_trace`): en
/// las runs headless sería trabajo extra que nadie lee.
#[derive(Default, Clone)]
struct HoldAcc {
    secs: f64,                     // tren·segundos retenidos esperando entrar al tramo
    events: u32,                   // retenciones DISTINTAS (flanco de subida de `waiting`)
    max_s: f64,                    // retención continua más larga de un solo tren (s)
    by_line: HashMap<String, f64>, // tren·segundos por línea (qué servicios sufren el tramo)
}

/// Un tramo del ranking de los más problemáticos (ver [`Engine::hold_ranking`]).
pub struct HoldSpot {
    pub from: String, // stop_id de la señal de entrada (inicio del cantón)
    pub to: String,   // stop_id del final del cantón
    pub held_s: f64,
    pub events: u32,
    pub max_s: f64,
    /// (línea, tren·segundos retenidos) de mayor a menor.
    pub lines: Vec<(String, f64)>,
}

pub struct Engine {
    cfg: EngineCfg,
    /// Retención acumulada por tramo `(from_nid, to_nid)`; `None` = no se mide (runs headless).
    holds: Option<HashMap<(u32, u32), HoldAcc>>,
    clock: f64,
    trains: Vec<Train>,
    infra: Infra,
    occ: Vec<Vec<usize>>, // ocupación densa por recurso (índice = res id); se reutiliza entre steps
    incidents: Vec<ActiveIncident>,
    train_min_delay: f64,
    max_delay: f64,
    peak_waiting: usize,
    waiting_now: usize,
    pax_min: f64,
    held_events: u64,
    /// Línea temporal del retraso acumulado de la red: `(instante_s, retraso_total_s)`, muestreada
    /// cada `SAMPLE_STEP_S` de tiempo simulado. Base del PAX-weighted delay, pico y recuperación.
    timeline: Vec<(u32, f64)>,
    /// Trenes EN SERVICIO en cada muestra de `timeline` (mismo índice): denominador del retraso
    /// medio por tren del seguimiento de retrasos.
    timeline_active: Vec<u32>,
    next_sample: f64,
    /// Registro de todas las llegadas tarde a estación (≥ `LATE_LOG_THRESHOLD_S`) de la jornada,
    /// en orden temporal. Alimenta el contador de minutos de retraso y el log de CTC del visor.
    late_arrivals: Vec<LateArrival>,
}

/// Umbral para registrar una llegada como "tarde": un tren cuenta como retrasado en una estación si
/// llega ≥ 1 min por detrás de su hora prevista GTFS (evita el ruido sub-minuto de la física).
const LATE_LOG_THRESHOLD_S: f64 = 60.0;
/// A partir de cuánto tiempo (s) RETENIDO por una señal (en plena vía o en el andén con la salida en
/// rojo) el retraso del tren se recalcula en vivo respecto al horario, sin esperar a llegar a la
/// siguiente estación. Sin esto, una incidencia larga no se veía en las gráficas hasta que se
/// levantaba y los trenes volvían a pasar por estaciones: el retraso aparecía de golpe, tarde.
const HOLD_LIVE_DELAY_S: f64 = 60.0;

/// Retraso (s) de un tren en la posición de ruta `s` (m) del cantón `i` (pts[i]→pts[i+1]) en el
/// instante `now`, respecto al horario: la hora a la que DEBÍA pasar por ese punto se interpola
/// linealmente entre la salida programada de pts[i] y la llegada programada a pts[i+1] según la
/// fracción de vía recorrida. Equivale a la previsión de retraso en la siguiente estación si el
/// tren reanuda la marcha al ritmo del horario (ahora + tiempo restante − llegada programada).
fn retraso_en_posicion(pts: &[Pt], cum_l: &[f64], i: usize, s: f64, now: f64) -> f64 {
    let (a, b) = (&pts[i], &pts[i + 1]);
    let len = (cum_l[i + 1] - cum_l[i]).max(1.0);
    let f = ((s - cum_l[i]) / len).clamp(0.0, 1.0);
    now - (a.d + (b.a - a.d).max(0.0) * f)
}

impl Engine {
    /// Construye el motor desde la red. Consume el MISMO horario descompuesto que la API del juego
    /// (`game::build_schedule`), asigna material por línea, y precalcula el perfil de velocidad de
    /// vía de cada cantón (ADIF/CVM + LTV) — todo una sola vez. `adif`/`ltv` = datos ya cargados
    /// (main/server); si faltan, cae a un Vmax de vía por defecto documentado.
    pub fn new(
        net: &Network,
        cfg: EngineCfg,
        adif: Option<&infrastructure::AdifNet>,
        ltv: Option<&ltv::LtvSet>,
        templates: &[IncidentTemplate],
        offsets: Option<&HashMap<String, i64>>,
        line_filter: Option<&str>,
    ) -> Self {
        let game_trains = game::build_schedule(net, false, line_filter);
        Self::with_schedule(net, cfg, adif, ltv, templates, offsets, &game_trains)
    }

    /// Como [`Engine::new`] pero sobre un horario YA construido (p. ej. una malla CADENCIADA del
    /// creador de horarios, `cadence::generate`), con la misma descomposición en cantones atómicos.
    pub fn with_schedule(
        net: &Network,
        mut cfg: EngineCfg,
        adif: Option<&infrastructure::AdifNet>,
        ltv: Option<&ltv::LtvSet>,
        templates: &[IncidentTemplate],
        offsets: Option<&HashMap<String, i64>>,
        game_trains: &[game::GameTrain],
    ) -> Self {
        let coord = |id: &str| net.coord(id); // estaciones y bifurcaciones

        // Vía única (canónica por stop_id) — necesaria para sub-bloques y para el testigo.
        let mut single_track_str: HashSet<String> = HashSet::new();
        for (a, b) in topology::single_track_pairs(net) {
            let (sa, sb) = (&net.graph[a].stop_id, &net.graph[b].stop_id);
            single_track_str.insert(canon(sa, sb));
            // Si el cantón pasa por bifurcaciones, sus trozos A→J…→B heredan la vía única.
            if let Some(g) = track_geometry::global() {
                let (from, to, v) = if g.via(sa, sb).is_empty() { (sb, sa, g.via(sb, sa)) } else { (sa, sb, g.via(sa, sb)) };
                if !v.is_empty() {
                    let mut prev = from.as_str();
                    for (j, _) in v {
                        single_track_str.insert(canon(prev, j));
                        prev = j;
                    }
                    single_track_str.insert(canon(prev, to));
                }
            }
        }

        // Cache de perfil de velocidad por cantón dirigido (una vez; no se recalcula por tren).
        let mut zone_cache: HashMap<(String, String), Vec<SpeedZone>> = HashMap::new();
        // Trazado real de vía (OSM) si existe; si no, la recta entre estaciones.
        let geom = track_geometry::global();
        let mut zones_for = |a: &str, b: &str, ca: (f64, f64), cb: (f64, f64)| -> Vec<SpeedZone> {
            let k = (a.to_string(), b.to_string());
            if let Some(z) = zone_cache.get(&k) {
                return z.clone();
            }
            // El perfil de Vmax se muestrea SOBRE la curva real: la recta puede pasar por encima de
            // otras vías (y tomar su Vmax) o no tocar ninguna.
            let (poly, length, ltv_tol) = match geom.and_then(|g| g.get(a, b)) {
                Some(g) => (g.pts, g.len_m.max(1.0), infrastructure::LTV_TOL_TRACK_M),
                None => (vec![ca, cb], infrastructure::haversine_m(ca.0, ca.1, cb.0, cb.1).max(1.0), infrastructure::LTV_TOL_CHORD_M),
            };
            let zones = adif
                .and_then(|net_adif| {
                    infrastructure::adif_profile_ltv_tol(net_adif, ltv, &poly, ADIF_STEP_M, ADIF_TOL_M, ltv_tol)
                })
                .map(|p| {
                    p.zones
                        .iter()
                        .map(|z| SpeedZone { from: z.from_m, to: z.to_m, vmax: z.vmax_kmh / 3.6 })
                        .collect::<Vec<_>>()
                })
                .filter(|z| !z.is_empty())
                .unwrap_or_else(|| vec![SpeedZone { from: 0.0, to: length, vmax: DEFAULT_TRACK_VMAX_MS }]);
            zone_cache.insert(k, zones.clone());
            zones
        };

        // Construir trenes (paradas con coordenadas, como el juego).
        let mut trains: Vec<Train> = Vec::with_capacity(game_trains.len());
        for gt in game_trains {
            // Desfase de fase de la línea (optimizador): desplaza TODA la marcha del tren un mismo
            // Δt → cambia solo la fase relativa entre líneas, no la estructura del recorrido.
            let off = offsets.and_then(|m| m.get(&gt.line)).copied().unwrap_or(0);
            let shift = |t: u32| -> f64 { (t as i64 + off).max(0) as f64 };
            let mut pts = Vec::with_capacity(gt.stops.len());
            for s in &gt.stops {
                let Some((lat, lon)) = coord(&s.s) else { continue };
                pts.push(Pt { id: s.s.clone(), a: shift(s.a), d: shift(s.d), lat, lon, pass: s.pass });
            }
            if pts.len() < 2 {
                continue;
            }
            // Distancias de ruta acumuladas y perfil de velocidad por cantón.
            let mut cum_l = Vec::with_capacity(pts.len());
            cum_l.push(0.0);
            let mut canton_zones: Vec<Vec<SpeedZone>> = Vec::with_capacity(pts.len() - 1);
            for w in pts.windows(2) {
                let ca = (w[0].lat, w[0].lon);
                let cb = (w[1].lat, w[1].lon);
                let len = canton_len_m(&w[0].id, &w[1].id, ca, cb);
                cum_l.push(cum_l.last().unwrap() + len);
                canton_zones.push(zones_for(&w[0].id, &w[1].id, ca, cb));
            }
            trains.push(Train {
                id: gt.train.clone(),
                line: gt.line.clone(),
                regional: topology::is_regional_line(&gt.line),
                veh: vehicle_for_line(&gt.line),
                pts,
                cum_l,
                speed_p0: 0.0,
                speed_z: Vec::new(),
                canton_zones,
                dwell_min: Vec::new(),
                seq: Vec::new(),
                phase: Phase::Dwell,
                station_idx: 0,
                canton_idx: 0,
                target_stop: 0,
                arrived_at: cfg.start_sec as f64,
                s: 0.0,
                v: 0.0,
                cb: 0,
                delay_s: 0.0,
                waiting: false,
                was_waiting: false,
                held_since: f64::NAN,
            });
        }

        // Inicio de la jornada = salida del PRIMER tren por HORARIO (menor `pts[0].d`), DERIVADO de
        // los datos, no fijado. Con un inicio fijo (p. ej. 05:00), los trenes cuya salida GTFS es
        // anterior "nacían" ya retrasados y arrastraban ese retraso falso toda la línea (artefacto de
        // ventana). Arrancar en la primera salida real elimina ese artefacto. `arrived_at` de cada
        // tren en su origen se ancla al inicio (dwell trivialmente cubierto; la salida la gobierna
        // su hora programada + señal de salida, como en la realidad).
        if let Some(first_dep) = trains.iter().map(|t| t.pts[0].d as u32).min() {
            cfg.start_sec = first_dep.min(cfg.end_sec);
            let start = cfg.start_sec as f64;
            for t in &mut trains {
                t.arrived_at = start;
            }
        }

        // Parada mínima por estación (densidad de servicio) — floor de dwell (no rebaquea horarios).
        let dwell_map = dwell_por_estacion(&trains);
        for t in &mut trains {
            t.dwell_min = t
                .pts
                .iter()
                .map(|p| if p.pass { 0.0 } else { dwell_map.get(&p.id).copied().unwrap_or(DWELL_MIN) })
                .collect();
            t.target_stop = next_real_stop(&t.pts, 0);
        }

        let infra = build_infra(net, &trains, single_track_str);

        // Secuencia de bloques por tren (posición de entrada de cada bloque + recursos precalculados).
        for t in &mut trains {
            t.seq = construir_secuencia(&t.pts, &t.cum_l, &infra);
            t.cb = 0;
        }

        // Variabilidad de velocidad: un sorteo por tren y por BLOQUE (tramo entre dos señales), tirado
        // UNA vez antes del día con un flujo propio de la semilla (determinista: "lanzar incidencia
        // ahora" re-simula idéntico). Se guarda la z normalizada y no el factor: así la media y la σ
        // se re-escalan con la Vmax del punto (zonas de velocidad / LTV dentro del mismo bloque).
        if let Some(p0) = cfg.speed_var.filter(|p| p.is_finite() && *p > 0.0) {
            let mut rng = StdRng::seed_from_u64(cfg.seed ^ SPEED_VAR_SALT);
            for t in &mut trains {
                t.speed_p0 = p0;
                t.speed_z = (0..t.seq.len()).map(|_| trunc_std_normal(&mut rng, SPEED_VAR_TRUNC_Z)).collect();
            }
        }

        // Incidencias: pesos espaciales del sistema + tirada de dados ANTES del día (reproducible).
        let spatial = spatial_weights(net, &trains, &infra);
        let incidents =
            roll_incidents(&spatial, templates, cfg.start_sec as f64, cfg.end_sec as f64, cfg.seed);

        let n_res = infra.n_res;
        Engine {
            cfg,
            holds: None,
            clock: cfg.start_sec as f64,
            trains,
            infra,
            occ: vec![Vec::new(); n_res],
            incidents,
            train_min_delay: 0.0,
            max_delay: 0.0,
            peak_waiting: 0,
            waiting_now: 0,
            pax_min: 0.0,
            held_events: 0,
            timeline: Vec::new(),
            timeline_active: Vec::new(),
            next_sample: cfg.start_sec as f64,
            late_arrivals: Vec::new(),
        }
    }

    pub fn clock(&self) -> f64 {
        self.clock
    }

    /// Un sub-paso de física para toda la red: Pass 1 (ocupación) + Pass 2 (movimiento cinemático).
    pub fn step(&mut self, dt: f64) {
        self.clock += dt;
        let now = self.clock;

        // Efectos de incidencias vigentes AHORA (señales en rojo por cantón, cantones en vía única).
        let (dyn_red, dyn_single) = dyn_effects(&self.infra, &self.incidents, now);

        let free_run = self.cfg.free_run;
        let Engine { trains, infra, occ, late_arrivals, holds, .. } = self;
        let dynp = Dyn { red: &dyn_red, single: &dyn_single };
        for v in occ.iter_mut() {
            v.clear(); // reutiliza la capacidad; nada de realojar por step
        }

        // Pass 1: ocupación de bloques según el bloque comprometido (cb) de cada tren activo.
        let mut active: Vec<bool> = Vec::with_capacity(trains.len());
        let mut chain: Vec<u32> = Vec::new(); // reutilizado: reservas de vía única (modo sin horarios)
        for (idx, t) in trains.iter().enumerate() {
            if t.phase == Phase::Done {
                active.push(false);
                continue;
            }
            active.push(true);
            let b = &t.seq[t.cb];
            if b.origin {
                continue;
            }
            // Terminal: un tren que finaliza se aparta a vía secundaria (no retiene el bloque).
            if b.last && b.arr_idx == t.pts.len() - 1 && infra.terminals.contains(&b.dest) {
                continue;
            }
            // VÍA ÚNICA: la estación es un APARTADERO fuera de la vía; un tren PARADO en la estación
            // destino la LIBERA para que el sentido contrario pueda cruzar (testigo compartido).
            if b.last && dynp.block_single(b) && t.phase == Phase::Dwell && t.station_idx == b.arr_idx {
                continue;
            }
            // VÍA DOBLE: el andén de la dirección es una EXTENSIÓN del tramo de aproximación — un tren
            // parado en él SIGUE ocupando ese tramo, así que la señal de entrada al tramo queda en ROJO
            // mientras el tren está en la estación (no se vacía al llegar). Los trenes de detrás esperan
            // en el tramo previo (colapso realista), no se cuelan en vías extra.
            occ[dynp.block_res(b) as usize].push(idx);
            // Sin horarios: el tramo de vía única que se recorre reserva también la cadena de vía
            // única que cruzará sin parar (evita interbloqueos en apartaderos donde no para).
            if free_run && t.phase == Phase::Running {
                cadena_unica(t, &dynp, t.cb, &mut chain);
                for &r in &chain {
                    if !occ[r as usize].contains(&idx) {
                        occ[r as usize].push(idx);
                    }
                }
            }
        }

        // Pass 2: por tren (orden determinista), física + gating de bloques con ocupación viva.
        let mut waiting = 0usize;
        for idx in 0..trains.len() {
            if !active[idx] {
                continue;
            }
            let held = avanzar_tren(idx, trains, infra, &dynp, occ, now, dt, late_arrivals, free_run);
            if held {
                waiting += 1;
            }
        }

        // Métricas del sub-paso (acumuladas en locales; se vuelcan a self tras soltar los borrows).
        let pax = (NOMINAL_PAX * demanda(now)).round();
        let dt_min = dt / 60.0;
        let mut add_train_min = 0.0;
        let mut step_max = 0.0_f64;
        let mut add_pax = 0.0;
        let mut total_delay = 0.0_f64; // retraso ACUMULADO de la red en este instante (s)
        let mut new_conflicts = 0u64;
        let mut in_service = 0u32;
        for (idx, t) in trains.iter_mut().enumerate() {
            if !active[idx] {
                continue;
            }
            if t.in_service() {
                in_service += 1;
            }
            if t.delay_s > 0.0 {
                let dmin = t.delay_s / 60.0;
                add_train_min += dmin * dt_min;
                total_delay += t.delay_s;
                // pax·min de retraso: pasajeros a bordo (nominal × demanda horaria) × minutos de
                // retraso REAL de ESTE tren, integrado en el tiempo. Cada tren pesa por LO TARDE que
                // va (no un 0/1 por estar retenido), así el KPI del visor (derivado del `d` del
                // trace) y el de /numeros son la MISMA magnitud honesta, no un proxy por retenidos.
                add_pax += dmin * pax * dt_min;
            }
            step_max = step_max.max(t.delay_s);
            // Conflicto DISTINTO: el tren entra en retención (flanco de subida de `waiting`).
            if t.waiting && !t.was_waiting {
                new_conflicts += 1;
            }
            if t.waiting {
                if let Some(h) = holds.as_mut() {
                    acumular_retencion(h, t, now, dt);
                }
            }
            t.was_waiting = t.waiting;
        }

        self.train_min_delay += add_train_min;
        self.pax_min += add_pax;
        self.max_delay = self.max_delay.max(step_max);
        self.waiting_now = waiting;
        self.peak_waiting = self.peak_waiting.max(waiting);
        self.held_events += new_conflicts;

        // Muestreo de la línea temporal de retraso acumulado (cada SAMPLE_STEP_S de tiempo simulado).
        while now + 1e-9 >= self.next_sample {
            self.timeline.push((self.next_sample as u32, total_delay));
            self.timeline_active.push(in_service);
            self.next_sample += SAMPLE_STEP_S;
        }
    }

    pub fn run(&mut self) -> RunMetrics {
        while self.clock < self.cfg.end_sec as f64 {
            self.step(self.cfg.dt_sec);
        }
        self.metrics()
    }

    /// Corre la jornada COMPLETA grabando un trace (muestreado cada `sample_dt` s) para que el
    /// frontend lo reproduzca: es el mecanismo "lo que ves = lo que se simula". Consume el motor.
    pub fn run_trace(&mut self, sample_dt: f64) -> Trace {
        // El ranking de tramos que más retienen solo lo lee el visor: se mide aquí y no en `run`.
        self.holds = Some(HashMap::new());
        let n = self.trains.len();
        let mut ids: Vec<String> = Vec::new();
        let mut id_idx: HashMap<String, u32> = HashMap::new();
        let mut tt: Vec<TraceTrain> = self
            .trains
            .iter()
            .map(|t| TraceTrain {
                id: t.id.clone(),
                line: t.line.clone(),
                t0: f64::NAN,
                a: Vec::new(),
                b: Vec::new(),
                f: Vec::new(),
                v: Vec::new(),
                d: Vec::new(),
            })
            .collect();

        let end = self.cfg.end_sec as f64;
        let mut next_sample = self.clock;
        while self.clock < end {
            self.step(self.cfg.dt_sec);
            if self.clock + 1e-9 >= next_sample {
                for i in 0..n {
                    if let Some((a, b, frac, v)) = self.trains[i].trace_sample() {
                        let ai = intern(a, &mut ids, &mut id_idx);
                        let bi = intern(b, &mut ids, &mut id_idx);
                        if tt[i].t0.is_nan() {
                            tt[i].t0 = next_sample;
                        }
                        tt[i].a.push(ai);
                        tt[i].b.push(bi);
                        tt[i].f.push((frac * 1000.0).round().clamp(0.0, 1000.0) as u16);
                        tt[i].v.push((v * 3.6).round().max(0.0) as u16);
                        // Retraso real del tren en este instante (s). El techo de u16 (~18 h) nunca
                        // se alcanza en una jornada; el peor caso real ronda 35 min.
                        tt[i].d.push(self.trains[i].delay_s.round().clamp(0.0, 65535.0) as u16);
                    }
                }
                next_sample += sample_dt;
            }
        }
        tt.retain(|t| !t.a.is_empty());
        let mut arrivals = std::mem::take(&mut self.late_arrivals);
        arrivals.sort_by_key(|a| a.t); // orden temporal para el contador incremental y el log
        Trace { start: self.cfg.start_sec as f64, end, dt: sample_dt, ids, trains: tt, arrivals }
    }

    /// Los `n` tramos (cantones dirigidos) que más tiempo han retenido trenes en total, de mayor a
    /// menor. Solo tiene datos tras [`Engine::run_trace`] (modo visual); en las runs headless está vacío.
    pub fn hold_ranking(&self, n: usize) -> Vec<HoldSpot> {
        let Some(h) = &self.holds else { return Vec::new() };
        let mut names: Vec<&str> = vec![""; self.infra.stop_index.len()];
        for (id, &i) in &self.infra.stop_index {
            names[i as usize] = id;
        }
        let mut v: Vec<(&(u32, u32), &HoldAcc)> = h.iter().collect();
        // Desempate determinista por ids (el HashMap itera en orden aleatorio por proceso).
        v.sort_by(|a, b| b.1.secs.total_cmp(&a.1.secs).then(a.0.cmp(b.0)));
        v.into_iter()
            .take(n)
            .map(|(&(a, b), acc)| {
                let mut lines: Vec<(String, f64)> = acc.by_line.iter().map(|(l, &s)| (l.clone(), s)).collect();
                lines.sort_by(|x, y| y.1.total_cmp(&x.1).then(x.0.cmp(&y.0)));
                HoldSpot {
                    from: names[a as usize].to_string(),
                    to: names[b as usize].to_string(),
                    held_s: acc.secs,
                    events: acc.events,
                    max_s: acc.max_s,
                    lines,
                }
            })
            .collect()
    }

    /// Total de la jornada: (tren·segundos retenidos ante señales, nº de retenciones) sumando TODOS
    /// los tramos (no solo el ranking). Métrica comparable entre modos de horario: al contrario que el
    /// retraso, no depende de ir adelantado o atrasado respecto al horario. Solo tras `run_trace`.
    pub fn hold_total(&self) -> (f64, u64) {
        self.holds.as_ref().map_or((0.0, 0), |h| {
            h.values().fold((0.0, 0), |(s, e), a| (s + a.secs, e + a.events as u64))
        })
    }

    /// Estado observable de los trenes EN MARCHA ahora (para el trace/reproductor).
    pub fn observe(&self) -> Vec<FrameTrain> {
        let (dyn_red, dyn_single) = dyn_effects(&self.infra, &self.incidents, self.clock);
        let dynp = Dyn { red: &dyn_red, single: &dyn_single };
        let mut out = Vec::new();
        for t in &self.trains {
            if t.phase != Phase::Running {
                continue; // parado en estación (dentro de ella) o finalizado: no se dibuja
            }
            let i = t.canton_idx;
            if i + 1 >= t.pts.len() {
                continue;
            }
            let (a, b) = (&t.pts[i], &t.pts[i + 1]);
            let len = (t.cum_l[i + 1] - t.cum_l[i]).max(1.0);
            let frac = ((t.s - t.cum_l[i]) / len).clamp(0.0, 1.0);
            let aspect = t
                .seq
                .get(t.cb + 1)
                .filter(|nb| !nb.origin)
                .map(|nb| {
                    let m = self.infra.sub_m_id(nb.from, nb.to);
                    match aspecto_bloque(&self.infra, &self.occ, &dynp, nb.from, nb.to, nb.sb as u32, m) {
                        Aspecto::Green => 0,
                        Aspecto::Amber => 1,
                        Aspecto::Red => 2,
                    }
                })
                .unwrap_or(0);
            out.push(FrameTrain {
                id: t.id.clone(),
                line: t.line.clone(),
                from: a.id.clone(),
                to: b.id.clone(),
                frac,
                lat: a.lat + (b.lat - a.lat) * frac,
                lon: a.lon + (b.lon - a.lon) * frac,
                speed_kmh: t.v * 3.6,
                delay_s: t.delay_s,
                waiting: t.waiting,
                aspect,
            });
        }
        out
    }

    /// Incidencias concretas que salieron en la tirada de este día (para reporte/depuración).
    pub fn incidents(&self) -> &[ActiveIncident] {
        &self.incidents
    }

    /// Inyecta una incidencia MANUAL (lanzada por el usuario "ahora"): se añade a las del día antes
    /// de correr el trace. Su ventana `[t0, t0+dur]` la aplica el motor como cualquier otra.
    pub fn inject_incident(&mut self, from: String, to: String, effect: IncidentEffect, t0: f64, dur_min: f64, motivo: String) {
        self.incidents.push(ActiveIncident { from, to, effect, t0, t1: t0 + dur_min * 60.0, motivo });
    }

    /// Línea temporal del retraso acumulado de red: `(instante_s, retraso_total_s)` cada minuto.
    pub fn timeline(&self) -> &[(u32, f64)] {
        &self.timeline
    }

    /// Trenes en servicio en cada muestra de [`Engine::timeline`] (mismo índice).
    pub fn timeline_active(&self) -> &[u32] {
        &self.timeline_active
    }

    pub fn metrics(&self) -> RunMetrics {
        // Pico del retraso acumulado y su instante, a partir de la línea temporal.
        let (peak_total_delay_s, peak_time) = self
            .timeline
            .iter()
            .fold((0.0_f64, self.cfg.start_sec), |(pk, pt), &(t, d)| {
                if d > pk { (d, t) } else { (pk, pt) }
            });
        // Recuperación: primer instante tras el pico en que el retraso acumulado baja del equilibrio.
        let recovery_time = self
            .timeline
            .iter()
            .find(|&&(t, d)| t >= peak_time && d < EQUILIBRIUM_S)
            .map(|&(t, _)| t);

        RunMetrics {
            n_trains: self.trains.len(),
            sim_seconds: self.cfg.end_sec - self.cfg.start_sec,
            train_min_delay: self.train_min_delay,
            max_delay_s: self.max_delay,
            peak_waiting: self.peak_waiting,
            final_waiting: self.waiting_now,
            pax_min: self.pax_min,
            held_events: self.held_events,
            peak_total_delay_s,
            peak_time,
            recovery_time,
        }
    }

    /// Registro de todas las llegadas tarde (≥ `LATE_LOG_THRESHOLD_S`) acumuladas en la jornada,
    /// en orden de aparición. Diagnóstico: permite listar los retrasos más severos por tren/estación.
    pub fn late_arrivals(&self) -> &[LateArrival] {
        &self.late_arrivals
    }

    /// Diagnóstico de UN tren por id, en el instante actual del reloj: estado de fase, retraso,
    /// retención por señal, y ubicación (estación de dwell o cantón en marcha, por stop_id). Para
    /// rastrear DÓNDE y POR QUÉ se acumula el retraso (física lenta vs retención). `None` = no existe.
    pub fn train_diag(&self, id: &str) -> Option<TrainDiag> {
        let t = self.trains.iter().find(|t| t.id == id)?;
        let (phase, from_id, to_id) = match t.phase {
            Phase::Dwell => {
                let k = t.station_idx;
                let from = t.pts[k].id.clone();
                let to = t.pts.get(t.target_stop).map(|p| p.id.clone()).unwrap_or_default();
                (0u8, from, to)
            }
            Phase::Running => {
                let i = t.canton_idx.min(t.pts.len().saturating_sub(2));
                (1u8, t.pts[i].id.clone(), t.pts[i + 1].id.clone())
            }
            Phase::Done => (2u8, String::new(), String::new()),
        };
        Some(TrainDiag {
            phase,
            line: t.line.clone(),
            delay_s: t.delay_s,
            waiting: t.waiting,
            from_id,
            to_id,
            speed_kmh: t.v * 3.6,
        })
    }

    /// Diagnóstico: ids de los trenes que ocupan AHORA cada sub-bloque del cantón dirigido
    /// `from_id → to_id` (según el estado de `occ` tras el último `step`). `(sub_bloque, id)`.
    /// `None` = el cantón no existe. Sirve para ver QUIÉN bloquea un tramo y cuánto.
    pub fn block_occupants(&self, from_id: &str, to_id: &str) -> Option<Vec<(u32, String)>> {
        let a = self.infra.nid(from_id)?;
        let b = self.infra.nid(to_id)?;
        let canon = self.infra.canon_res(a, b)?;
        let stat_single = self.infra.single_track.contains(&canon);
        let m = self.infra.sub_m_id(a, b);
        let mut out = Vec::new();
        for sb in 0..m {
            let res = if stat_single {
                canon
            } else {
                self.infra.dir_res(a, b, sb).unwrap_or(canon)
            };
            for &idx in &self.occ[res as usize] {
                out.push((sb, self.trains[idx].id.clone()));
            }
        }
        Some(out)
    }
}

/// Foto de diagnóstico de un tren en el instante actual (ver `Engine::train_diag`).
pub struct TrainDiag {
    pub phase: u8, // 0=dwell (parado en estación), 1=en marcha, 2=finalizado
    pub line: String,
    pub delay_s: f64,
    pub waiting: bool,
    pub from_id: String, // estación de dwell, o inicio del cantón en marcha
    pub to_id: String,   // próxima parada real (dwell) o fin del cantón
    pub speed_kmh: f64,
}

/// Avanza un tren un `step` completo (sub-pasos físicos ≤ `PHYS_SUBSTEP_S`). Devuelve `true` si el
/// tren queda RETENIDO por señal/ocupación (no por dwell ni por esperar su hora).
fn avanzar_tren(
    idx: usize,
    trains: &mut [Train],
    infra: &Infra,
    dynp: &Dyn,
    occ: &mut Vec<Vec<usize>>,
    now: f64,
    dt: f64,
    late: &mut Vec<LateArrival>,
    free_run: bool,
) -> bool {
    // --- Parado en estación: gestión de dwell / hora de salida / señal de salida ---
    if trains[idx].phase == Phase::Dwell {
        trains[idx].waiting = false;
        let k = trains[idx].station_idx;
        let dwell_ok = now - trains[idx].arrived_at >= trains[idx].dwell_min[k];
        let sched_ok = !debe_esperar_horario(free_run, k, now, trains[idx].pts[k].d);
        if !(dwell_ok && sched_ok) {
            trains[idx].held_since = f64::NAN; // parado por su dwell/hora, no retenido
            return false;
        }
        // Intentar cruzar la señal de salida (primer bloque del cantón k→k+1 = seq[cb+1]).
        let cb = trains[idx].cb;
        if cb + 1 >= trains[idx].seq.len() {
            trains[idx].phase = Phase::Done;
            return false;
        }
        let nb = trains[idx].seq[cb + 1].clone();
        if !bloque_entrada(infra, occ, dynp, trains, idx, &nb, now, free_run) {
            trains[idx].waiting = true; // salida bloqueada por ocupación
            // Retenido en el andén con la salida en rojo: pasado el umbral, el retraso vivo es lo que
            // lleva de más sobre su hora de salida.
            let t = &mut trains[idx];
            if marcar_retencion(t, now) {
                t.delay_s = t.delay_s.max(now - t.pts[k].d);
            }
            return true;
        }
        trains[idx].held_since = f64::NAN;
        // Sale: libera el bloque de andén y ocupa el primer bloque del cantón de salida.
        ocupar(occ, dynp, idx, &trains[idx].seq[cb], false);
        trains[idx].cb = cb + 1;
        ocupar(occ, dynp, idx, &trains[idx].seq[trains[idx].cb], true);
        trains[idx].phase = Phase::Running;
        trains[idx].canton_idx = k;
        trains[idx].target_stop = next_real_stop(&trains[idx].pts, k);
        trains[idx].v = 0.0;
        // continúa hacia la integración de movimiento este mismo sub-paso
    }

    if trains[idx].phase != Phase::Running {
        return false;
    }

    // --- En marcha: integración cinemática con gating de bloques por posición ---
    let mut held = false;
    let mut remaining = dt;
    while remaining > 1e-9 {
        let h = remaining.min(PHYS_SUBSTEP_S);
        remaining -= h;

        // Cantón y límite de vía en la posición actual.
        let ci = trains[idx].canton_idx;
        let local_x = trains[idx].s - trains[idx].cum_l[ci];
        let infra_v = physics::infra_limit(&trains[idx].canton_zones[ci], local_x);
        // Velocidad de crucero = Vmax permitida × factor del maquinista en este tramo entre señales.
        let v_lim = trains[idx].veh.vmax.min(infra_v);
        let mut v_cap = match trains[idx].speed_z.get(trains[idx].cb) {
            Some(&z) => v_lim * speed_factor(trains[idx].speed_p0, v_lim * 3.6, z),
            None => v_lim,
        };

        // Objetivos de frenada: próxima parada REAL (v=0) y señal en ROJO más cercana (v=0).
        let stop_x = trains[idx].cum_l[trains[idx].target_stop];
        let mut restrictions = vec![Restriction { x: stop_x, v: 0.0 }];
        let cb = trains[idx].cb;
        if cb + 1 < trains[idx].seq.len() {
            let nb = trains[idx].seq[cb + 1].clone();
            // Solo importa el bloque siguiente si está ANTES de la próxima parada real.
            if nb.x_enter < stop_x - 1e-6 {
                let enterable = bloque_entrada(infra, occ, dynp, trains, idx, &nb, now, free_run);
                if !enterable {
                    restrictions.push(Restriction { x: nb.x_enter, v: 0.0 }); // rojo → frenar antes
                } else {
                    // Ámbar: siguiente libre pero el de más allá ocupado → reducir un 20%.
                    if let Some(nn) = trains[idx].seq.get(cb + 2) {
                        if !nn.origin
                            && nn.x_enter < stop_x - 1e-6
                            && bloque_ocupado(dynp, occ, trains, nn, Some(idx))
                        {
                            v_cap = v_cap.min(trains[idx].veh.vmax.min(infra_v) * AMBER_FACTOR);
                        }
                    }
                }
            }
        }
        let v_allow = v_cap.min(physics::braking_cap(&restrictions, trains[idx].s, trains[idx].veh.decel));

        // Integración (acelera a ¾ de la máxima hacia el límite; si no, sigue la envolvente).
        let v = trains[idx].v;
        let v_next = if v < v_allow {
            (v + ACCEL_FRACTION * trains[idx].veh.accel_at(v) * h).min(v_allow)
        } else {
            v_allow
        };
        let ds = 0.5 * (v + v_next) * h;
        let mut new_s = trains[idx].s + ds;
        trains[idx].v = v_next;

        // ¿Ha llegado a la próxima parada REAL?
        if new_s >= stop_x - 0.5 {
            let k = trains[idx].target_stop;
            trains[idx].s = stop_x;
            trains[idx].v = 0.0;
            trains[idx].canton_idx = (k.saturating_sub(1)).min(trains[idx].pts.len() - 2);
            trains[idx].delay_s = (now - trains[idx].pts[k].a).max(0.0);
            // Registro de CTC: si el tren llega ≥1 min tarde a esta estación, se anota (instante,
            // tren, línea, stop, retraso). Vale para paradas intermedias Y terminal (mismo punto).
            if trains[idx].delay_s >= LATE_LOG_THRESHOLD_S {
                late.push(LateArrival {
                    t: now as u32,
                    train: trains[idx].id.clone(),
                    line: trains[idx].line.clone(),
                    stop: trains[idx].pts[k].id.clone(),
                    delay: trains[idx].delay_s.min(65535.0) as u16,
                });
            }
            if k >= trains[idx].pts.len() - 1 {
                // Terminal: libera el bloque y termina.
                ocupar(occ, dynp, idx, &trains[idx].seq[trains[idx].cb], false);
                trains[idx].phase = Phase::Done;
            } else {
                trains[idx].phase = Phase::Dwell;
                trains[idx].station_idx = k;
                trains[idx].arrived_at = now;
            }
            trains[idx].waiting = false;
            return held;
        }

        // Avance de bloques cruzados (mid-cantón / puntos de paso) con ocupación viva.
        loop {
            let cb = trains[idx].cb;
            if cb + 1 >= trains[idx].seq.len() {
                break;
            }
            let nb = trains[idx].seq[cb + 1].clone();
            if nb.x_enter >= stop_x - 1e-6 || nb.x_enter > new_s {
                break; // el siguiente bloque es la parada, o aún no lo alcanza
            }
            if bloque_entrada(infra, occ, dynp, trains, idx, &nb, now, free_run) {
                ocupar(occ, dynp, idx, &trains[idx].seq[cb], false);
                trains[idx].cb = cb + 1;
                ocupar(occ, dynp, idx, &trains[idx].seq[trains[idx].cb], true);
                // Al cruzar la frontera de un cantón (sb 0), avanza el cantón físico.
                if nb.sb == 0 {
                    trains[idx].canton_idx = (trains[idx].canton_idx + 1).min(trains[idx].pts.len() - 2);
                }
            } else {
                // Señal en rojo: clavar el tren justo antes de la señal.
                new_s = nb.x_enter;
                trains[idx].v = 0.0;
                held = true;
                break;
            }
        }
        trains[idx].s = new_s;
        trains[idx].waiting = held;
        if held {
            break;
        }
    }
    // Retenido en plena vía: pasado el umbral, retraso vivo = el de su posición respecto al horario.
    let t = &mut trains[idx];
    if held {
        if marcar_retencion(t, now) {
            let live = retraso_en_posicion(&t.pts, &t.cum_l, t.canton_idx, t.s, now);
            t.delay_s = t.delay_s.max(live);
        }
    } else {
        t.held_since = f64::NAN;
    }
    held
}

/// Factor de velocidad de crucero de un maquinista (nadie circula clavado a Vmax): `f = 1 − p(V) +
/// σ(V)·z`, con `p(V) = p0·V/120` (acotado a `SPEED_VAR_MAX`), `σ(V) = p(V)/2` y `z` ∈ [−2, 2] el
/// sorteo del tramo. Es decir `f ~ N(1−p(V), σ(V)²)` truncada SIMÉTRICAMENTE a `[1−2p(V), 1]`:
/// nunca supera Vmax, y cuanto MENOR es la Vmax más se acerca la media a Vmax y más estrecha es la
/// campana (a 120 km/h se cumplen exactamente los valores del usuario). Invariante: p0 = 0 → f = 1.
fn speed_factor(p0: f64, vmax_kmh: f64, z: f64) -> f64 {
    let p = (p0 * vmax_kmh.max(0.0) / SPEED_VAR_REF_KMH).clamp(0.0, SPEED_VAR_MAX);
    1.0 - p + (p / SPEED_VAR_TRUNC_Z) * z.clamp(-SPEED_VAR_TRUNC_Z, SPEED_VAR_TRUNC_Z)
}

/// Normal estándar truncada a `[−k, k]` (rechazo sobre Box-Muller; con k = 2 acepta ~95 %).
/// Aleatoriedad de conducción, en un flujo independiente del de las incidencias.
fn trunc_std_normal(rng: &mut StdRng, k: f64) -> f64 {
    loop {
        let u1 = rng.gen::<f64>().max(f64::MIN_POSITIVE);
        let u2 = rng.gen::<f64>();
        let z = (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos();
        if z.abs() <= k {
            return z;
        }
    }
}

/// ¿Debe el tren seguir parado en la parada `k` esperando su hora programada `dep`? Con horario,
/// NUNCA sale antes de su hora. En modo sin horarios solo la cabecera (`k == 0`) conserva su hora
/// de salida; en el resto sale en cuanto cumple la parada mínima (la comprueba quien llama).
fn debe_esperar_horario(free_run: bool, k: usize, now: f64, dep: f64) -> bool {
    (!free_run || k == 0) && now < dep
}

/// Suma un paso `dt` de retención del tren `t` (que está `waiting`) al tramo cuya señal de entrada
/// le tiene parado: el bloque siguiente a su bloque comprometido (`seq[cb+1]`). Vale tanto si espera
/// en plena vía como en el andén con la salida en rojo (en ambos casos `cb+1` es el bloque vetado).
/// Se agrega por CANTÓN (`from→to`), no por sub-bloque: es la unidad que entiende el usuario.
fn acumular_retencion(h: &mut HashMap<(u32, u32), HoldAcc>, t: &Train, now: f64, dt: f64) {
    let Some(nb) = t.seq.get(t.cb + 1) else { return };
    let acc = h.entry((nb.from, nb.to)).or_default();
    acc.secs += dt;
    if !t.was_waiting {
        acc.events += 1;
    }
    if !t.held_since.is_nan() {
        acc.max_s = acc.max_s.max(now - t.held_since);
    }
    match acc.by_line.get_mut(&t.line) {
        Some(v) => *v += dt,
        None => {
            acc.by_line.insert(t.line.clone(), dt);
        }
    }
}

/// Anota el inicio de la retención y dice si ya supera `HOLD_LIVE_DELAY_S` (→ recalcular el retraso
/// en vivo). Mientras sigue retenido el retraso solo puede crecer; al llegar a la siguiente
/// estación se vuelve a medir exacto (hora real − llegada programada).
fn marcar_retencion(t: &mut Train, now: f64) -> bool {
    if t.held_since.is_nan() {
        t.held_since = now;
    }
    now - t.held_since >= HOLD_LIVE_DELAY_S
}

// ---- Física de bloques pura (reutilizada de la Fase 1b) ------------------------------------

/// Interna un stop_id en la tabla del trace (id→índice), para arrays compactos de enteros.
fn intern(s: &str, tab: &mut Vec<String>, idx: &mut HashMap<String, u32>) -> u32 {
    if let Some(&i) = idx.get(s) {
        i
    } else {
        let i = tab.len() as u32;
        tab.push(s.to_string());
        idx.insert(s.to_string(), i);
        i
    }
}

/// Índice de la próxima parada REAL (pass=false) estrictamente posterior a `from`; el último si no.
fn next_real_stop(pts: &[Pt], from: usize) -> usize {
    for j in (from + 1)..pts.len() {
        if !pts[j].pass {
            return j;
        }
    }
    pts.len() - 1
}

/// ¿Está OCUPADO el recurso del bloque `b`? (gating del bucle CALIENTE). `except` = el propio tren.
/// Todo se lee de índices `u32` ya precalculados en el bloque — cero claves `String`.
#[inline]
fn bloque_ocupado(dynp: &Dyn, occ: &[Vec<usize>], trains: &[Train], b: &Block, except: Option<usize>) -> bool {
    // Señal de entrada en rojo por incidencia (solo el sub-bloque 0 = la señal de entrada al cantón).
    if b.sb == 0 && dynp.red.contains(&(b.from, b.to)) {
        return true;
    }
    let unica = dynp.block_single(b);
    let arr = &occ[dynp.block_res(b) as usize];
    if !unica && b.plat_tracks > 1 {
        return anden_lleno(arr, trains, except, b.plat_tracks);
    }
    let cap = if unica { 1 } else { b.cap_double as usize };
    arr.iter().filter(|&&x| Some(x) != except).count() >= cap
}

/// Último tramo hacia una estación con VARIAS vías principales por acceso (Sants: 2). La señal de
/// entrada se cierra si hay un tren CIRCULANDO por el tramo (la aproximación sigue siendo una sola
/// vía), o si los trenes PARADOS en el andén ya ocupan todas las vías principales de ese acceso. Un
/// tren estacionado no basta para cerrarla: el siguiente entra por la otra vía.
fn anden_lleno(occupants: &[usize], trains: &[Train], except: Option<usize>, tracks: u32) -> bool {
    let mut parados = 0;
    for &x in occupants {
        if Some(x) == except {
            continue;
        }
        if trains[x].phase != Phase::Dwell {
            return true;
        }
        parados += 1;
    }
    parados >= tracks
}

/// MODO SIN HORARIOS: recursos de vía única que un tren debe reservar AL ENTRAR en el bloque
/// `seq[k]`, además del propio: si su siguiente estación es un PUNTO DE PASO (no para) y el cantón
/// que sigue también es vía única, cruzará sin detenerse, así que necesita toda la cadena de vía
/// única hasta su próxima parada real. Sin esto, dos exprés en sentidos opuestos se encuentran en un
/// apartadero donde ninguno para y cada uno retiene el tramo que pide el otro (interbloqueo real del
/// R15 en Riudecanyes-Botarell). Con horario no hace falta: los cruces los ordena el horario.
fn cadena_unica(t: &Train, dynp: &Dyn, k: usize, out: &mut Vec<u32>) {
    out.clear();
    let Some(b) = t.seq.get(k) else { return };
    if b.origin || !dynp.block_single(b) {
        return;
    }
    for nb in &t.seq[k + 1..] {
        if nb.sb != 0 {
            continue; // sub-bloques interiores del mismo cantón (vía única = un solo recurso)
        }
        // `nb` empieza en la estación `pts[arr_idx-1]`: si el tren para allí, ahí acaba la cadena.
        if !t.pts[nb.arr_idx - 1].pass || !dynp.block_single(nb) {
            break;
        }
        out.push(nb.res_single);
    }
}

fn regional_entrando_pronto(
    infra: &Infra,
    trains: &[Train],
    rk: u32,
    now: f64,
    except: usize,
) -> bool {
    let Some(users) = infra.single_track_users.get(&rk) else { return false };
    for &(k, j) in users {
        if k == except || !trains[k].regional {
            continue;
        }
        let entrada = trains[k].pts[j].d + trains[k].delay_s;
        if entrada >= now && entrada <= now + VENTANA_PRIORIDAD_SEC {
            return true;
        }
    }
    false
}

fn bloque_entrada(
    infra: &Infra,
    occ: &[Vec<usize>],
    dynp: &Dyn,
    trains: &[Train],
    idx: usize,
    nb: &Block,
    now: f64,
    free_run: bool,
) -> bool {
    if bloque_ocupado(dynp, occ, trains, nb, Some(idx)) {
        return false;
    }
    if free_run && nb.sb == 0 {
        // Toda la cadena de vía única que cruzará sin parar ha de estar libre (ver `cadena_unica`).
        let k = trains[idx].cb + 1;
        let mut chain = Vec::new();
        cadena_unica(&trains[idx], dynp, k, &mut chain);
        if chain.iter().any(|&r| occ[r as usize].iter().any(|&x| x != idx)) {
            return false;
        }
    }
    // Prioridad de regionales en vía única: se anticipa con su HORARIO; sin horarios no hay reserva.
    if !free_run
        && !trains[idx].regional
        && dynp.block_single(nb)
        && regional_entrando_pronto(infra, trains, nb.res_single, now, idx)
    {
        return false;
    }
    true
}

fn ocupar(occ: &mut [Vec<usize>], dynp: &Dyn, idx: usize, b: &Block, add: bool) {
    if b.origin {
        return;
    }
    let arr = &mut occ[dynp.block_res(b) as usize];
    if add {
        if !arr.contains(&idx) {
            arr.push(idx);
        }
    } else if let Some(pos) = arr.iter().position(|&x| x == idx) {
        arr.swap_remove(pos);
    }
}

// ---- Aspecto de señal — ruta FRÍA (visor): resuelve recursos por HashMap en `Infra` -----------

/// ¿Ocupado el recurso (a,b,sb)? Variante FRÍA (solo visor): resuelve el recurso por lookup en vez
/// de leerlo del bloque (no vale la pena precalcular para una foto puntual).
fn occ_cold(infra: &Infra, occ: &[Vec<usize>], dynp: &Dyn, a: u32, b: u32, sb: u32) -> bool {
    if sb == 0 && dynp.red.contains(&(a, b)) {
        return true;
    }
    let canon = infra.canon_res(a, b);
    let unica = canon.is_some_and(|c| infra.single_track.contains(&c) || dynp.single.contains(&c));
    let res = if unica {
        match canon {
            Some(c) => c,
            None => return false,
        }
    } else {
        match infra.dir_res(a, b, sb) {
            Some(r) => r,
            None => return false,
        }
    };
    // Capacidad 1 por bloque (incluido el de aproximación al andén) — igual que el bucle caliente.
    occ.get(res as usize).is_some_and(|arr| !arr.is_empty())
}

fn aspecto_bloque(infra: &Infra, occ: &[Vec<usize>], dynp: &Dyn, a: u32, b: u32, sb: u32, m: u32) -> Aspecto {
    if occ_cold(infra, occ, dynp, a, b, sb) {
        return Aspecto::Red;
    }
    if siguiente_rojo(infra, occ, dynp, a, b, sb, m) {
        Aspecto::Amber
    } else {
        Aspecto::Green
    }
}

fn siguiente_rojo(infra: &Infra, occ: &[Vec<usize>], dynp: &Dyn, a: u32, b: u32, sb: u32, m: u32) -> bool {
    if sb < m - 1 {
        return occ_cold(infra, occ, dynp, a, b, sb + 1);
    }
    salida_ocupada(infra, occ, dynp, b, a)
}

fn salida_ocupada(infra: &Infra, occ: &[Vec<usize>], dynp: &Dyn, st: u32, came_from: u32) -> bool {
    let Some(dests) = infra.edges_from.get(&st) else { return false };
    for &to in dests {
        if to == came_from {
            continue;
        }
        if occ_cold(infra, occ, dynp, st, to, 0) {
            return true;
        }
    }
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Aspecto {
    Green,
    Amber,
    Red,
}

// ---- Precálculo (una vez, en new) ---------------------------------------------------------

/// Parada mínima por estación (20–60 s) escalada por la densidad de servicio del GTFS (√ frecuencia).
fn dwell_por_estacion(trains: &[Train]) -> HashMap<String, f64> {
    let mut freq: HashMap<&str, u32> = HashMap::new();
    let mut max_freq = 1u32;
    for t in trains {
        for p in &t.pts {
            if !p.pass {
                let c = freq.entry(p.id.as_str()).or_insert(0);
                *c += 1;
                max_freq = max_freq.max(*c);
            }
        }
    }
    freq.iter()
        .map(|(&id, &c)| {
            let d = (DWELL_MIN + (DWELL_MAX - DWELL_MIN) * (c as f64 / max_freq as f64).sqrt()).round();
            (id.to_string(), d)
        })
        .collect()
}

/// Secuencia de bloques de un tren, situados por POSICIÓN de ruta. `x_enter` del bloque (cantón i,
/// sub-bloque sb) = `cum_l[i] + sb·(L_i/m_i)`. Centinela `origin` para la salida de la cabecera.
fn construir_secuencia(pts: &[Pt], cum_l: &[f64], infra: &Infra) -> Vec<Block> {
    let nid = |id: &str| infra.stop_index[id];
    let mut seq = Vec::new();
    let o = nid(&pts[0].id);
    seq.push(Block {
        origin: true,
        from: o,
        to: o,
        sb: 0,
        last: false,
        dest: o,
        arr_idx: 0,
        x_enter: 0.0,
        res: 0,
        res_single: 0,
        stat_single: false,
        cap_double: 1,
        plat_tracks: 1,
    });
    for i in 0..pts.len() - 1 {
        let a = nid(&pts[i].id);
        let b = nid(&pts[i + 1].id);
        let m = infra.sub_m_id(a, b) as usize;
        let len = cum_l[i + 1] - cum_l[i];
        let res_single = infra.canon_res(a, b).expect("todo cantón tiene recurso canónico");
        let stat_single = infra.single_track.contains(&res_single);
        for sb in 0..m {
            let res = if stat_single {
                res_single
            } else {
                infra.dir_res(a, b, sb as u32).expect("todo sub-bloque de vía doble tiene recurso")
            };
            seq.push(Block {
                origin: false,
                from: a,
                to: b,
                sb,
                last: sb == m - 1,
                dest: b,
                arr_idx: i + 1,
                x_enter: cum_l[i] + len * sb as f64 / m as f64,
                res,
                res_single,
                stat_single,
                // Capacidad 1 por bloque, incluido el último (aproximación al andén): así la señal
                // de entrada al último tramo se pone ROJA cuando un tren lo ocupa (como en la
                // realidad). Los terminales liberan el bloque al finalizar (no lo retienen).
                cap_double: 1,
                // Salvo en estaciones con varias vías principales por acceso (Sants): ahí un tren
                // PARADO solo cierra la señal cuando están todas ocupadas (ver `anden_lleno`).
                plat_tracks: if sb == m - 1 && !stat_single { topology::approach_main_tracks(&pts[i + 1].id) } else { 1 },
            });
        }
    }
    seq
}

fn build_infra(net: &Network, trains: &[Train], single_track_str: HashSet<String>) -> Infra {
    let coord = |id: &str| net.coord(id); // estaciones y bifurcaciones

    // 1. Internar cada stop_id → nid (índices 0..N contiguos, en orden de primer uso: determinista).
    let mut stop_index: HashMap<String, u32> = HashMap::new();
    for t in trains {
        for p in &t.pts {
            let next = stop_index.len() as u32;
            stop_index.entry(p.id.clone()).or_insert(next);
        }
    }
    // 3. Sub-bloques por cantón dirigido + aristas salientes + ASIGNACIÓN de recursos de ocupación
    //    a índices `u32` (canónico por cantón; dirigido por sub-bloque en vía doble). El bucle
    //    caliente indexa `occ` por estos ids sin volver a hashear cadenas.
    let mut sub_blocks: HashMap<(u32, u32), u32> = HashMap::new();
    let mut edges_from: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut res_canon: HashMap<(u32, u32), u32> = HashMap::new();
    let mut res_dir: HashMap<(u32, u32, u32), u32> = HashMap::new();
    let mut single_track: HashSet<u32> = HashSet::new();
    let mut next_res: u32 = 0;
    for t in trains {
        for w in t.pts.windows(2) {
            let a = stop_index[&w[0].id];
            let b = stop_index[&w[1].id];
            if sub_blocks.contains_key(&(a, b)) {
                continue;
            }
            let unica = single_track_str.contains(&canon(&w[0].id, &w[1].id));
            let m = match (coord(&w[0].id), coord(&w[1].id)) {
                (Some(ca), Some(cb)) => calc_sub_blocks_len(canton_len_m(&w[0].id, &w[1].id, ca, cb) / 1000.0, unica) as u32,
                _ => 1,
            };
            // Señales extra declaradas en topología (solo vía doble: en vía única manda el testigo).
            let m = if unica { m } else { m + topology::extra_block_signals(&w[0].id, &w[1].id) };
            sub_blocks.insert((a, b), m);
            edges_from.entry(a).or_default().push(b);
            let ckey = if a < b { (a, b) } else { (b, a) };
            let cres = *res_canon.entry(ckey).or_insert_with(|| {
                let r = next_res;
                next_res += 1;
                r
            });
            if unica {
                single_track.insert(cres); // vía única estática → comparte el recurso canónico
            } else {
                for sb in 0..m {
                    res_dir.insert((a, b, sb), next_res);
                    next_res += 1;
                }
            }
        }
    }

    // 4. Terminales: extremos que nunca son parada de paso.
    let mut extremo: HashSet<u32> = HashSet::new();
    let mut paso: HashSet<u32> = HashSet::new();
    for t in trains {
        let n = t.pts.len();
        for (j, p) in t.pts.iter().enumerate() {
            let id = stop_index[&p.id];
            if j == 0 || j == n - 1 {
                extremo.insert(id);
            } else {
                paso.insert(id);
            }
        }
    }
    let terminals: HashSet<u32> = extremo.into_iter().filter(|id| !paso.contains(id)).collect();

    // 5. Usuarios de cada cantón de vía única (para la prioridad de cruce regional), por recurso canónico.
    let mut single_track_users: HashMap<u32, Vec<(usize, usize)>> = HashMap::new();
    for (idx, t) in trains.iter().enumerate() {
        for j in 0..t.pts.len() - 1 {
            let a = stop_index[&t.pts[j].id];
            let b = stop_index[&t.pts[j + 1].id];
            let ckey = if a < b { (a, b) } else { (b, a) };
            if let Some(&cres) = res_canon.get(&ckey) {
                if single_track.contains(&cres) {
                    single_track_users.entry(cres).or_default().push((idx, j));
                }
            }
        }
    }

    Infra {
        stop_index,
        sub_blocks,
        single_track,
        terminals,
        edges_from,
        single_track_users,
        res_dir,
        res_canon,
        n_res: next_res as usize,
    }
}

/// Pesos ESPACIALES de dónde ocurren las incidencias (misma distribución para todos los tipos):
/// más probable en **nudos/Barcelona**, **estaciones grandes** (más vías), **tramos frecuentados**
/// (más servicios) y **terminales**. Devuelve `(from, to, peso)` por cantón dirigido único.
fn spatial_weights(net: &Network, trains: &[Train], infra: &Infra) -> Vec<(String, String, f64)> {
    let mut freq: HashMap<(String, String), f64> = HashMap::new();
    for t in trains {
        for w in t.pts.windows(2) {
            *freq.entry((w[0].id.clone(), w[1].id.clone())).or_insert(0.0) += 1.0;
        }
    }
    let max_freq = freq.values().copied().fold(1.0_f64, f64::max);
    let tracks_of = |id: &str| topology::platform_tracks(net.stop_name(id)) as f64;
    let max_tracks = freq
        .keys()
        .map(|(a, b)| tracks_of(a).max(tracks_of(b)))
        .fold(2.0_f64, f64::max);
    let is_hub = |id: &str| net.stop_name(id).to_lowercase().contains("barcelona");

    let mut out: Vec<(String, String, f64)> = freq
        .into_iter()
        .map(|((a, b), f)| {
            let tnorm = tracks_of(&a).max(tracks_of(&b)) / max_tracks;
            let fnorm = f / max_freq;
            let is_terminal = |id: &str| infra.nid(id).is_some_and(|n| infra.terminals.contains(&n));
            let terminal = if is_terminal(&a) || is_terminal(&b) { 1.0 } else { 0.0 };
            let hub = if is_hub(&a) || is_hub(&b) { 2.0 } else { 0.0 };
            let w = (1.0 + tnorm) * (1.0 + fnorm) * (1.0 + terminal) * (1.0 + hub);
            (a, b, w)
        })
        .collect();
    // Orden ESTABLE: el `HashMap` itera en orden aleatorio por proceso, y `sample_canton` indexa por
    // posición → sin ordenar, la MISMA semilla daría ubicaciones distintas en cada ejecución.
    out.sort_by(|x, y| x.0.cmp(&y.0).then(x.1.cmp(&y.1)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fleet_por_linea() {
        assert_eq!(fleet_for_line("R2"), "450");
        assert_eq!(fleet_for_line("R2N"), "450");
        assert_eq!(fleet_for_line("R1"), "465");
        assert_eq!(fleet_for_line("R3"), "465"); // R3 no es 470 (excluida) → alta frecuencia
        assert_eq!(fleet_for_line("R11"), "447"); // R11 excluida de 470 y no alta frecuencia
        assert_eq!(fleet_for_line("R7"), "447"); // por defecto
    }

    #[test]
    fn vehiculo_465_existe_y_es_civia() {
        let v = vehicle_for_line("R1"); // R1 → 465
        assert!((v.vmax - 120.0 / 3.6).abs() < 1e-6);
        assert!(v.mass_kg > 150_000.0);
    }

    #[test]
    fn retraso_vivo_por_posicion() {
        let pt = |d: f64, a: f64| Pt { id: String::new(), a, d, lat: 0.0, lon: 0.0, pass: false };
        // Sale de A a las 1000 s, debe llegar a B a las 1600 s; 3000 m de vía.
        let pts = vec![pt(1000.0, 0.0), pt(1700.0, 1600.0)];
        let cum = vec![0.0, 3000.0];
        // A mitad de vía (1500 m) debía pasar a las 1300 s; retenido ahí a las 2500 s → 1200 s tarde,
        // = previsión en B: 2500 + 300 s restantes (al ritmo del horario) − 1600.
        let d = retraso_en_posicion(&pts, &cum, 0, 1500.0, 2500.0);
        assert!((d - 1200.0).abs() < 1e-9);
        assert!((d - (2500.0 + 300.0 - 1600.0)).abs() < 1e-9);
        // Justo en la salida y a su hora: 0.
        assert!(retraso_en_posicion(&pts, &cum, 0, 0.0, 1000.0).abs() < 1e-9);
    }

    #[test]
    fn calc_sub_blocks_referencia_usuario() {
        let a = (41.4300, 2.1800);
        let b = (41.4300 + 3.4 / 111.0, 2.1800);
        assert_eq!(calc_sub_blocks(a, b, false), 3);
        assert_eq!(calc_sub_blocks(a, b, true), 1);
    }

    #[test]
    fn roll_de_incidencias_determinista() {
        let cantons = vec![
            ("A".to_string(), "B".to_string(), 1.0),
            ("C".to_string(), "D".to_string(), 3.0),
        ];
        let tpl = default_templates();
        let (s, e) = (5.0 * 3600.0, 24.0 * 3600.0);
        let a = roll_incidents(&cantons, &tpl, s, e, 42);
        let b = roll_incidents(&cantons, &tpl, s, e, 42);
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!((&x.from, &x.to), (&y.from, &y.to));
            assert!((x.t0 - y.t0).abs() < 1e-9);
        }
    }

    /// Tasa "cada X horas": con prob 1 cada 4 h en una jornada de 19 h → round(4,75) = 5 secciones
    /// iguales de 3,8 h → exactamente 5 instancias, cada una DENTRO de su sección.
    #[test]
    fn roll_por_secciones_de_x_horas() {
        let cantons = vec![("A".to_string(), "B".to_string(), 1.0)];
        let tpl = vec![IncidentTemplate {
            motivo: "t".into(),
            effect: IncidentEffect::RedSignal { both: false },
            dur_min: 10.0,
            prob: 1.0,
            every_h: 4.0,
            enabled: true,
        }];
        let (s, e) = (5.0 * 3600.0, 24.0 * 3600.0);
        for seed in 0..20 {
            let r = roll_incidents(&cantons, &tpl, s, e, seed);
            assert_eq!(r.len(), 5, "seed {seed}");
            let sec = (e - s) / 5.0;
            for (k, inc) in r.iter().enumerate() {
                let s0 = s + k as f64 * sec;
                assert!(inc.t0 >= s0 && inc.t0 < s0 + sec);
            }
        }
        // "cada 24 h" en una jornada más corta = 1 sola tirada (la antigua probabilidad diaria).
        assert_eq!(n_sections(e - s, 24.0), 1);
        assert_eq!(n_sections(e - s, 100.0), 1);
        // prob 0 → nunca.
        let mut t0 = tpl.clone();
        t0[0].prob = 0.0;
        assert!(roll_incidents(&cantons, &t0, s, e, 7).is_empty());
    }

    /// `Infra` mínima para probar la resolución de efectos a índices: A=0,B=1,C=2,D=3; C|D tiene
    /// recurso canónico 0 (para el efecto de vía única).
    fn infra_para_test() -> Infra {
        let stop_index: HashMap<String, u32> =
            [("A", 0), ("B", 1), ("C", 2), ("D", 3)].iter().map(|(s, i)| (s.to_string(), *i)).collect();
        let mut res_canon = HashMap::new();
        res_canon.insert((2u32, 3u32), 0u32); // canon(C,D) → recurso 0
        Infra {
            stop_index,
            sub_blocks: HashMap::new(),
            single_track: HashSet::new(),
            terminals: HashSet::new(),
            edges_from: HashMap::new(),
            single_track_users: HashMap::new(),
            res_dir: HashMap::new(),
            res_canon,
            n_res: 1,
        }
    }

    #[test]
    fn dyn_effects_aplica_rojo_y_via_unica_en_ventana() {
        let infra = infra_para_test();
        let incs = vec![
            ActiveIncident {
                from: "A".into(), to: "B".into(),
                effect: IncidentEffect::RedSignal { both: true },
                t0: 100.0, t1: 200.0, motivo: "x".into(),
            },
            ActiveIncident {
                from: "C".into(), to: "D".into(),
                effect: IncidentEffect::SingleTrack,
                t0: 100.0, t1: 200.0, motivo: "y".into(),
            },
        ];
        let (red, single) = dyn_effects(&infra, &incs, 150.0);
        assert!(red.contains(&(0, 1)) && red.contains(&(1, 0))); // ambos sentidos (A↔B)
        assert!(single.contains(&0)); // recurso canónico de C|D
        let (red2, single2) = dyn_effects(&infra, &incs, 50.0); // fuera de ventana
        assert!(red2.is_empty() && single2.is_empty());
    }

    #[test]
    fn factor_de_velocidad_gaussiana_truncada() {
        let mut rng = StdRng::seed_from_u64(7);
        let p0 = 0.03;
        let n = 20_000;
        let zs: Vec<f64> = (0..n).map(|_| trunc_std_normal(&mut rng, SPEED_VAR_TRUNC_Z)).collect();
        let stats = |v_kmh: f64| {
            let xs: Vec<f64> = zs.iter().map(|&z| speed_factor(p0, v_kmh, z)).collect();
            let mean = xs.iter().sum::<f64>() / n as f64;
            let sd = (xs.iter().map(|f| (f - mean).powi(2)).sum::<f64>() / n as f64).sqrt();
            let lo = xs.iter().cloned().fold(f64::INFINITY, f64::min);
            let hi = xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            (mean, sd, lo, hi)
        };
        // A 120 km/h (referencia): media 1−p, truncado simétrico a [1−2p, 1], dispersión real.
        let (mean, sd, lo, hi) = stats(120.0);
        assert!((mean - (1.0 - p0)).abs() < 0.001, "media {mean}");
        assert!(lo >= 1.0 - 2.0 * p0 - 1e-12 && hi <= 1.0 + 1e-12);
        assert!(sd > 0.3 * p0 && sd < 0.5 * p0, "σ {sd}");
        // A 60 km/h: p(V) = p/2 → más cerca de Vmax y campana más estrecha; a 140, al revés.
        let (m60, sd60, lo60, _) = stats(60.0);
        assert!((m60 - (1.0 - p0 / 2.0)).abs() < 0.001 && sd60 < sd && lo60 >= 1.0 - p0 - 1e-12);
        let (m140, sd140, _, _) = stats(140.0);
        assert!(m140 < mean && sd140 > sd);
        // p0 = 0 → sin variabilidad (invariante: todos a Vmax).
        assert_eq!(speed_factor(0.0, 120.0, -2.0), 1.0);
    }

    #[test]
    fn sin_horarios_solo_la_cabecera_espera_su_hora() {
        // Con horario: nunca sale antes de su hora, en ninguna parada.
        assert!(debe_esperar_horario(false, 0, 100.0, 200.0));
        assert!(debe_esperar_horario(false, 3, 100.0, 200.0));
        assert!(!debe_esperar_horario(false, 3, 250.0, 200.0));
        // Sin horarios: la cabecera conserva su hora de salida; el resto sale tras la parada mínima.
        assert!(debe_esperar_horario(true, 0, 100.0, 200.0));
        assert!(!debe_esperar_horario(true, 3, 100.0, 200.0));
    }

    #[test]
    fn retencion_se_acumula_en_el_tramo_vetado() {
        let blk = |from: u32, to: u32, sb: usize| Block {
            origin: false, from, to, sb, last: false, dest: to, arr_idx: 1, x_enter: 0.0,
            res: 0, res_single: 0, stat_single: false, cap_double: 1, plat_tracks: 1,
        };
        let mut t = Train {
            id: "T1".into(), line: "R2".into(), regional: false, veh: vehicle_for_line("R2"),
            pts: Vec::new(), cum_l: Vec::new(), canton_zones: Vec::new(), speed_p0: 0.0,
            speed_z: Vec::new(), dwell_min: Vec::new(), seq: vec![blk(0, 1, 0), blk(1, 2, 0)],
            phase: Phase::Running, station_idx: 0, canton_idx: 0, target_stop: 1, arrived_at: 0.0,
            s: 0.0, v: 0.0, cb: 0, delay_s: 0.0, waiting: true, held_since: 100.0, was_waiting: false,
        };
        let mut h = HashMap::new();
        acumular_retencion(&mut h, &t, 102.0, 2.0); // 1er paso retenido: nueva retención
        t.was_waiting = true;
        acumular_retencion(&mut h, &t, 104.0, 2.0); // sigue retenido: misma retención
        let acc = &h[&(1, 2)]; // el tramo vetado es el SIGUIENTE bloque (cb+1), no el actual
        assert_eq!(acc.secs, 4.0);
        assert_eq!(acc.events, 1);
        assert_eq!(acc.max_s, 4.0);
        assert_eq!(acc.by_line["R2"], 4.0);
    }

    /// Tren mínimo para probar reglas de ocupación (solo importan fase, bloques y paradas).
    fn tren_prueba(phase: Phase, seq: Vec<Block>, pts: Vec<Pt>) -> Train {
        Train {
            id: "T".into(), line: "R15".into(), regional: true, veh: vehicle_for_line("R15"),
            pts, cum_l: Vec::new(), canton_zones: Vec::new(), speed_p0: 0.0, speed_z: Vec::new(),
            dwell_min: Vec::new(), seq, phase, station_idx: 0, canton_idx: 0, target_stop: 0,
            arrived_at: 0.0, s: 0.0, v: 0.0, cb: 0, delay_s: 0.0, waiting: false,
            held_since: f64::NAN, was_waiting: false,
        }
    }
    fn bloque(from: u32, to: u32, arr_idx: usize, single: bool, res: u32) -> Block {
        Block {
            origin: false, from, to, sb: 0, last: true, dest: to, arr_idx, x_enter: 0.0,
            res, res_single: res, stat_single: single, cap_double: 1, plat_tracks: 1,
        }
    }

    #[test]
    fn anden_con_dos_vias_solo_se_cierra_lleno_o_con_tren_entrando() {
        let trains = vec![
            tren_prueba(Phase::Dwell, vec![], vec![]),
            tren_prueba(Phase::Dwell, vec![], vec![]),
            tren_prueba(Phase::Running, vec![], vec![]),
        ];
        assert!(!anden_lleno(&[0], &trains, None, 2)); // uno parado: la otra vía sigue libre
        assert!(anden_lleno(&[0, 1], &trains, None, 2)); // las dos vías del acceso ocupadas
        assert!(!anden_lleno(&[0, 1], &trains, Some(1), 2)); // el propio tren no cuenta
        assert!(anden_lleno(&[2], &trains, None, 2)); // un tren circulando por la aproximación
    }

    #[test]
    fn cadena_de_via_unica_hasta_la_proxima_parada() {
        let pt = |id: &str, pass: bool| Pt { id: id.into(), a: 0.0, d: 0.0, lat: 0.0, lon: 0.0, pass };
        // A →(único) B (no para) →(único) C (para) →(único) D
        let pts = vec![pt("A", false), pt("B", true), pt("C", false), pt("D", false)];
        let mut origin = bloque(0, 0, 0, false, 0);
        origin.origin = true;
        let seq = vec![origin, bloque(0, 1, 1, true, 10), bloque(1, 2, 2, true, 11), bloque(2, 3, 3, true, 12)];
        let t = tren_prueba(Phase::Running, seq, pts);
        let (red, single) = (HashSet::new(), HashSet::new());
        let dynp = Dyn { red: &red, single: &single };
        let mut out = Vec::new();
        cadena_unica(&t, &dynp, 1, &mut out);
        assert_eq!(out, vec![11]); // reserva B→C (cruza B sin parar), no C→D (para en C)
        cadena_unica(&t, &dynp, 2, &mut out);
        assert!(out.is_empty()); // desde B→C la siguiente estación (C) es parada
    }

    #[test]
    fn next_real_stop_salta_puntos_de_paso() {
        let pts = vec![
            Pt { id: "A".into(), a: 0.0, d: 0.0, lat: 0.0, lon: 0.0, pass: false },
            Pt { id: "P".into(), a: 60.0, d: 60.0, lat: 0.0, lon: 0.0, pass: true },
            Pt { id: "B".into(), a: 120.0, d: 120.0, lat: 0.0, lon: 0.0, pass: false },
        ];
        assert_eq!(next_real_stop(&pts, 0), 2); // salta el punto de paso P
    }
}
