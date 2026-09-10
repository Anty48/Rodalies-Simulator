//! Motor de datos del juego web (Fase 2).
//!
//! Expone en JSON, **directamente desde el feed GTFS de Fomento_Transit**, todo lo que el
//! juego necesita para construirse dinámicamente: la topología (estaciones con vías reales,
//! secuencias ordenadas de cada línea y color oficial) y los horarios (GTFS «tal cual» o los
//! **horarios optimizados** que escribe el optimizador en `report/optimized/`).
//!
//! Sustituye a los JSON estáticos que traía el juego de Godot (extraídos de PDF y con la
//! secuencia de estaciones aproximada por «vecino más cercano»): aquí la secuencia es la real
//! del GTFS, y las vías por estación salen de `topology`.

use crate::gtfs_loader::Network;
use crate::topology;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

// --------------------------------------------------------------------------
// Topología para el juego: estaciones + líneas (secuencias + color)
// --------------------------------------------------------------------------

#[derive(Serialize)]
struct GameCorridor {
    /// Líneas que comparten este grupo de vías (p. ej. ["R4","R7"]).
    lines: Vec<String>,
    tracks: u32,
}

#[derive(Serialize)]
struct GameStation {
    id: String,
    name: String,
    lat: f64,
    lon: f64,
    /// Número de vías/andenes reales (de `topology`, con valor por defecto 2).
    tracks: u32,
    /// Líneas (route_short_name) que dan servicio a esta estación.
    lines: Vec<String>,
    /// Reparto de las `tracks` en corredores por subconjunto de líneas (solo donde el reparto
    /// está confirmado por fuente, p. ej. Montcada Bifurcació); vacío = pool único (`tracks`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    corridors: Vec<GameCorridor>,
    /// URLs de esquemas de vías reales (trenscat.com, ver `reference/trenscat/`), si los hay
    /// para esta estación. Referencia visual para modelar cantones/agujas reales.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    diagrams: Vec<String>,
}

/// Índice `stop_id -> [rutas de imagen]` de `reference/trenscat/index.json`, si existe.
/// Se recarga en cada llamada a `network_json` (barato: unas pocas decenas de KB) para que
/// añadir diagramas nuevos a `reference/trenscat/` se refleje sin reiniciar el servidor.
fn trenscat_diagrams() -> HashMap<String, Vec<String>> {
    #[derive(serde::Deserialize)]
    struct Entry {
        stop_id: String,
        images: Vec<String>,
    }
    let Ok(raw) = std::fs::read_to_string("reference/trenscat/index.json") else {
        return HashMap::new();
    };
    let Ok(entries) = serde_json::from_str::<Vec<Entry>>(&raw) else {
        return HashMap::new();
    };
    entries
        .into_iter()
        .map(|e| {
            let urls = e
                .images
                .into_iter()
                .map(|f| format!("/reference/trenscat/{}/{}", e.stop_id, f))
                .collect();
            (e.stop_id, urls)
        })
        .collect()
}

#[derive(Serialize)]
struct GameDirection {
    /// stop_id de origen y destino del sentido (clave estable).
    from: String,
    to: String,
    /// Etiqueta legible «Origen → Destino».
    label: String,
    /// Secuencia REAL y ordenada de stop_id de este sentido (del GTFS).
    stations: Vec<String>,
}

#[derive(Serialize)]
struct GameLine {
    line: String,
    /// Color oficial del feed GTFS (route_color) o, si falta, un color estable derivado.
    color: String,
    /// `true` si es un servicio Regionals/media distancia (categoría con prioridad de paso
    /// sobre cercanías en cruces de vía única; ver `topology::is_regional_line`).
    regional: bool,
    directions: Vec<GameDirection>,
}

/// Cantón (arista dirigida entre dos estaciones consecutivas de algún servicio) y las
/// líneas que lo recorren. Es la unidad de vía real: dibujar por cantones —en vez de por
/// una sola polilínea de itinerario— cubre TODAS las secciones por donde circula algún tren
/// y evita las rectas «entre puntos aleatorios» de un itinerario mal ordenado.
#[derive(Serialize)]
struct GameEdge {
    from: String,
    to: String,
    lines: Vec<String>,
}

#[derive(Serialize)]
struct GameNetwork {
    generated_at: String,
    n_stations: usize,
    n_lines: usize,
    lines: Vec<GameLine>,
    stations: Vec<GameStation>,
    /// Cantones (secciones de vía) con las líneas que los usan.
    edges: Vec<GameEdge>,
    /// Pares de estaciones (por stop_id) que forman tramos de vía única (de `topology`).
    single_track: Vec<[String; 2]>,
    /// Cizallamientos: pares de cantones que se cruzan físicamente (de `topology`). Cada
    /// elemento es `[[a,b],[c,d]]`: ocupar el cantón a↔b obliga a retener en rojo c↔d y
    /// viceversa, en cualquier sentido de circulación.
    shears: Vec<[[String; 2]; 2]>,
}

/// Grafo de cantones ATÓMICOS: pares (a,b) dirigidos que son tramos de vía reales, no
/// expresables como cadena de otros saltos ya existentes. Se calcula de forma GLOBAL —todas
/// las líneas juntas—, no línea por línea: un salto exprés de una línea (p. ej. R11 saltando
/// B) que OTRA línea (p. ej. R2) recorre parada a parada deja de ser atómico y se descompone
/// en sus cantones reales. Así un tren que se salta paradas comparte EXACTAMENTE las mismas
/// aristas del grafo que uno que para en todas —y por tanto su ocupación, semáforos y
/// cizallamientos— en vez de circular por un "plano paralelo" sin interacción.
struct AtomicGraph {
    adj: HashMap<String, Vec<String>>,
    atomic: std::collections::HashSet<(String, String)>,
    coords: HashMap<String, (f64, f64)>,
}

impl AtomicGraph {
    fn is_atomic(&self, a: &str, b: &str) -> bool {
        self.atomic.contains(&(a.to_string(), b.to_string()))
    }

    /// Camino más corto (nº de saltos) de `a` a `b` usando SOLO cantones atómicos, incluyendo
    /// ambos extremos. `None` si no hay descomposición posible (tramo exclusivo de esta línea,
    /// sin ninguna otra que pare en las estaciones intermedias) — en ese caso no se inventa
    /// una topología que no está en los datos; se mantiene el salto directo tal cual.
    ///
    /// La búsqueda se restringe a nodos GEOGRÁFICAMENTE entre `a` y `b` (misma tolerancia que
    /// `build_atomic_graph`): sin esto, el grafo atómico global está muy conectado (decenas de
    /// líneas se cruzan) y un BFS sin restricción encuentra "caminos" por el número de saltos
    /// que dan un rodeo absurdo por una rama completamente distinta — p. ej. un salto real
    /// Santa Perpètua↔Barberà (un par de km) "descompuesto" vía Cerdanyola/Montcada Bifurcació
    /// (decenas de km al sur), aunque esos cantones sean atómicos de por sí.
    fn decompose(&self, a: &str, b: &str) -> Option<Vec<String>> {
        if a == b {
            return Some(vec![a.to_string()]);
        }
        let in_bounds = |id: &str| -> bool {
            let (Some(&pa), Some(&pb), Some(&pc)) =
                (self.coords.get(a), self.coords.get(b), self.coords.get(id))
            else {
                return true; // sin coordenadas: no se puede acotar, se deja pasar
            };
            let dab = haversine_m(pa, pb);
            haversine_m(pa, pc) <= dab * 1.2 && haversine_m(pc, pb) <= dab * 1.2
        };
        let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut queue: std::collections::VecDeque<Vec<String>> = std::collections::VecDeque::new();
        queue.push_back(vec![a.to_string()]);
        visited.insert(a.to_string());
        while let Some(path) = queue.pop_front() {
            let last = path.last().unwrap().clone();
            let Some(nbrs) = self.adj.get(&last) else { continue };
            for nbr in nbrs {
                if nbr == b {
                    let mut full = path.clone();
                    full.push(nbr.clone());
                    return Some(full);
                }
                if !in_bounds(nbr) {
                    continue;
                }
                if visited.insert(nbr.clone()) {
                    let mut np = path.clone();
                    np.push(nbr.clone());
                    queue.push_back(np);
                }
            }
        }
        None
    }
}

/// Distancia entre dos puntos (metros), fórmula de Haversine.
fn haversine_m(a: (f64, f64), b: (f64, f64)) -> f64 {
    let r = 6_371_000.0_f64;
    let (p1, p2) = (a.0.to_radians(), b.0.to_radians());
    let dphi = (b.0 - a.0).to_radians();
    let dlambda = (b.1 - a.1).to_radians();
    let h = (dphi / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dlambda / 2.0).sin().powi(2);
    2.0 * r * h.sqrt().clamp(-1.0, 1.0).asin()
}

/// Construye el grafo atómico a partir de TODOS los saltos crudos (paradas consecutivas de
/// cualquier trayecto de tren, cualquier línea) del feed activo.
///
/// Atomicidad por GEOMETRÍA (lat/lon reales), no por alcanzabilidad de grafo: un salto (a,b)
/// es un atajo si existe otra parada `c` a la que `a` también salta directamente (en CUALQUIER
/// línea) que queda geográficamente "de camino" entre a y b (dist(a,c)+dist(c,b) ≈ dist(a,b)).
/// La alcanzabilidad de grafo pura NO sirve aquí: al mezclar viajes de ida y vuelta de líneas
/// distintas, casi cualquier par de paradas adyacentes "se alcanza" por algún rodeo absurdo
/// combinando tramos de sentidos opuestos, así que casi nada salía atómico. La geometría no
/// tiene ese problema: solo cuenta si `c` está realmente en medio del trayecto real.
fn build_atomic_graph(net: &Network) -> AtomicGraph {
    let mut raw_pairs: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    for svc in &net.services {
        if svc.is_bus {
            continue;
        }
        for w in svc.schedule.windows(2) {
            raw_pairs.insert((w[0].stop_id.clone(), w[1].stop_id.clone()));
        }
    }
    let mut raw_adj: HashMap<String, Vec<String>> = HashMap::new();
    for (a, b) in &raw_pairs {
        raw_adj.entry(a.clone()).or_default().push(b.clone());
    }
    let coord_of = |id: &str| -> Option<(f64, f64)> {
        net.node(id).and_then(|n| {
            let s = &net.graph[n];
            match (s.lat, s.lon) {
                (Some(la), Some(lo)) => Some((la, lo)),
                _ => None,
            }
        })
    };

    let mut atomic: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    for (a, b) in &raw_pairs {
        let mut es_atomico = true;
        if let (Some(pa), Some(pb)) = (coord_of(a), coord_of(b)) {
            let dab = haversine_m(pa, pb);
            if let Some(nbrs) = raw_adj.get(a) {
                for c in nbrs {
                    if c == b {
                        continue;
                    }
                    let Some(pc) = coord_of(c) else { continue };
                    let dac = haversine_m(pa, pc);
                    let dcb = haversine_m(pc, pb);
                    // c está "de camino" de a a b: margen del 20% para curvatura real de vía
                    // (tramos costeros/de valle como Tortosa-Amposta se curvan bastante más
                    // que una recta; un margen del 5% dejaba pasar atajos reales sin descomponer).
                    if dac + dcb <= dab * 1.2 && dac > 1.0 && dcb > 1.0 {
                        es_atomico = false;
                        break;
                    }
                }
            }
        }
        // Sin coordenadas: no se puede verificar geometría, se mantiene tal cual (no se
        // inventa una descomposición sin datos para confirmarla).
        if es_atomico {
            atomic.insert((a.clone(), b.clone()));
        }
    }
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();
    for (a, b) in &atomic {
        adj.entry(a.clone()).or_default().push(b.clone());
    }
    let mut coords: HashMap<String, (f64, f64)> = HashMap::new();
    for (a, b) in &raw_pairs {
        if let Some(p) = coord_of(a) {
            coords.entry(a.clone()).or_insert(p);
        }
        if let Some(p) = coord_of(b) {
            coords.entry(b.clone()).or_insert(p);
        }
    }
    AtomicGraph { adj, atomic, coords }
}

/// Color estable de reserva cuando el feed no trae `route_color` (mismo criterio que el
/// mapa de la red): tono derivado del nombre, luminosidad moderada para fondo claro.
fn fallback_color(line: &str) -> String {
    let h: u32 = line
        .bytes()
        .fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    format!("hsl({}, 65%, 45%)", h % 360)
}

/// JSON de la topología del juego (estaciones + líneas con secuencias reales y colores).
pub fn network_json(net: &Network, lines: &[String], generated_at: &str) -> String {
    // Acumuladores de estaciones: id -> (nombre, lat, lon) y líneas que la sirven. Se llenan
    // recorriendo TODOS los servicios de tren (no autobuses), de modo que toda estación por la
    // que circula algún tren queda registrada (y así todo extremo de cantón es una estación
    // conocida), no sólo las del itinerario canónico de cada sentido.
    let mut st_meta: HashMap<String, (String, f64, f64)> = HashMap::new();
    let mut st_lines: HashMap<String, Vec<String>> = HashMap::new();
    for svc in &net.services {
        if svc.is_bus {
            continue;
        }
        for st in &svc.schedule {
            if let Some(n) = net.node(&st.stop_id) {
                let node = &net.graph[n];
                if let (Some(lat), Some(lon)) = (node.lat, node.lon) {
                    st_meta
                        .entry(st.stop_id.clone())
                        .or_insert_with(|| (node.stop_name.clone(), lat, lon));
                    let entry = st_lines.entry(st.stop_id.clone()).or_default();
                    if !entry.contains(&svc.route_short_name) {
                        entry.push(svc.route_short_name.clone());
                    }
                }
            }
        }
    }

    let mut out_lines: Vec<GameLine> = Vec::new();
    for line in lines {
        let dirs = crate::calculator::schedules::line_directions(net, line);
        let mut directions: Vec<GameDirection> = Vec::new();
        for d in dirs.into_iter().filter(|d| d.n_services >= 2) {
            let Some(itin) = crate::calculator::schedules::itinerary(net, line, &d.key) else {
                continue;
            };
            let seq: Vec<String> = itin.stops.iter().map(|s| s.stop_id.clone()).collect();
            directions.push(GameDirection {
                from: d.key.0.clone(),
                to: d.key.1.clone(),
                label: d.label,
                stations: seq,
            });
        }
        // Sin itinerario activo: la línea igualmente existe si está definida como tren en
        // routes.txt (p. ej. R7 suprimida temporalmente por obras). Solo se descarta si no es
        // ni siquiera una línea de tren real (autobuses de sustitución u otro filtro externo).
        if directions.is_empty() && !net.rail_lines.contains(line) {
            continue;
        }
        let color = net
            .line_colors
            .get(line)
            .cloned()
            .unwrap_or_else(|| fallback_color(line));
        out_lines.push(GameLine {
            line: line.clone(),
            color,
            regional: topology::is_regional_line(line),
            directions,
        });
    }

    // Conjunto de estaciones con coordenadas (para validar extremos de cantón).
    let st_coords: std::collections::HashSet<String> = st_meta.keys().cloned().collect();

    // Estaciones ordenadas por id (determinista).
    let diagrams_by_id = trenscat_diagrams();
    let mut stations: Vec<GameStation> = st_meta
        .into_iter()
        .map(|(id, (name, lat, lon))| {
            let tracks = topology::platform_tracks(&name);
            let corridors = topology::corridors(&name)
                .unwrap_or_default()
                .into_iter()
                .map(|c| GameCorridor {
                    lines: c.lines.iter().map(|s| s.to_string()).collect(),
                    tracks: c.tracks,
                })
                .collect();
            let mut ls = st_lines.remove(&id).unwrap_or_default();
            ls.sort();
            let diagrams = diagrams_by_id.get(&id).cloned().unwrap_or_default();
            GameStation {
                id,
                name,
                lat,
                lon,
                tracks,
                lines: ls,
                corridors,
                diagrams,
            }
        })
        .collect();
    stations.sort_by(|a, b| a.id.cmp(&b.id));

    // Cantones: recorremos TODOS los servicios de tren (no autobuses) y registramos cada par
    // de paradas consecutivas como una arista dirigida, acumulando qué líneas la usan. Así el
    // trazado cubre cada sección por la que circula algún tren (incluidas ramificaciones).
    let mut edge_lines: BTreeMap<(String, String), std::collections::BTreeSet<String>> = BTreeMap::new();
    for svc in &net.services {
        if svc.is_bus {
            continue;
        }
        for w in svc.schedule.windows(2) {
            edge_lines
                .entry((w[0].stop_id.clone(), w[1].stop_id.clone()))
                .or_default()
                .insert(svc.route_short_name.clone());
        }
    }
    // Descomposición GLOBAL por cantones atómicos: un servicio exprés que salta paradas
    // (A→C) se reparte entre los cantones reales A→B→C si OTRA línea (cualquiera) ya para
    // en B — así comparte exactamente la misma arista, en vez de dibujar (y simular) un
    // atajo recto que no existe físicamente. Si nadie más para ahí, no hay descomposición
    // posible y se mantiene el salto directo (no se inventa una estación intermedia).
    let coord = |id: &str| st_coords.contains(id);
    let atomic_graph = build_atomic_graph(net);
    let mut kept: BTreeMap<(String, String), std::collections::BTreeSet<String>> = BTreeMap::new();
    for ((a, b), lns) in &edge_lines {
        if !(coord(a) && coord(b)) {
            continue;
        }
        if atomic_graph.is_atomic(a, b) {
            for l in lns {
                kept.entry((a.clone(), b.clone())).or_default().insert(l.clone());
            }
            continue;
        }
        match atomic_graph.decompose(a, b) {
            Some(path) if path.len() > 2 => {
                for w in path.windows(2) {
                    if !(coord(&w[0]) && coord(&w[1])) {
                        continue;
                    }
                    for l in lns {
                        kept.entry((w[0].clone(), w[1].clone())).or_default().insert(l.clone());
                    }
                }
            }
            _ => {
                for l in lns {
                    kept.entry((a.clone(), b.clone())).or_default().insert(l.clone());
                }
            }
        }
    }
    let edges: Vec<GameEdge> = kept
        .into_iter()
        .map(|((from, to), lines)| GameEdge {
            from,
            to,
            lines: lines.into_iter().collect(),
        })
        .collect();

    // Tramos de vía única (de topology), traducidos a pares de stop_id.
    let single_track: Vec<[String; 2]> = topology::single_track_pairs(net)
        .into_iter()
        .map(|(a, b)| [net.graph[a].stop_id.clone(), net.graph[b].stop_id.clone()])
        .filter(|[a, b]| coord(a) && coord(b))
        .collect();

    // Cizallamientos (de topology), traducidos a pares de cantones por stop_id.
    let shears: Vec<[[String; 2]; 2]> = topology::shear_conflicts(net)
        .into_iter()
        .map(|((a1, b1), (a2, b2))| [[a1, b1], [a2, b2]])
        .filter(|[[a1, b1], [a2, b2]]| coord(a1) && coord(b1) && coord(a2) && coord(b2))
        .collect();

    let net_out = GameNetwork {
        generated_at: generated_at.to_string(),
        n_stations: stations.len(),
        n_lines: out_lines.len(),
        lines: out_lines,
        stations,
        edges,
        single_track,
        shears,
    };
    serde_json::to_string(&net_out).unwrap_or_else(|_| "{}".into())
}

// --------------------------------------------------------------------------
// Horarios para el juego: GTFS «tal cual» u optimizados
// --------------------------------------------------------------------------

#[derive(Serialize)]
struct GameStop {
    /// stop_id.
    s: String,
    /// Llegada (segundos desde medianoche).
    a: u32,
    /// Salida (segundos desde medianoche).
    d: u32,
}

#[derive(Serialize)]
struct GameTrain {
    train: String,
    line: String,
    /// Origen y destino (stop_id) del recorrido, como clave de sentido.
    from: String,
    to: String,
    /// Desfase aplicado en minutos (0 si son horarios GTFS sin optimizar).
    offset_min: i64,
    stops: Vec<GameStop>,
}

#[derive(Serialize)]
struct GameSchedule {
    source: String,
    service_id: String,
    n_trains: usize,
    trains: Vec<GameTrain>,
}

/// Lee los desfases por línea (min) de los CSV optimizados en `report/optimized/`.
/// Devuelve `route_short_name -> offset_min`. Vacío si no hay ninguna optimización.
fn optimized_offsets(dir: &Path) -> HashMap<String, i64> {
    let mut offsets = HashMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return offsets;
    };
    for e in entries.flatten() {
        let fname = e.file_name().to_string_lossy().to_string();
        let Some(line) = fname.strip_suffix("_optimized.csv") else {
            continue;
        };
        let Ok(mut rdr) = csv::ReaderBuilder::new().trim(csv::Trim::All).from_path(e.path()) else {
            continue;
        };
        // La columna offset_min es constante por línea: basta la primera fila de datos.
        if let Some(rec) = rdr.records().flatten().next() {
            // Cabecera: trip_short_name,trip_id,stop_sequence,stop_id,stop_name,llegada,salida,offset_min
            if let Some(v) = rec.get(7).and_then(|v| v.parse::<i64>().ok()) {
                offsets.insert(line.to_string(), v);
            }
        }
    }
    offsets
}

/// JSON de los horarios del día tipo dominante (día laborable), sólo trenes (sin autobuses).
/// `optimized = true` aplica los desfases por línea de `report/optimized/`.
pub fn schedule_json(net: &Network, optimized: bool, line_filter: Option<&str>) -> String {
    let service_id = net.dominant_service().unwrap_or_default();
    let offsets = if optimized {
        optimized_offsets(Path::new("report/optimized"))
    } else {
        HashMap::new()
    };

    let atomic_graph = build_atomic_graph(net);

    let mut trains: Vec<GameTrain> = Vec::new();
    for svc in &net.services {
        if svc.is_bus || svc.service_id != service_id {
            continue;
        }
        if let Some(l) = line_filter {
            if svc.route_short_name != l {
                continue;
            }
        }
        if svc.schedule.len() < 2 {
            continue;
        }
        let off_min = offsets.get(&svc.route_short_name).copied().unwrap_or(0);
        let off = off_min * 60;
        // Reparte los saltos NO atómicos (exprés) en sus cantones reales, con puntos de paso
        // sintéticos (a=d, sin parada) interpolados por el tiempo nominal de cada cantón real
        // — así el tren circula por la misma vía física que los que paran en todas, y su
        // ocupación de cantón interactúa correctamente con ellos.
        let mut stops: Vec<GameStop> = Vec::with_capacity(svc.schedule.len());
        for (idx, st) in svc.schedule.iter().enumerate() {
            let arr = (st.arrival_sec as i64 + off).max(0) as u32;
            let dep = (st.departure_sec as i64 + off).max(0) as u32;
            if idx > 0 {
                let prev = &svc.schedule[idx - 1];
                if !atomic_graph.is_atomic(&prev.stop_id, &st.stop_id) {
                    if let Some(path) = atomic_graph.decompose(&prev.stop_id, &st.stop_id) {
                        if path.len() > 2 {
                            let dep_prev = stops.last().map(|s: &GameStop| s.d).unwrap_or(dep);
                            let total = arr.saturating_sub(dep_prev).max(1) as f64;
                            let weights: Vec<f64> = path
                                .windows(2)
                                .map(|w| {
                                    net.edge_between(&w[0], &w[1])
                                        .map(|e| e.nominal_run_secs.max(1) as f64)
                                        .unwrap_or(1.0)
                                })
                                .collect();
                            let sum_w: f64 = weights.iter().sum::<f64>().max(1.0);
                            let mut acc = dep_prev as f64;
                            for (i, mid) in path[1..path.len() - 1].iter().enumerate() {
                                acc += total * (weights[i] / sum_w);
                                let t = acc.round() as u32;
                                stops.push(GameStop { s: mid.clone(), a: t, d: t });
                            }
                        }
                    }
                }
            }
            stops.push(GameStop { s: st.stop_id.clone(), a: arr, d: dep });
        }
        trains.push(GameTrain {
            train: svc.train_number.clone(),
            line: svc.route_short_name.clone(),
            from: svc.schedule.first().map(|s| s.stop_id.clone()).unwrap_or_default(),
            to: svc.schedule.last().map(|s| s.stop_id.clone()).unwrap_or_default(),
            offset_min: off_min,
            stops,
        });
    }
    // Orden determinista por hora de salida del primer servicio.
    trains.sort_by_key(|t| t.stops.first().map(|s| s.d).unwrap_or(0));

    // Recuento por línea para diagnóstico rápido (BTreeMap = orden estable).
    let mut _per_line: BTreeMap<&str, usize> = BTreeMap::new();
    for t in &trains {
        *_per_line.entry(t.line.as_str()).or_insert(0) += 1;
    }

    let out = GameSchedule {
        source: if optimized { "optimized".into() } else { "gtfs".into() },
        service_id,
        n_trains: trains.len(),
        trains,
    };
    serde_json::to_string(&out).unwrap_or_else(|_| "{}".into())
}
