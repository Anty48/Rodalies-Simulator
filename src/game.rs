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
struct GameStation {
    id: String,
    name: String,
    lat: f64,
    lon: f64,
    /// Número de vías/andenes reales (de `topology`, con valor por defecto 2).
    tracks: u32,
    /// Líneas (route_short_name) que dan servicio a esta estación.
    lines: Vec<String>,
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
}

/// ¿Existe un camino de `a` a `b` en `adj` SIN usar la arista directa `a→b`? (BFS). Se usa
/// para la reducción transitiva del trazado: si lo hay, la arista `a→b` es un atajo (exprés)
/// que la vía real ya cubre paso a paso, así que no debe dibujarse.
fn path_exists_excluding(adj: &HashMap<&str, Vec<&str>>, a: &str, b: &str) -> bool {
    let mut visto: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut cola: std::collections::VecDeque<&str> = std::collections::VecDeque::new();
    visto.insert(a);
    if let Some(vs) = adj.get(a) {
        for &v in vs {
            if v == b {
                continue; // salta la arista directa a→b (una única vez, desde el origen)
            }
            if visto.insert(v) {
                cola.push_back(v);
            }
        }
    }
    while let Some(u) = cola.pop_front() {
        if u == b {
            return true;
        }
        if let Some(vs) = adj.get(u) {
            for &v in vs {
                if visto.insert(v) {
                    cola.push_back(v);
                }
            }
        }
    }
    false
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
        if dirs.is_empty() {
            continue; // línea sin servicios de tren (p. ej. sólo autobuses)
        }
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
        if directions.is_empty() {
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
            directions,
        });
    }

    // Conjunto de estaciones con coordenadas (para validar extremos de cantón).
    let st_coords: std::collections::HashSet<String> = st_meta.keys().cloned().collect();

    // Estaciones ordenadas por id (determinista).
    let mut stations: Vec<GameStation> = st_meta
        .into_iter()
        .map(|(id, (name, lat, lon))| {
            let tracks = topology::platform_tracks(&name);
            let mut ls = st_lines.remove(&id).unwrap_or_default();
            ls.sort();
            GameStation {
                id,
                name,
                lat,
                lon,
                tracks,
                lines: ls,
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
    // Reducción transitiva POR LÍNEA: un servicio exprés que salta paradas produce un
    // «atajo» A→C aunque la vía real pase por B (existen A→B y B→C). Ese atajo dibujaría una
    // línea recta espuria. Para cada línea eliminamos toda arista (u,v) que sea alcanzable de
    // u a v por OTRO camino de la misma línea; así queda sólo la adyacencia fina (la vía real),
    // conservando ramificaciones (que no se descomponen). El resultado se une entre líneas.
    let coord = |id: &str| st_coords.contains(id);
    let mut per_line: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for ((a, b), lns) in &edge_lines {
        if !(coord(a) && coord(b)) {
            continue;
        }
        for l in lns {
            per_line.entry(l.clone()).or_default().push((a.clone(), b.clone()));
        }
    }
    let mut kept: BTreeMap<(String, String), std::collections::BTreeSet<String>> = BTreeMap::new();
    for (line, es) in &per_line {
        let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
        for (a, b) in es {
            adj.entry(a.as_str()).or_default().push(b.as_str());
        }
        for (a, b) in es {
            if !path_exists_excluding(&adj, a, b) {
                kept.entry((a.clone(), b.clone())).or_default().insert(line.clone());
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

    let net_out = GameNetwork {
        generated_at: generated_at.to_string(),
        n_stations: stations.len(),
        n_lines: out_lines.len(),
        lines: out_lines,
        stations,
        edges,
        single_track,
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
        let stops: Vec<GameStop> = svc
            .schedule
            .iter()
            .map(|st| GameStop {
                s: st.stop_id.clone(),
                a: (st.arrival_sec as i64 + off).max(0) as u32,
                d: (st.departure_sec as i64 + off).max(0) as u32,
            })
            .collect();
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
