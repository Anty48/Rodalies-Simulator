//! Horarios CADENCIADOS: el creador de la web (`/horarios`) coloca, en relojes de una estación de
//! referencia, a qué minuto de cada hora pasa un tren de cada recorrido; aquí se convierte esa
//! especificación en una malla de trenes (`Vec<GameTrain>`) que el motor simula igual que el GTFS.
//!
//! Los recorridos NO se inventan: cada patrón origen→destino sale del GTFS (su trayecto real y sus
//! tiempos de marcha), y la malla cadenciada solo DESPLAZA ese viaje tipo en el tiempo. Así la
//! física y la topología siguen siendo las reales; solo cambia cuándo sale cada tren.
//!
//! Reglas (decisiones del usuario, 2026-10-14):
//! * Referencia por línea: Barcelona-Sants para casi todas; R7/R8 en Cerdanyola Universitat; R3 en
//!   La Garriga (hoy no pasa por Sants); RT en Tarragona; RL en Lleida. Si un recorrido no pasa por
//!   su primera referencia se edita en la siguiente que toque (R13 La Plana–Lleida → Lleida). En el
//!   resto de referencias por las que pasa se MUESTRA calculado, sin editar.
//! * El origen va ligado al destino: se elige el recorrido entero (patrón origen→destino del GTFS).
//!   Se descartan los viajes puntuales y los que acaban en una estación que la línea casi siempre
//!   atraviesa (vueltas cortas en L'Hospitalet, Montcada-Bifurcació…).
//! * El sistema abre a las 04:30: el primer tren de cada marcador es el que pasa por la referencia
//!   en el primer minuto marcado cuya salida de cabecera no sea anterior a la apertura.

use crate::game::{GameStop, GameTrain};
use crate::gtfs_loader::Network;
use crate::topology;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// Apertura del sistema en el creador: ningún tren sale de cabecera antes (s desde medianoche).
pub const OPEN_SEC: u32 = 4 * 3600 + 30 * 60;
/// Última salida de cabecera de la malla cadenciada. El motor corta la jornada a las 24:00, así que
/// un tren que saliera más tarde apenas circularía; 23:30 deja acabar los recorridos cortos.
pub const LAST_DEP_SEC: u32 = 23 * 3600 + 30 * 60;
/// Un patrón origen→destino con menos viajes al día es un tren puntual: no se ofrece.
const MIN_TRIPS: usize = 2;
/// Un extremo de recorrido que la línea ATRAVIESA ≥ este factor × las veces que acaba allí es una
/// vuelta corta ocasional (L'Hospitalet en la R4, Montcada-Bifurcació…): se descarta el patrón.
const SHORT_TURN_RATIO: usize = 4;
/// Hora del viaje tipo: de los viajes de un patrón se toma el más cercano al mediodía (marchas de
/// hora valle, sin las holguras de punta).
const TEMPLATE_HOUR_SEC: i64 = 12 * 3600;

/// Estación de referencia (donde se colocan los relojes). `stop_id` del GTFS de Renfe/Adif.
struct RefDef {
    key: &'static str,
    stop_id: &'static str,
}
const REFS: &[RefDef] = &[
    RefDef { key: "sants", stop_id: "71801" },
    RefDef { key: "cerdanyola", stop_id: "72503" },
    RefDef { key: "garriga", stop_id: "77102" },
    RefDef { key: "tarragona", stop_id: "71500" },
    RefDef { key: "lleida", stop_id: "78400" },
];

/// Orden de referencias en que se EDITA cada línea (la primera por la que pasa el recorrido).
fn ref_priority(line: &str) -> &'static [&'static str] {
    match line {
        "R7" | "R8" => &["cerdanyola"],
        "R3" | "R3a" => &["garriga"],
        l if l.starts_with("RT") => &["tarragona"],
        l if l.starts_with("RL") => &["lleida"],
        _ => &["sants", "tarragona", "lleida"],
    }
}

/// Conjuntos de VÍAS PRINCIPALES por estación de referencia (el GTFS no lo trae). En Sants es el
/// mismo reparto que los corredores del juego (`CORREDORES` de app.js): la R2 y los regionales
/// comparten un par de vías y la R1/R4 otro. Cada conjunto lleva dos relojes (uno por sentido).
const TRACK_GROUPS: &[(&str, &str, &[&str])] = &[
    ("sants", "Vías R2 · regionales", &["R2", "R2N", "R2S", "R11", "R13", "R14", "R15", "R16", "R17"]),
    ("sants", "Vías R1 · R4", &["R1", "R4"]),
    ("cerdanyola", "Vías R7", &["R7"]),
    ("cerdanyola", "Vías R8", &["R8"]),
];
const DEFAULT_GROUP: &str = "Vías principales";

// ---- Catálogo (lo que el creador puede colocar) -----------------------------------------------

#[derive(Serialize, Clone)]
pub struct DirOut {
    /// Siguiente parada más habitual tras la referencia en este sentido (el rótulo del reloj es
    /// «<referencia> dirección <label>»). Vacío si en este sentido los trenes acaban en la referencia.
    pub label: String,
    /// `true` = la mayoría de trenes de este reloj ACABAN en la referencia (llegadas).
    pub terminal: bool,
}
#[derive(Serialize, Clone)]
pub struct GroupOut {
    pub label: String,
    pub dirs: Vec<DirOut>, // siempre 2 (sentido 0 / 1)
}
#[derive(Serialize, Clone)]
pub struct RefOut {
    pub key: String,
    pub stop_id: String,
    pub name: String,
    pub groups: Vec<GroupOut>,
}
/// Otra referencia por la que pasa el recorrido: se pinta calculada (sin editar).
#[derive(Serialize, Clone)]
pub struct AlsoOut {
    pub ref_key: String,
    pub group: usize,
    pub dir: usize,
    /// Segundos desde la salida de cabecera hasta el paso por esa referencia.
    pub off: u32,
}
#[derive(Serialize, Clone)]
pub struct PatternOut {
    pub id: String,
    pub line: String,
    pub color: String,
    pub regional: bool,
    pub from: String,
    pub to: String,
    pub from_name: String,
    pub to_name: String,
    /// Viajes al día de este recorrido en el GTFS (orienta qué es habitual).
    pub n: usize,
    pub ref_key: String,
    pub group: usize,
    pub dir: usize,
    /// Segundos desde la salida de cabecera hasta el paso por la referencia.
    pub ref_off: u32,
    /// Duración total del recorrido (s).
    pub dur: u32,
    pub also: Vec<AlsoOut>,
}
#[derive(Serialize, Clone)]
pub struct Catalog {
    pub open: u32,
    pub last_dep: u32,
    pub refs: Vec<RefOut>,
    pub patterns: Vec<PatternOut>,
}

/// Catálogo + viajes tipo (privados) para generar la malla.
pub struct CadenceCatalog {
    pub out: Catalog,
    templates: HashMap<String, (GameTrain, u32)>, // id patrón → (viaje tipo, ref_off)
}

/// Ángulo (rad) del sentido de marcha en `k`: de la parada anterior a la siguiente (o desde/hasta
/// la propia `k` si es extremo). Plano local con corrección de coseno de la latitud.
fn heading(net: &Network, stops: &[GameStop], k: usize) -> Option<f64> {
    let a = net.coord(&stops[k.saturating_sub(1)].s)?;
    let b = net.coord(&stops[(k + 1).min(stops.len() - 1)].s)?;
    let dx = (b.1 - a.1) * a.0.to_radians().cos();
    let dy = b.0 - a.0;
    if dx == 0.0 && dy == 0.0 {
        return None;
    }
    Some(dy.atan2(dx))
}

/// Eje principal de un haz de sentidos de marcha (media AXIAL: ángulos doblados, así ida y vuelta
/// suman en vez de anularse), normalizado a (−90°, 90°] → el sentido 0 apunta hacia el este/norte.
fn axis_of(angles: &[f64]) -> f64 {
    let (s, c) = angles.iter().fold((0.0, 0.0), |(s, c), a| (s + (2.0 * a).sin(), c + (2.0 * a).cos()));
    let mut ax = s.atan2(c) / 2.0; // (−90°, 90°]
    if ax <= -std::f64::consts::FRAC_PI_2 {
        ax += std::f64::consts::PI;
    }
    ax
}

/// Sentido (0/1) de un rumbo respecto al eje de la estación.
fn dir_of(angle: f64, axis: f64) -> usize {
    if (angle - axis).cos() >= 0.0 { 0 } else { 1 }
}

/// ¿Es el patrón una vuelta corta ocasional? (`pass` = veces que la línea atraviesa el extremo,
/// `term` = veces que acaba/empieza allí). Una línea que casi siempre sigue de largo no tiene ahí
/// su cabecera natural.
fn is_short_turn(pass: usize, term: usize) -> bool {
    pass >= SHORT_TURN_RATIO * term.max(1)
}

fn group_of(ref_key: &str, line: &str) -> (usize, &'static str) {
    let mut idx = 0;
    for &(r, label, lines) in TRACK_GROUPS {
        if r != ref_key {
            continue;
        }
        if lines.contains(&line) {
            return (idx, label);
        }
        idx += 1;
    }
    (idx, DEFAULT_GROUP) // tras los grupos con nombre (o el único, si la estación no tiene)
}

pub fn build_catalog(net: &Network) -> CadenceCatalog {
    let trains = crate::game::build_schedule(net, false, None);

    // Recuento por línea: veces que cada parada es extremo de recorrido y veces que se atraviesa.
    let mut term: HashMap<(&str, &str), usize> = HashMap::new();
    let mut pass: HashMap<(&str, &str), usize> = HashMap::new();
    let mut groups: BTreeMap<(&str, &str, &str), Vec<&GameTrain>> = BTreeMap::new();
    for t in &trains {
        *term.entry((&t.line, &t.from)).or_default() += 1;
        *term.entry((&t.line, &t.to)).or_default() += 1;
        if t.stops.len() > 2 {
            for s in &t.stops[1..t.stops.len() - 1] {
                *pass.entry((&t.line, &s.s)).or_default() += 1;
            }
        }
        if t.from != t.to {
            groups.entry((&t.line, &t.from, &t.to)).or_default().push(t);
        }
    }

    struct Raw {
        out: PatternOut,
        angle: Option<f64>,
        next_name: Option<String>,
        also_angles: Vec<(usize, Option<f64>)>, // índice en `also` → rumbo
        tpl: GameTrain,
    }
    let mut raws: Vec<Raw> = Vec::new();
    for ((line, from, to), trips) in &groups {
        if trips.len() < MIN_TRIPS {
            continue;
        }
        let ends_ok = [from, to].iter().all(|e| {
            let p = pass.get(&(*line, **e)).copied().unwrap_or(0);
            let tc = term.get(&(*line, **e)).copied().unwrap_or(0);
            !is_short_turn(p, tc)
        });
        if !ends_ok {
            continue;
        }
        // Viaje tipo: la secuencia de paradas más común (exprés y ómnibus son recorridos distintos
        // en tiempo) y, de esa, el viaje más cercano al mediodía. Desempates deterministas.
        let mut seqs: BTreeMap<Vec<&str>, Vec<&GameTrain>> = BTreeMap::new();
        for t in trips {
            let key: Vec<&str> = t.stops.iter().filter(|s| !s.pass).map(|s| s.s.as_str()).collect();
            seqs.entry(key).or_default().push(t);
        }
        let best = seqs.values().max_by_key(|v| v.len()).expect("grupo no vacío");
        let tpl = *best
            .iter()
            .min_by_key(|t| (t.stops[0].d as i64 - TEMPLATE_HOUR_SEC).abs())
            .expect("secuencia no vacía");
        let dep0 = tpl.stops[0].d;
        let last = tpl.stops.len() - 1;
        let time_at = |k: usize| if k == last { tpl.stops[k].a } else { tpl.stops[k].d };

        // Referencia de edición: la primera de su lista por la que pasa.
        let Some((rkey, k)) = ref_priority(line).iter().find_map(|rk| {
            let sid = REFS.iter().find(|r| r.key == *rk)?.stop_id;
            tpl.stops.iter().position(|s| s.s == sid).map(|k| (*rk, k))
        }) else {
            continue;
        };
        let (group, _) = group_of(rkey, line);
        let next_name = tpl.stops[k + 1..].iter().find(|s| !s.pass).map(|s| net.stop_name(&s.s).to_string());
        let mut also = Vec::new();
        let mut also_angles = Vec::new();
        for r in REFS {
            if r.key == rkey {
                continue;
            }
            if let Some(j) = tpl.stops.iter().position(|s| s.s == r.stop_id) {
                also_angles.push((also.len(), heading(net, &tpl.stops, j)));
                also.push(AlsoOut {
                    ref_key: r.key.into(),
                    group: group_of(r.key, line).0,
                    dir: 0,
                    off: time_at(j).saturating_sub(dep0),
                });
            }
        }
        raws.push(Raw {
            out: PatternOut {
                id: format!("{line}|{from}|{to}"),
                line: line.to_string(),
                color: net.line_colors.get(*line).cloned().unwrap_or_else(|| "#888888".into()),
                regional: topology::is_regional_line(line),
                from: from.to_string(),
                to: to.to_string(),
                from_name: net.stop_name(from).to_string(),
                to_name: net.stop_name(to).to_string(),
                n: trips.len(),
                ref_key: rkey.into(),
                group,
                dir: 0,
                ref_off: time_at(k).saturating_sub(dep0),
                dur: tpl.stops[last].a.saturating_sub(dep0),
                also,
            },
            angle: heading(net, &tpl.stops, k),
            next_name,
            also_angles,
            tpl: tpl.clone(),
        });
    }

    // Sentidos: eje principal por estación de referencia con TODOS los rumbos que pasan por ella
    // (editables y calculados), así los dos conjuntos de vías de Sants comparten "sentido 0".
    let mut by_ref: HashMap<&str, Vec<f64>> = HashMap::new();
    for r in &raws {
        if let Some(a) = r.angle {
            by_ref.entry(REFS.iter().find(|x| x.key == r.out.ref_key).unwrap().key).or_default().push(a);
        }
        for (i, a) in &r.also_angles {
            if let Some(a) = a {
                by_ref.entry(REFS.iter().find(|x| x.key == r.out.also[*i].ref_key).unwrap().key).or_default().push(*a);
            }
        }
    }
    let axes: HashMap<&str, f64> = by_ref.iter().map(|(k, v)| (*k, axis_of(v))).collect();
    for r in &mut raws {
        let ax = axes.get(r.out.ref_key.as_str()).copied().unwrap_or(0.0);
        r.out.dir = r.angle.map(|a| dir_of(a, ax)).unwrap_or(0);
        for (i, a) in &r.also_angles {
            let ax = axes.get(r.out.also[*i].ref_key.as_str()).copied().unwrap_or(0.0);
            r.out.also[*i].dir = a.map(|a| dir_of(a, ax)).unwrap_or(0);
        }
    }

    // Relojes por estación: conjuntos de vías con patrones editables o calculados, y el rótulo de
    // cada sentido = la siguiente parada más habitual tras la referencia (o el destino si acaba allí).
    let mut refs = Vec::new();
    for rd in REFS {
        let mut used: BTreeMap<usize, ()> = BTreeMap::new();
        for r in &raws {
            if r.out.ref_key == rd.key {
                used.insert(r.out.group, ());
            }
            for a in &r.out.also {
                if a.ref_key == rd.key {
                    used.insert(a.group, ());
                }
            }
        }
        if used.is_empty() {
            continue;
        }
        let mut gs = Vec::new();
        for (&g, _) in &used {
            let label = TRACK_GROUPS
                .iter()
                .filter(|(r, _, _)| *r == rd.key)
                .nth(g)
                .map(|(_, l, _)| l.to_string())
                .unwrap_or_else(|| DEFAULT_GROUP.into());
            let mut dirs = Vec::new();
            for d in 0..2 {
                let mut cnt: BTreeMap<String, usize> = BTreeMap::new();
                for r in raws.iter().filter(|r| r.out.ref_key == rd.key && r.out.group == g && r.out.dir == d) {
                    // "" = el tren acaba en la referencia (no hay siguiente parada).
                    let name = r.next_name.clone().unwrap_or_default();
                    *cnt.entry(name).or_default() += r.out.n;
                }
                let best = cnt.into_iter().max_by_key(|(_, c)| *c).map(|(n, _)| n);
                dirs.push(match best {
                    Some(n) if !n.is_empty() => DirOut { label: n, terminal: false },
                    Some(_) => DirOut { label: String::new(), terminal: true },
                    None => DirOut { label: format!("sentido {}", d + 1), terminal: false },
                });
            }
            gs.push(GroupOut { label, dirs });
        }
        // Los índices de grupo de los patrones apuntan al ORDEN de `TRACK_GROUPS`; se reindexan
        // aquí a la posición en `gs` (solo grupos usados).
        let remap: HashMap<usize, usize> = used.keys().enumerate().map(|(i, &g)| (g, i)).collect();
        for r in &mut raws {
            if r.out.ref_key == rd.key {
                r.out.group = remap[&r.out.group];
            }
            for a in &mut r.out.also {
                if a.ref_key == rd.key {
                    a.group = remap[&a.group];
                }
            }
        }
        refs.push(RefOut {
            key: rd.key.into(),
            stop_id: rd.stop_id.into(),
            name: net.stop_name(rd.stop_id).to_string(),
            groups: gs,
        });
    }

    let mut templates = HashMap::new();
    let mut patterns = Vec::new();
    for r in raws {
        templates.insert(r.out.id.clone(), (r.tpl, r.out.ref_off));
        patterns.push(r.out);
    }
    CadenceCatalog { out: Catalog { open: OPEN_SEC, last_dep: LAST_DEP_SEC, refs, patterns }, templates }
}

// ---- Especificación (lo que guarda el creador) y generación ------------------------------------

fn one() -> u32 {
    1
}

/// Un tren que pasa por la referencia en el minuto `min` de cada `every` horas.
#[derive(Deserialize, Serialize, Clone)]
pub struct Marker {
    pub pat: String,
    pub min: u32,
    #[serde(default = "one")]
    pub every: u32,
    /// Variante: cada `alt.every` horas, ese tren hace OTRO recorrido (p. ej. R3: a Ripoll cada
    /// hora y, cada 3 h, a Puigcerdà en su lugar).
    #[serde(default)]
    pub alt: Option<Alt>,
}
#[derive(Deserialize, Serialize, Clone)]
pub struct Alt {
    pub pat: String,
    pub every: u32,
}
/// GRUPO de líneas que comparten un mismo reloj (p. ej. los regionales de Estació de França): una
/// SECUENCIA de recorridos y unos minutos de paso. Los pasos del día, en orden temporal, se reparten
/// rotando por la secuencia (minutos :00 y :30 con R13, R14, R15 → R13 a y media, R14 en punto,
/// R15 a y media…). Sirve para cadenciar en bloque servicios distintos con un mismo intervalo.
#[derive(Deserialize, Serialize, Clone)]
pub struct Group {
    #[serde(default)]
    pub name: String,
    /// Recorridos (ids de patrón) en el orden de la secuencia.
    pub items: Vec<String>,
    /// Minutos de paso por la referencia dentro de cada ciclo de `every` horas.
    pub mins: Vec<u32>,
    #[serde(default = "one")]
    pub every: u32,
}
#[derive(Deserialize, Serialize, Clone)]
pub struct Spec {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub markers: Vec<Marker>,
    #[serde(default)]
    pub groups: Vec<Group>,
}

/// Pasos de un GRUPO: `(paso por la referencia, índice en la secuencia)`. Los pasos son todos los
/// `mins` de cada ciclo de `every` horas, en orden temporal; la rotación empieza en el primer paso
/// cuyo PRIMER recorrido de la secuencia no sale de cabecera antes de la apertura (como un marcador
/// suelto) y avanza un puesto por paso. Un paso cuyo recorrido no cabe en el horario de apertura
/// (sale antes de las 04:30 o después de la última salida) se pierde, pero la rotación sigue: el
/// orden de la secuencia no depende de qué trenes caben.
fn group_slots(mins: &[u32], every_h: u32, offs: &[u32]) -> Vec<(u32, usize)> {
    let n = offs.len();
    if n == 0 || mins.is_empty() {
        return Vec::new();
    }
    let cycle = every_h.clamp(1, 24) * 3600;
    let mut times: Vec<u32> = Vec::new();
    let mut base = 0;
    while base < 30 * 3600 {
        for &m in mins {
            times.push(base + m.min(59) * 60);
        }
        base += cycle;
    }
    times.sort_unstable();
    times.dedup();
    let Some(k0) = times.iter().position(|&t| t >= OPEN_SEC + offs[0]) else { return Vec::new() };
    let mut out = Vec::new();
    for (k, &t) in times[k0..].iter().enumerate() {
        let i = k % n;
        if t >= OPEN_SEC + offs[i] && t - offs[i] <= LAST_DEP_SEC {
            out.push((t, i));
        }
    }
    out
}

/// Horas de paso por la referencia (s desde medianoche) de un marcador al minuto `min`, cada
/// `every_h` horas, para un recorrido que tarda `off` s de cabecera a referencia: el primero es el
/// primer minuto `min` cuya salida de cabecera no es anterior a la apertura; el último, el que aún
/// sale de cabecera como tarde a `LAST_DEP_SEC`.
fn slots(min: u32, every_h: u32, off: u32) -> Vec<u32> {
    let min = min.min(59);
    let step = every_h.clamp(1, 24) * 3600;
    let mut t = min * 60;
    while t < OPEN_SEC + off {
        t += 3600;
    }
    let mut out = Vec::new();
    while t - off <= LAST_DEP_SEC {
        out.push(t);
        t += step;
    }
    out
}

/// Un tren del viaje tipo desplazado para que salga de cabecera a `dep`.
fn shifted(tpl: &GameTrain, dep: u32, id: String) -> GameTrain {
    let delta = dep as i64 - tpl.stops[0].d as i64;
    let sh = |x: u32| (x as i64 + delta).max(0) as u32;
    GameTrain {
        train: id,
        line: tpl.line.clone(),
        from: tpl.from.clone(),
        to: tpl.to.clone(),
        offset_min: 0,
        stops: tpl.stops.iter().map(|s| GameStop { s: s.s.clone(), a: sh(s.a), d: sh(s.d), pass: s.pass }).collect(),
    }
}

/// Genera la malla de un horario cadenciado. Devuelve los trenes (ordenados por salida, ids únicos
/// `<línea>-C<nnn>`) y avisos legibles (recorridos que ya no existen en el GTFS cargado…).
pub fn generate(cat: &CadenceCatalog, spec: &Spec) -> (Vec<GameTrain>, Vec<String>) {
    let mut warnings = Vec::new();
    let mut raw: Vec<(u32, &GameTrain)> = Vec::new(); // (salida de cabecera, viaje tipo)
    for m in &spec.markers {
        let Some((base, off)) = cat.templates.get(&m.pat) else {
            warnings.push(format!("El recorrido {} ya no existe en el GTFS cargado: se omite.", m.pat));
            continue;
        };
        let alt = m.alt.as_ref().and_then(|a| match cat.templates.get(&a.pat) {
            Some(t) => Some((t, a.every.max(1))),
            None => {
                warnings.push(format!("La variante {} ya no existe en el GTFS cargado: se omite.", a.pat));
                None
            }
        });
        let ss = slots(m.min, m.every, *off);
        let Some(&first) = ss.first() else { continue };
        for t in ss {
            // Variante cada N horas (contadas desde el primer tren del marcador), si su salida de
            // cabecera cabe en el horario de apertura; si no, ese tren hace el recorrido base.
            if let Some(((tpl_alt, off_alt), every_alt)) = alt {
                let h = (t - first) / 3600;
                if h % every_alt == 0 && t >= OPEN_SEC + off_alt && t - off_alt <= LAST_DEP_SEC {
                    raw.push((t - off_alt, tpl_alt));
                    continue;
                }
            }
            raw.push((t - off, base));
        }
    }
    for g in &spec.groups {
        let mut seq: Vec<(&GameTrain, u32)> = Vec::new();
        for p in &g.items {
            match cat.templates.get(p) {
                Some((t, off)) => seq.push((t, *off)),
                None => warnings.push(format!("El recorrido {p} del grupo «{}» ya no existe en el GTFS cargado: se omite.", g.name)),
            }
        }
        let offs: Vec<u32> = seq.iter().map(|x| x.1).collect();
        for (t, i) in group_slots(&g.mins, g.every, &offs) {
            raw.push((t - seq[i].1, seq[i].0));
        }
    }
    raw.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.line.cmp(&b.1.line)).then(a.1.from.cmp(&b.1.from)));
    let mut per_line: HashMap<&str, u32> = HashMap::new();
    let trains = raw
        .into_iter()
        .map(|(dep, tpl)| {
            let k = per_line.entry(tpl.line.as_str()).or_default();
            *k += 1;
            shifted(tpl, dep, format!("{}-C{:03}", tpl.line, k))
        })
        .collect();
    (trains, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primer_tren_no_sale_antes_de_la_apertura() {
        // Paso por Sants al minuto 12, 40 min desde cabecera: 04:52 saldría a las 04:12 (antes de
        // abrir) → el primero pasa a las 05:12 (sale 04:32).
        let s = slots(12, 1, 40 * 60);
        assert_eq!(s[0], 5 * 3600 + 12 * 60);
        assert!(s[0] - 40 * 60 >= OPEN_SEC);
        // Cada hora, sin pasar de la última salida.
        assert!(s.windows(2).all(|w| w[1] - w[0] == 3600));
        assert!(*s.last().unwrap() - 40 * 60 <= LAST_DEP_SEC);
        // Cada 2 h → la mitad de trenes (±1).
        let s2 = slots(12, 2, 40 * 60);
        assert!(s2.windows(2).all(|w| w[1] - w[0] == 7200));
        // Sale de la propia referencia (off 0) al minuto 45: 04:45 ya está abierto.
        assert_eq!(slots(45, 1, 0)[0], 4 * 3600 + 45 * 60);
    }

    #[test]
    fn grupo_reparte_los_pasos_rotando_la_secuencia() {
        // Minutos :30 y :00 con tres recorridos (off 18 min): desde la apertura los pasos son
        // 04:30, 05:00, 05:30… pero el 1º de la secuencia saldría a las 04:12 → la rotación empieza
        // a las 05:00 (sale 04:42): R13 05:00, R14 05:30, R15 06:00, R13 06:30…
        let s = group_slots(&[30, 0], 1, &[18 * 60, 18 * 60, 18 * 60]);
        assert_eq!(&s[..4], &[(5 * 3600, 0), (5 * 3600 + 1800, 1), (6 * 3600, 2), (6 * 3600 + 1800, 0)]);
        // Un recorrido que no cabe pierde su paso pero no altera la rotación del resto.
        // (el 2º sale 20 h antes de pasar: en el día no cabe ninguno de sus pasos)
        let s = group_slots(&[0], 1, &[0, 20 * 3600]);
        let firsts: Vec<u32> = s.iter().filter(|x| x.1 == 0).map(|x| x.0).collect();
        assert!(firsts.windows(2).all(|w| w[1] - w[0] == 7200));
        assert!(s.iter().filter(|x| x.1 == 1).all(|x| x.0 >= OPEN_SEC + 20 * 3600));
        assert!(group_slots(&[], 1, &[0]).is_empty());
    }

    #[test]
    fn sentidos_por_eje_axial() {
        use std::f64::consts::PI;
        // Rumbos de ida (≈NE) y vuelta (≈SO) por la misma estación: el eje sale NE y los separa.
        let ne = PI / 4.0;
        let ax = axis_of(&[ne, ne + 0.1, ne + PI, ne + PI - 0.05]);
        assert!((ax - ne).abs() < 0.1, "eje {ax}");
        assert_eq!(dir_of(ne, ax), 0);
        assert_eq!(dir_of(ne + PI, ax), 1);
    }

    #[test]
    fn vuelta_corta_ocasional() {
        assert!(is_short_turn(135, 15)); // R4 en L'Hospitalet
        assert!(!is_short_turn(42, 27)); // R4 en Vilafranca: cabecera habitual
        assert!(!is_short_turn(0, 30)); // extremo real de la línea
    }
}
