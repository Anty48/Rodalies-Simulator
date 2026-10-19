//! **Trenes de mercancías**: escenarios, generador semialeatorio y construcción de sus recorridos.
//!
//! No hay datos abiertos de surcos de mercancías (ADIF no los publica), así que se modelan por
//! **corredores** reales con una cadencia diaria configurable. Cada corredor une dos estaciones
//! con terminal o playa de mercancías (Castellbisbal, Tarragona/puerto, Portbou/frontera…): los
//! trenes **aparecen y desaparecen siempre en esos mismos sitios**. El recorrido se enruta sobre la
//! vía que recorre Rodalies (grafo atómico del GTFS, `game::AtomicRouter`), así comparten cantones,
//! señales y vía única con los trenes de viajeros.
//!
//! Escenarios guardados en `report/freight.json` (página `/mercancias`): «generados» (por corredor,
//! cambian con la semilla) o «fijos» (lista de trenes con su hora: un día generado que se guardó o
//! trenes puestos a mano). Se elige el escenario antes de empezar la run (`frt=<id>`).

use crate::game::{insert_junctions, AtomicRouter, GameStop, GameTrain};
use crate::gtfs_loader::Network;
use rand::{rngs::StdRng, Rng, SeedableRng};
use serde::{Deserialize, Serialize};

pub const FREIGHT_PATH: &str = "report/freight.json";
/// Línea con la que aparecen los mercancías en el trace/visor.
pub const FREIGHT_LINE: &str = "MERC";
/// Sal del flujo aleatorio del generador (independiente de incidencias y velocidades).
const FREIGHT_SALT: u64 = 0xF4E1_6475;

/// Parámetros físicos de un tren de mercancías tipo.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct FreightStock {
    pub vmax_kmh: f64,
    pub length_m: f64,
    pub mass_t: f64,
    pub power_kw: f64,
}

impl Default for FreightStock {
    /// Tren tipo de contenedores/automóviles en ancho ibérico: 100 km/h, ~500 m, ~1.200 t, una
    /// locomotora 253 (5.600 kW). Valores típicos de la red convencional (aproximados).
    fn default() -> Self {
        FreightStock { vmax_kmh: 100.0, length_m: 500.0, mass_t: 1200.0, power_kw: 5600.0 }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct FreightCorridor {
    pub id: String,
    pub name: String,
    /// stop_id (o nombre) de origen y destino: las terminales donde nacen y mueren los trenes.
    pub from: String,
    pub to: String,
    /// Estaciones de paso obligadas (para fijar el itinerario real del corredor).
    pub via: Vec<String>,
    /// Trenes por día en CADA sentido (media; el número real del día se sortea).
    pub per_day: f64,
    pub both_ways: bool,
    pub stock: FreightStock,
    pub enabled: bool,
}

impl Default for FreightCorridor {
    fn default() -> Self {
        FreightCorridor {
            id: String::new(),
            name: String::new(),
            from: String::new(),
            to: String::new(),
            via: Vec::new(),
            per_day: 2.0,
            both_ways: true,
            stock: FreightStock::default(),
            enabled: true,
        }
    }
}

/// Un tren concreto con su hora (escenario fijo).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct FixedFreight {
    pub name: String,
    pub from: String,
    pub to: String,
    pub via: Vec<String>,
    /// Hora de salida "HH:MM".
    pub dep: String,
    pub stock: FreightStock,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct FreightScenario {
    pub id: String,
    pub name: String,
    /// "generated" (corredores, depende de la semilla) o "fixed" (lista de trenes).
    pub kind: String,
    pub corridors: Vec<FreightCorridor>,
    pub trains: Vec<FixedFreight>,
    /// Evitar las horas punta de viajeros (como hace ADIF en el nudo de Barcelona).
    pub avoid_peak: bool,
    pub note: String,
}

impl Default for FreightScenario {
    fn default() -> Self {
        FreightScenario {
            id: String::new(),
            name: String::new(),
            kind: "generated".into(),
            corridors: Vec::new(),
            trains: Vec::new(),
            avoid_peak: true,
            note: String::new(),
        }
    }
}

/// Escenario por defecto: corredores semirrealistas de la red convencional catalana.
pub fn default_scenarios() -> Vec<FreightScenario> {
    let c = |id: &str, name: &str, from: &str, to: &str, via: &[&str], per_day: f64| FreightCorridor {
        id: id.into(),
        name: name.into(),
        from: from.into(),
        to: to.into(),
        via: via.iter().map(|s| s.to_string()).collect(),
        per_day,
        ..Default::default()
    };
    vec![FreightScenario {
        id: "semireal".into(),
        name: "Mercancías semirrealistas".into(),
        kind: "generated".into(),
        corridors: vec![
            // Corredor Mediterráneo por el Penedès (ruta clásica Tarragona → Castellbisbal → Europa):
            // comparte vía con la R4 entre Sant Vicenç y Castellbisbal.
            c("med", "Corredor Mediterráneo (Tarragona ↔ Castellbisbal)", "71500", "72210", &["71600", "72204", "72209"], 6.0),
            // Castellbisbal ↔ frontera por Mollet/Granollers/Girona (R8 + R11).
            c("fra", "Castellbisbal ↔ Portbou (frontera)", "72210", "79315", &["79100", "79300"], 3.0),
            // Acceso al Port de Barcelona por el Llobregat (L'Hospitalet como puerta de Can Tunis).
            c("port", "Castellbisbal ↔ L'Hospitalet (Port de Barcelona)", "72210", "72305", &["72211", "72300"], 3.0),
            // Automóviles de SEAT Martorell hacia el puerto.
            c("seat", "Martorell (SEAT) ↔ L'Hospitalet (Port)", "72209", "72305", &["72211", "72300"], 2.0),
            // Lleida (Aragón) ↔ Castellbisbal: TODO lo que viene de Lleida baja por Montblanc y Valls a
            // Sant Vicenç de Calders y sube por el corredor del Penedès (nunca por Manresa–Terrassa–
            // Sabadell). El tramo Picamoixons–Les Borges no tiene trenes de viajeros en el GTFS:
            // ver `topology::FREIGHT_ONLY_TRACK`.
            c("lle", "Lleida ↔ Castellbisbal (por Montblanc y Sant Vicenç)", "78400", "72210", &["73008", "71600", "72204"], 2.0),
        ],
        trains: Vec::new(),
        avoid_peak: true,
        note: "Cadencias aproximadas (no hay surcos publicados). Ajustables por corredor.".into(),
    }]
}

pub fn load_scenarios() -> Vec<FreightScenario> {
    std::fs::read_to_string(FREIGHT_PATH)
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<FreightScenario>>(&s).ok())
        .unwrap_or_else(default_scenarios)
}

pub fn save_scenarios(v: &[FreightScenario]) -> bool {
    std::fs::create_dir_all("report").ok();
    std::fs::write(FREIGHT_PATH, serde_json::to_string_pretty(v).unwrap_or_default()).is_ok()
}

/// Un tren de mercancías listo para el motor: su recorrido (mismo formato que los de viajeros) y
/// su material.
#[derive(Clone)]
pub struct FreightTrain {
    pub gt: GameTrain,
    pub stock: FreightStock,
    pub corridor: String,
}

/// Peso horario de circulación de mercancías (0–24 h): sobre todo de noche y en horas valle.
fn hour_weight(h: f64, avoid_peak: bool) -> f64 {
    let peak = (6.5..9.5).contains(&h) || (17.0..20.0).contains(&h);
    if peak {
        return if avoid_peak { 0.05 } else { 0.4 };
    }
    if !(5.0..22.0).contains(&h) {
        1.0
    } else if (20.0..22.0).contains(&h) || (5.0..6.5).contains(&h) {
        0.8
    } else {
        0.55
    }
}

/// Resuelve una referencia de estación (stop_id exacto, nombre exacto o contenido) a stop_id.
pub fn resolve_stop(net: &Network, s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if net.node(s).is_some() {
        return Some(s.to_string());
    }
    let low = s.to_lowercase();
    net.graph
        .node_weights()
        .find(|n| n.stop_name.to_lowercase() == low)
        .or_else(|| net.find_stop_by_name(s))
        .map(|n| n.stop_id.clone())
}

/// Poisson(λ) por el método de Knuth (λ pequeños: trenes/día de un corredor).
fn poisson(rng: &mut StdRng, lambda: f64) -> usize {
    if lambda <= 0.0 {
        return 0;
    }
    let l = (-lambda).exp();
    let (mut k, mut p) = (0usize, 1.0f64);
    loop {
        p *= rng.gen::<f64>();
        if p <= l {
            return k;
        }
        k += 1;
    }
}

/// Muestra una hora de salida en `[start, end)` según el peso horario.
fn sample_dep(rng: &mut StdRng, start: f64, end: f64, avoid_peak: bool) -> f64 {
    let bins = (((end - start) / 600.0).ceil() as usize).max(1); // bins de 10 min
    let w = (end - start) / bins as f64;
    let ws: Vec<f64> = (0..bins).map(|i| hour_weight(((start + (i as f64 + 0.5) * w) / 3600.0) % 24.0, avoid_peak)).collect();
    let tot: f64 = ws.iter().sum::<f64>().max(1e-9);
    let mut r = rng.gen::<f64>() * tot;
    for (i, x) in ws.iter().enumerate() {
        if r < *x {
            return start + (i as f64 + rng.gen::<f64>()) * w;
        }
        r -= x;
    }
    start
}

/// Fracción del peso horario diario que cae en `[start, end)` (para escalar `per_day`).
fn window_fraction(start: f64, end: f64, avoid_peak: bool) -> f64 {
    let day: f64 = (0..144).map(|i| hour_weight((i as f64 + 0.5) / 6.0, avoid_peak)).sum();
    let bins = (((end - start) / 600.0).ceil() as usize).max(1);
    let w = (end - start) / bins as f64;
    let win: f64 = (0..bins)
        .map(|i| hour_weight(((start + (i as f64 + 0.5) * w) / 3600.0) % 24.0, avoid_peak) * (w / 600.0))
        .sum();
    (win / day.max(1e-9)).clamp(0.0, 1.0)
}

/// Lista de trenes (origen, destino, vía, hora, material, corredor) del escenario para una
/// jornada `[start, end)`. Los generados dependen de `seed`; los fijos no.
pub fn plan(sc: &FreightScenario, seed: u64, start: f64, end: f64) -> Vec<FixedFreight> {
    if sc.kind == "fixed" {
        return sc.trains.clone();
    }
    let mut rng = StdRng::seed_from_u64(seed ^ FREIGHT_SALT);
    let frac = window_fraction(start, end, sc.avoid_peak);
    let mut out = Vec::new();
    for c in sc.corridors.iter().filter(|c| c.enabled) {
        let dirs: &[bool] = if c.both_ways { &[false, true] } else { &[false] };
        for &rev in dirs {
            let n = poisson(&mut rng, c.per_day * frac);
            for _ in 0..n {
                let t = sample_dep(&mut rng, start, end, sc.avoid_peak);
                let (from, to, via) = if rev {
                    (c.to.clone(), c.from.clone(), c.via.iter().rev().cloned().collect())
                } else {
                    (c.from.clone(), c.to.clone(), c.via.clone())
                };
                out.push(FixedFreight { name: c.name.clone(), from, to, via, dep: hhmm(t), stock: c.stock.clone() });
            }
        }
    }
    out.sort_by(|a, b| a.dep.cmp(&b.dep));
    out
}

pub fn hhmm(t: f64) -> String {
    let t = t.max(0.0) as u32;
    format!("{:02}:{:02}", t / 3600, (t % 3600) / 60)
}

fn parse_hhmm(s: &str) -> Option<u32> {
    let mut it = s.trim().split(':');
    let h: u32 = it.next()?.trim().parse().ok()?;
    let m: u32 = it.next()?.trim().parse().ok()?;
    Some(h * 3600 + m * 60)
}

/// Construye los recorridos de los trenes planificados. Los que no se pueden enrutar (estación
/// desconocida o sin vía común) se descartan y se devuelven en el segundo vector con el motivo.
pub fn build_trains(net: &Network, plan: &[FixedFreight]) -> (Vec<FreightTrain>, Vec<String>) {
    let mut out = Vec::new();
    let mut errors = Vec::new();
    if plan.is_empty() {
        return (out, errors);
    }
    let router = AtomicRouter::new(net);
    for (n, f) in plan.iter().enumerate() {
        let Some(dep) = parse_hhmm(&f.dep) else {
            errors.push(format!("{}: hora inválida «{}»", f.name, f.dep));
            continue;
        };
        let mut wp: Vec<String> = Vec::new();
        let mut bad = None;
        for s in std::iter::once(&f.from).chain(f.via.iter()).chain(std::iter::once(&f.to)) {
            match resolve_stop(net, s) {
                Some(id) => wp.push(id),
                None => bad = Some(s.clone()),
            }
        }
        if let Some(b) = bad {
            errors.push(format!("{}: estación desconocida «{b}»", f.name));
            continue;
        }
        let mut path: Vec<String> = vec![wp[0].clone()];
        let mut ok = true;
        for w in wp.windows(2) {
            match router.route(&w[0], &w[1]) {
                Some(p) => path.extend(p.into_iter().skip(1)),
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok || path.len() < 2 {
            errors.push(format!("{}: sin vía común entre {} y {}", f.name, net.stop_name(&wp[0]), net.stop_name(wp.last().unwrap())));
            continue;
        }
        // Horario nominal ("surco") a ~65 % de la Vmax: solo sirve para ordenar y para el visor; el
        // mercancías circula por física y no espera horas en ruta (solo la salida de origen).
        let v_nom = (f.stock.vmax_kmh / 3.6 * 0.65).max(5.0);
        let mut t = dep as f64;
        let mut stops = Vec::with_capacity(path.len());
        let last = path.len() - 1;
        for (i, id) in path.iter().enumerate() {
            if i > 0 {
                let d = match (router.coord(&path[i - 1]), router.coord(id)) {
                    (Some(a), Some(b)) => crate::calculator::infrastructure::haversine_m(a.0, a.1, b.0, b.1),
                    _ => 1000.0,
                };
                t += d / v_nom;
            }
            let ts = t.round() as u32;
            stops.push(GameStop { s: id.clone(), a: ts, d: ts, pass: i != 0 && i != last });
        }
        let stops = insert_junctions(stops);
        out.push(FreightTrain {
            gt: GameTrain {
                train: format!("M{}", 90001 + n),
                line: FREIGHT_LINE.to_string(),
                from: path[0].clone(),
                to: path[last].clone(),
                offset_min: 0,
                stops,
            },
            stock: f.stock.clone(),
            corridor: f.name.clone(),
        });
    }
    (out, errors)
}

/// Escenario por id (`none`/vacío = sin mercancías).
pub fn scenario_by_id(id: &str) -> Option<FreightScenario> {
    if id.is_empty() || id == "none" {
        return None;
    }
    load_scenarios().into_iter().find(|s| s.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generador_determinista_y_evita_puntas() {
        let sc = &default_scenarios()[0];
        let a = plan(sc, 7, 5.0 * 3600.0, 24.0 * 3600.0);
        let b = plan(sc, 7, 5.0 * 3600.0, 24.0 * 3600.0);
        assert_eq!(a.len(), b.len());
        assert!(a.iter().zip(&b).all(|(x, y)| x.dep == y.dep && x.from == y.from));
        assert!(!a.is_empty(), "un día típico debe tener mercancías");
        // Mucho menos tráfico en punta que fuera de ella.
        let peak = a.iter().filter(|f| {
            let t = parse_hhmm(&f.dep).unwrap() as f64 / 3600.0;
            (7.0..9.0).contains(&t)
        }).count();
        assert!(peak * 4 <= a.len());
    }

    #[test]
    fn hora_ida_y_vuelta() {
        assert_eq!(parse_hhmm("07:35"), Some(7 * 3600 + 35 * 60));
        assert_eq!(hhmm(7.0 * 3600.0 + 35.0 * 60.0), "07:35");
    }
}
