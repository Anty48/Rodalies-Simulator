//! Datos operativos de infraestructura que el GTFS no aporta: número de vías por
//! estación y tramos de vía única. Derivados de conocimiento operativo y del simulador
//! de referencia (Godot: `estaciones.json` `vias`, `lineas.json` `via_unica_desde`).

use std::collections::HashSet;

use petgraph::graph::NodeIndex;

use crate::gtfs_loader::Network;

/// Nº TOTAL de vías por estación: principales (paso/andén) + no principales (apartaderos
/// donde se estacionan los trenes que terminan servicio y dan la vuelta, dejan pasar a un
/// Regional, o pernoctan — ningún tren "desaparece", todos deben tener dónde descansar). Los
/// grandes nudos tienen muchas vías; el resto, 2 por defecto (vía doble típica sin apartadero).
/// Fuente primaria: `game/godot-original/datos/estaciones.json` (`vias`, 117 estaciones
/// emparejadas con el GTFS por nombre, sesión 2026-09-10) — ya reflejaba este concepto total
/// (principales+no principales), a diferencia de trenscat.com, cuyas fichas de texto libre no
/// dan un recuento total estructurado fiable de extraer en bloque (solo menciones sueltas de
/// números de vía dentro de la narrativa histórica de cada estación).
/// Vías PRINCIPALES de andén por cada ACCESO (sentido de llegada) de una estación, para las
/// estaciones donde un tren parado no debe cerrar la señal anterior: con 2, la señal de entrada solo
/// se pone en rojo cuando las dos vías del acceso están ocupadas (o hay un tren entrando en el tramo).
/// La salida sigue siendo una única vía por sentido: un tren que acaba de salir cierra las dos.
/// Fuente: usuario (2026-10-15) — Barcelona-Sants tiene 14 vías: 6 de alta velocidad (no utilizables
/// por Rodalies) + 2 por cada uno de los 4 accesos (L'Hospitalet, Bellvitge, Pl. Catalunya,
/// Pg. de Gràcia). Por defecto 1 (el andén es la prolongación del tramo de aproximación).
pub fn approach_main_tracks(stop_id: &str) -> u32 {
    APPROACH_MAIN_TRACKS.iter().find(|(id, _)| *id == stop_id).map_or(1, |(_, n)| *n)
}
const APPROACH_MAIN_TRACKS: &[(&str, u32)] = &[
    ("71801", 2), // Barcelona-Sants
    // Tortosa: final en fondo de saco del ramal desde L'Aldea, con vía principal + 3 vías de 241-428 m
    // (OSM, raw/osm_rail.json). Los regionales que invierten allí tienen 15-35 min de margen en el
    // andén; con una sola vía el R16 que llega detrás esperaba en Camp-redó a que se fueran.
    ("65400", 2),
];

pub fn platform_tracks(stop_name: &str) -> u32 {
    for (name, tracks) in PLATFORM_TRACKS {
        if *name == stop_name {
            return *tracks;
        }
    }
    2
}

const PLATFORM_TRACKS: &[(&str, u32)] = &[
    ("Arenys de Mar", 5),
    ("Badalona", 3),
    ("Balenyà-Els Hostalets", 2),
    // Frontera: Cerbère y Portbou son estaciones de intercambio de ancho con grandes playas.
    ("Cerbère", 10),
    ("Cervera", 3),
    ("Balenyà-Tona-Seva", 3),
    ("Barberà del Vallès", 2),
    ("Barcelona Arc de Triomf", 2),
    ("Barcelona El Clot", 4),
    ("Barcelona Estació de França", 12),
    ("Barcelona Fabra i Puig", 4),
    ("Barcelona La Sagrera-Meridiana", 2),
    ("Barcelona Sant Andreu", 6),
    ("Barcelona Torre Baró -Vallbona", 2),
    ("Barcelona-Passeig de Gràcia", 2),
    ("Barcelona-Sants", 14),
    ("Blanes", 4),
    ("Borgonyà", 2),
    ("Cabrera de Mar-Vilassar de Mar", 2),
    ("Calafell", 4),
    ("Caldes d'Estrac", 2),
    ("Calella", 5),
    ("Campdevànol", 2),
    ("Canet de Mar", 2),
    ("Cardedeu", 3),
    ("Castellbisbal", 6),
    ("Castelldefels", 5),
    ("Centelles", 2),
    ("Cerdanyola Universitat", 5),
    ("Cerdanyola del Vallès", 3),
    // Cornellà: 2 principales + 8 no principales de COCHERA (pernocta de la flota; usuario 2026-09-24).
    ("Cornellà", 10),
    ("Cubelles", 2),
    ("Cunit", 2),
    ("El Masnou", 3),
    ("El Papiol", 2),
    ("El Prat de Llobregat", 4),
    ("El Vendrell", 4),
    ("Els Monjos", 2),
    ("Figaró", 2),
    ("Garraf", 5), // usuario 2026-10-20: 5 vías (la referencia Godot daba 2); permite apartarse en el Garraf
    ("Gavà", 4),
    ("Gelida", 2),
    ("Granollers Centre", 9),
    ("Granollers-Canovelles", 3),
    ("Girona", 4),
    ("Gualba", 2),
    ("Hostalric", 2),
    ("L'Arboç", 3),
    ("L'Hospitalet de Llobregat", 20),
    ("La Farga de Bebié", 2),
    ("La Garriga", 3),
    ("La Granada", 2),
    ("La Llagosta", 2),
    ("La Molina", 3),
    // La Plana-Picamoixons: 4 vías, TODAS principales: 2 de la R13 (hacia Valls) y 2 de la R14.
    ("La Plana-Picamoixons", 4),
    ("La Tor de Querol-Enveig", 3),
    ("Lavern-Subirats", 2),
    ("Lleida-Pirineus", 6),
    ("Les Franqueses del Vallès", 2),
    ("Les Franqueses-Granollers Nord", 2),
    ("Llinars del Vallès", 2),
    ("Malgrat de Mar", 2),
    ("Manlleu", 2),
    ("Manresa", 6),
    ("Martorell Central", 6),
    ("Mataró", 7),
    ("Maçanet-Massanes", 6),
    ("Molins de Rei", 4),
    ("Mollet-Sant Fost", 4),
    ("Mollet-Santa Rosa", 2),
    ("Montcada i Reixac", 3),
    ("Montcada i Reixac-Manresa", 2),
    ("Montcada i Reixac-Santa Maria", 2),
    ("Montcada-Bifurcació", 15),
    ("Montcada-Ripollet", 2),
    ("Montgat", 2),
    ("Montgat-Nord", 2),
    ("Montmeló", 2),
    ("Ocata", 2),
    ("Palautordera", 2),
    ("Parets del Vallès", 2),
    ("Pineda de Mar", 2),
    ("Portbou", 10),
    ("Planoles", 1),
    ("Platja de Castelldefels", 2),
    ("Premià de Mar", 2),
    ("Puigcerdà", 3),
    ("Ribes de Freser", 3),
    ("Riells i Viabrea-Breda", 2),
    ("Ripoll", 5),
    ("Rubí Can Vallhonrat", 2),
    ("Sabadell Centre", 2),
    ("Sabadell Nord", 2),
    ("Sabadell Sud", 3),
    ("Sant Adrià de Besòs", 2),
    ("Sant Andreu de Llavaneres", 2),
    ("Sant Celoni", 5),
    ("Sant Cugat Coll Favà", 2),
    ("Sant Feliu de Llobregat", 2),
    ("Sant Joan Despí", 2),
    ("Sant Martí de Centelles", 2),
    ("Sant Miquel de Gonteres", 2),
    ("Sant Pol de Mar", 2),
    ("Sant Sadurní d'Anoia", 3),
    // SVC (usuario 2026-09-24): 14 vías — 2 principales regionales, 2 principales R13, el resto no
    // principales (cabecera de R2S y R4, que dan la vuelta/aparcan ahí). Reparto en app.js CORREDORES.
    ("Sant Vicenç de Calders", 14),
    ("Sant Vicenç de Castellet", 3),
    ("Santa Susanna", 2),
    ("Segur de Calafell", 2),
    // Tarragona: principales + 8 vías no principales de estacionamiento (usuario 2026-09-24).
    ("Tarragona", 10),
    ("Sitges", 3),
    ("Terrassa Est", 2),
    ("Terrassa Estació del Nord", 6),
    ("Tordera", 3),
    ("Torelló", 3),
    ("Toses", 1),
    ("Urtx-Alp", 1),
    ("Vacarisses", 2),
    ("Vacarisses-Torreblanca", 2),
    ("Vic", 5),
    ("Viladecans", 2),
    ("Viladecavalls", 2),
    ("Vilafranca del Penedès", 4),
    ("Vilanova i la Geltrú", 8),
    ("Vilassar de Mar", 2),
];

/// Corrección de ETIQUETA DE LÍNEA de viajes mal etiquetados en el feed de Renfe (el mismo error
/// sale en Google Maps). Una línea solo tiene sus cabeceras reales: un viaje de `line` que EMPIEZA o
/// ACABA fuera de `terminals` no es de esa línea y pasa a `relabel`.
struct LineFix {
    line: &'static str,
    terminals: &'static [&'static str],
    relabel: &'static str,
}

const LINE_FIXES: &[LineFix] = &[
    // R2 Sud muere en Sant Vicenç de Calders: más allá es territorio REGIONAL. Los «R2S» a Flix y a
    // Quinto (vía Caspe) son R15 (usuario 2026-09-24): pintaban el verde de la R2S hasta el Ebro.
    LineFix {
        line: "R2S",
        terminals: &["Barcelona Estació de França", "Castelldefels", "Vilanova i la Geltrú", "Sant Vicenç de Calders"],
        relabel: "R15",
    },
    // R2 Nord: Aeroport/França ↔ Granollers/Sant Celoni/Maçanet. Los «R2N» Portbou↔Maçanet son R11
    // (la R2N no llega a Girona).
    LineFix {
        line: "R2N",
        terminals: &["El Prat Aeroport", "Barcelona Estació de França", "Granollers Centre", "Sant Celoni", "Maçanet-Massanes"],
        relabel: "R11",
    },
];

/// Colores de línea que el feed trae MAL: `(línea, color oficial)`. La R13 viene con #146520, el
/// mismo verde que la R2S; el oficial (logo de Rodalies, `game/web/lines/R13_…png`) es rosa.
const LINE_COLOR_FIXES: &[(&str, &str)] = &[("R13", "#E52E87")];

/// Aplica las correcciones del feed (etiquetas de línea y colores) a la red recién cargada.
/// Devuelve cuántos viajes se reetiquetaron.
pub fn fix_feed(net: &mut Network) -> usize {
    let name = |net: &Network, id: &str| net.stop_name(id).to_string();
    let mut changed = Vec::new();
    for (i, svc) in net.services.iter().enumerate() {
        let Some(fix) = LINE_FIXES.iter().find(|f| f.line == svc.route_short_name) else { continue };
        let (Some(first), Some(last)) = (svc.schedule.first(), svc.schedule.last()) else { continue };
        let ok = |id: &str| fix.terminals.contains(&name(net, id).as_str());
        if !ok(&first.stop_id) || !ok(&last.stop_id) {
            changed.push((i, fix.relabel));
        }
    }
    for &(i, to) in &changed {
        net.services[i].route_short_name = to.to_string();
    }
    for (line, color) in LINE_COLOR_FIXES {
        net.line_colors.insert(line.to_string(), color.to_string());
    }
    changed.len()
}

/// Configuración de vía única por ANCLA+TERMINAL: `(línea, "estación donde empieza
/// (subcadena)", "terminal del extremo de vía única (subcadena)")`. Se recorre un trayecto
/// SOLO si termina en ese extremo — así no se marca por error el tramo de vuelta hacia el
/// núcleo urbano (doble vía) cuando la estación de inicio aparece cerca del final de un
/// trayecto en sentido contrario. Válido para líneas con patrones de inicio/fin estables (sin
/// patrones truncados que no lleguen a tocar ninguno de los dos extremos).
const SINGLE_TRACK: &[(&str, &str, &str)] = &[];

/// Semáforos de tramo EXTRA por cantón de vía doble (los dos sentidos), por `stop_id`: `(a, b, n)`.
/// La regla general (`engine::calc_sub_blocks`) pone las señales por distancia (1 cada ~1,6 km); aquí
/// se declaran los cantones donde la explotación necesita más densidad de bloqueo que esa regla.
/// Cada señal extra = un sub-bloque más = un tren más que cabe en el cantón.
const EXTRA_BLOCK_SIGNALS: &[(&str, &str, u32)] = &[
    // (Vacía.) Hubo una señal extra França (79400) ↔ Passeig de Gràcia (71802) para el atasco a la
    // salida de França; la causa real era medir el tramo en línea recta (1,9 km en vez de 4,6). Con
    // el trazado real el tramo se parte en la bifurcación de Aragó y la regla por distancia ya le da
    // más señales que las que tenía. Añade aquí `(a, b, n)` si un cantón necesita más densidad.
];

/// Nº de semáforos de tramo extra del cantón `a`↔`b` (independiente del sentido); 0 si no hay.
pub fn extra_block_signals(a: &str, b: &str) -> u32 {
    EXTRA_BLOCK_SIGNALS
        .iter()
        .find(|(x, y, _)| (*x == a && *y == b) || (*x == b && *y == a))
        .map_or(0, |e| e.2)
}

/// Vía única por ZONA de `stop_id`: `(línea, [stop_ids de la zona de vía única])`. Un cantón
/// (par de paradas consecutivas de un trayecto de esa línea) se marca vía única si AMBOS
/// extremos están en la lista de su línea. Se indexa por `stop_id` (no por nombre) para evitar
/// ambigüedades de codificación/acentos y ser exacto respecto al GTFS. La lista se construye
/// SIEMPRE incluyendo la estación-ancla (p. ej. Reus, La Garriga): así el PRIMER tramo desde el
/// ancla hacia el ramal ya sale vía única, mientras que su tramo hacia el núcleo (Barcelona)
/// queda doble porque el otro extremo NO está en la zona. Fuente: usuario (sesión 2026-09-12) +
/// topología GTFS verificada segmento a segmento.
///
/// Zonas confirmadas por el usuario:
///   R1  desde Arenys de Mar hacia Maçanet-Massanes (costa norte).
///   R3  desde La Garriga hacia Puigcerdà / La Tor de Querol (antes arrancaba en Montcada Bif.).
///   R13 desde Sant Vicenç de Calders hacia Lleida por el interior (Valls–Picamoixons–Montblanc).
///   R14 desde Reus hacia Lleida por el interior (Selva del Camp–Alcover–Picamoixons).
///   R15 desde Reus hacia Zaragoza por el interior (Falset–Móra–Flix–Casp–Zaida…).
///   R17 y RT2 comparten el único tramo Salou-Port Aventura ↔ Tarragona.
///   RL3 y RL4 desde Manresa hacia Lleida por el interior (Calaf–Cervera–Tàrrega).
///   R7  ramal Cerdanyola del Vallès ↔ Cerdanyola Universitat.
/// El resto de la red se asume vía doble (incluidas todas las estaciones, que hacen de
/// apartadero/cruce), como pidió el usuario.
const SINGLE_TRACK_ZONE_IDS: &[(&str, &[&str])] = &[
    // R1: costa norte desde Arenys de Mar (79600) hasta Maçanet-Massanes (79200).
    ("R1", &["79600", "79601", "79602", "79603", "79604", "79608", "79605", "79606", "79607", "79200"]),
    // R3 (y variante sintética R3a): desde La Garriga (77102) hasta La Tor de Querol (77310).
    ("R3", &[
        "77102", "77103", "77104", "77105", "77106", "77107", "77109", "77110", "77111",
        "77112", "77113", "77114", "77200", "77301", "77303", "77304", "77305", "77306",
        "77307", "77309", "77310",
    ]),
    ("R3a", &[
        "77109", "77110", "77111", "77112", "77113", "77114", "77200", "77301", "77303",
        "77304", "77305", "77306", "77307", "77309", "77310",
    ]),
    // R13: desde Sant Vicenç de Calders (71600) por el interior (Valls–Picamoixons) hasta Lleida.
    ("R13", &[
        "71600", "72101", "76001", "76002", "76003", "76004", "73100", "73010", "73009",
        "73008", "73007", "73006", "73005", "73004", "73003", "73002", "73001", "78400",
    ]),
    // R14: desde Reus (71400) por el interior (Selva del Camp–Alcover) hasta Picamoixons.
    ("R14", &["71400", "73102", "73101", "73100"]),
    // R15: desde Reus (71400) hacia Zaragoza por el interior (Falset–Móra–Flix–Casp–Zaida).
    ("R15", &[
        "71400", "71307", "71306", "71305", "71304", "71303", "71302", "71300", "71211",
        "71210", "71209", "71208", "71207", "71206", "71205", "71204", "71201", "71200",
        "71108", "71105", "71103",
    ]),
    // R17 y RT2: único tramo Salou-Port Aventura (65411) ↔ Tarragona (71500).
    ("R17", &["65411", "71500"]),
    ("RT2", &["65411", "71500"]),
    // RL3: todo su recorrido Lleida (78400) ↔ Cervera (78500) es vía única (interior de Lleida).
    ("RL3", &["78400", "78402", "78403", "78404", "78405", "78406", "78407", "78408", "78500"]),
    // RL4: desde Manresa (78600) por el interior (Calaf–Cervera–Tàrrega) hasta Lleida (78400).
    ("RL4", &[
        "78400", "78402", "78403", "78404", "78405", "78406", "78407", "78408", "78500",
        "78501", "78502", "78503", "78504", "78505", "78506", "78600",
    ]),
    // R7: ramal de vía única Cerdanyola del Vallès (78706) ↔ Cerdanyola Universitat (72503).
    ("R7", &["72503", "78706"]),
];

/// Cizallamientos: pares de cantones que se cruzan físicamente — ocupar uno obliga a retener el
/// otro en rojo. DESACTIVADO (usuario, sesión 2026-09-12): el cizallamiento configurado (ramal
/// único de R7 ↔ cantón de R4 en Cerdanyola) hacía que el semáforo de la vía única de R7
/// obedeciera al tramo Cerdanyola↔Montcada-Santa Maria, poniéndose en rojo cada vez que pasaba
/// un R4 por al lado; eso NO es correcto y colapsaba Cerdanyola/Montcada (nudo del tronco → el
/// atasco se propagaba a toda la red). El semáforo de vía única solo debe obedecer a SU tramo.
pub const SHEAR_CONFLICTS: &[((&str, &str), (&str, &str))] = &[];

/// Líneas de Rodalies metropolitanas (cercanías, paran en todas las estaciones de su
/// itinerario). Todo lo que no esté aquí (R11, R13-R17, RG1, RL3/4, RT1/2…) se trata como
/// Regionals/Media distancia: categoría con prioridad de paso en cruces de vía única —
/// confirmado por el usuario: "los regionales que se saltan paradas hacen que los cercanías
/// se aparten en vía no principal para dejarlos pasar".
const RODALIES_METROPOLITANAS: &[&str] = &["R1", "R2", "R2N", "R2S", "R3", "R3a", "R4", "R7", "R8"];

/// `true` si `line` es un servicio Regionals/media distancia (prioridad alta en vía única).
pub fn is_regional_line(line: &str) -> bool {
    !RODALIES_METROPOLITANAS.contains(&line)
}

/// Etiqueta de línea que se muestra al usuario. La R3a es el tramo aislado del R3 (Vic–La Tor
/// de Querol): el resto del R3 está en obras y Renfe le ha dado nombre propio en el GTFS, pero
/// operativamente sigue siendo el R3. Se muestra con la identidad del R3 (icono, color, nombre)
/// aunque sus trenes sigan circulando. Los ficheros GTFS conservan "R3a" intactos: esta es la
/// ÚNICA regla de canonicalización, aplicada en las fronteras de la API (juego y calculador).
pub fn display_line(short: &str) -> &str {
    match short {
        "R3a" => "R3",
        other => other,
    }
}

#[cfg(test)]
mod display_line_tests {
    use super::display_line;
    #[test]
    fn r3a_se_pliega_sobre_r3() {
        assert_eq!(display_line("R3a"), "R3");
    }
    #[test]
    fn otras_lineas_intactas() {
        for l in ["R3", "R2", "R2N", "RG1", "RT1"] {
            assert_eq!(display_line(l), l);
        }
    }
}

/// Conjunto de segmentos (pares de nodos NO dirigidos) que son de vía única.
/// Se calcula recorriendo los trayectos de cada línea configurada y marcando todos
/// los tramos desde la estación indicada hasta el final, más los pares fijos directos.
pub fn single_track_pairs(net: &Network) -> HashSet<(NodeIndex, NodeIndex)> {
    let mut set = HashSet::new();
    for (line, from_sub, term_sub) in SINGLE_TRACK {
        let from_sub = from_sub.to_lowercase();
        let term_sub = term_sub.to_lowercase();
        for svc in net.services.iter().filter(|s| &s.route_short_name == line && !s.is_bus) {
            let Some(last) = svc.schedule.last() else { continue };
            if !net.stop_name(&last.stop_id).to_lowercase().contains(&term_sub) {
                continue; // este trayecto no llega al extremo de vía única: no tocar
            }
            // Índice de la estación a partir de la cual es vía única (en cualquier sentido).
            let idx = svc.schedule.iter().position(|st| {
                net.stop_name(&st.stop_id).to_lowercase().contains(&from_sub)
            });
            let Some(i0) = idx else { continue };
            // Desde i0 hasta el final: marca cada tramo consecutivo.
            for w in svc.schedule[i0..].windows(2) {
                if let (Some(a), Some(b)) = (net.node(&w[0].stop_id), net.node(&w[1].stop_id)) {
                    set.insert(unordered(a, b));
                }
            }
        }
    }
    for (line, zone) in SINGLE_TRACK_ZONE_IDS {
        let in_zone = |stop_id: &str| zone.contains(&stop_id);
        for svc in net.services.iter().filter(|s| &s.route_short_name == line && !s.is_bus) {
            for w in svc.schedule.windows(2) {
                if in_zone(w[0].stop_id.as_str()) && in_zone(w[1].stop_id.as_str()) {
                    if let (Some(a), Some(b)) = (net.node(&w[0].stop_id), net.node(&w[1].stop_id)) {
                        set.insert(unordered(a, b));
                    }
                }
            }
        }
    }
    set
}

/// Paradas de un cantón con las BIFURCACIONES del trazado real intercaladas (`a, J…, b`): los
/// trozos `a→J`, `J→…`, `…→b` son la vía física que comparte con otros ramales. Si el trazado solo
/// existe en el sentido contrario, se usa invertido (misma vía). Sin trazado: `[a, b]`.
pub fn canton_pieces(a: &str, b: &str) -> Vec<String> {
    let mut nodes = vec![a.to_string()];
    if let Some(g) = crate::track_geometry::global() {
        let fwd = g.via(a, b);
        if !fwd.is_empty() {
            nodes.extend(fwd.iter().map(|(j, _)| j.clone()));
        } else {
            nodes.extend(g.via(b, a).iter().rev().map(|(j, _)| j.clone()));
        }
    }
    nodes.push(b.to_string());
    nodes
}

/// Par NO dirigido de stop_id (orden lexicográfico) — clave de un trozo de vía física.
fn piece_key(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

/// Trozos de vía física (pares NO dirigidos de stop_id o bifurcación, ver [`canton_pieces`]) que son
/// de VÍA ÚNICA. Fuente única para el motor y para el visor.
///
/// Un cantón de vía única que pasa por una bifurcación se parte en ella, pero sus trozos SOLO son
/// vía única si ningún cantón de vía DOBLE pasa también por ese mismo trozo: el tramo común lo es de
/// la línea doble. Caso real: la R17 Tarragona → Salou-Port Aventura (vía única) comparte con la R15
/// Tarragona → Vila-seca (vía doble) el tramo Tarragona → Bif. Vila-seca; heredaba la vía única, y
/// dos trozos únicos seguidos con una aguja en medio (donde no se puede cruzar) interbloqueaban los
/// R15. Incluye los tramos de [`FREIGHT_ONLY_TRACK`] declarados de vía única.
pub fn single_track_pieces(net: &Network) -> HashSet<(String, String)> {
    let single = single_track_pairs(net);
    let mut out: HashSet<(String, String)> = HashSet::new();
    let mut double: HashSet<(String, String)> = HashSet::new();
    let mut seen: HashSet<(NodeIndex, NodeIndex)> = HashSet::new();
    for svc in net.services.iter().filter(|s| !s.is_bus) {
        for w in svc.schedule.windows(2) {
            let (Some(na), Some(nb)) = (net.node(&w[0].stop_id), net.node(&w[1].stop_id)) else { continue };
            if na == nb || !seen.insert((na, nb)) {
                continue;
            }
            let es_unica = single.contains(&unordered(na, nb));
            for p in canton_pieces(&w[0].stop_id, &w[1].stop_id).windows(2) {
                let k = piece_key(&p[0], &p[1]);
                if es_unica {
                    out.insert(k);
                } else {
                    double.insert(k);
                }
            }
        }
    }
    out.retain(|k| !double.contains(k));
    for (a, b, unica) in freight_only_links() {
        if unica {
            out.insert(piece_key(a, b));
        }
    }
    out
}

/// Vía que EXISTE físicamente pero por la que el GTFS vigente no lleva ningún tren de viajeros
/// (servicio sustituido por autobús por obras, o línea sin viajeros): `(descripción, vía única,
/// paradas en orden de vía)`. Solo la usan los MERCANCÍAS (`freight.rs`, enrutado) y el visor la
/// dibuja; el motor la trata como cualquier otro cantón. `scripts/fetch_osm_track.py` repite esta
/// lista (`FREIGHT_ONLY_TRACK`) para trazar también su vía real: si la cambias, cámbiala allí.
pub const FREIGHT_ONLY_TRACK: &[(&str, bool, &[&str])] = &[
    // Lleida – La Plana-Picamoixons por Montblanc (R13/R14 con autobús en el feed): itinerario de los
    // mercancías de Lleida hacia el corredor (Valls – Sant Vicenç de Calders). Vía única, como el
    // resto de la línea del interior (ver la zona R13 de `SINGLE_TRACK_ZONE_IDS`).
    ("La Plana-Picamoixons – Les Borges Blanques (por Montblanc)", true, &[
        "73100", "73010", "73009", "73008", "73007", "73006", "73005", "73004", "73003",
    ]),
];

/// Pares consecutivos `(a, b, vía única)` de [`FREIGHT_ONLY_TRACK`] (un sentido; la vía es la misma).
pub fn freight_only_links() -> impl Iterator<Item = (&'static str, &'static str, bool)> {
    FREIGHT_ONLY_TRACK
        .iter()
        .flat_map(|(_, unica, ids)| ids.windows(2).map(move |w| (w[0], w[1], *unica)))
}

fn node_by_name(net: &Network, sub: &str) -> Option<NodeIndex> {
    net.find_stop_by_name(sub).and_then(|n| net.node(&n.stop_id))
}

/// Resuelve `SHEAR_CONFLICTS` a pares de aristas dirigidas concretas (stop_id, stop_id), en
/// AMBOS sentidos de cada cantón (el cizallamiento aplica independientemente del sentido de
/// circulación). Vacío si alguna estación no se encuentra en la red cargada.
pub fn shear_conflicts(net: &Network) -> Vec<((String, String), (String, String))> {
    let mut out = Vec::new();
    for ((a1, b1), (a2, b2)) in SHEAR_CONFLICTS {
        let (Some(n_a1), Some(n_b1), Some(n_a2), Some(n_b2)) =
            (node_by_name(net, a1), node_by_name(net, b1), node_by_name(net, a2), node_by_name(net, b2))
        else {
            continue;
        };
        let id = |n: NodeIndex| net.graph[n].stop_id.clone();
        out.push(((id(n_a1), id(n_b1)), (id(n_a2), id(n_b2))));
    }
    out
}

/// Un corredor: grupo de vías físicas compartido SOLO por un subconjunto de líneas (no toda
/// estación reparte sus vías en un único pool). Sustituye, donde el reparto está confirmado
/// por fuente, al conteo plano de `platform_tracks`.
pub struct Corridor {
    pub lines: &'static [&'static str],
    pub tracks: u32,
}

/// Corredores por estación (subcadena del nombre). Solo se rellena donde el reparto
/// línea→vías está confirmado por fuente (trenscat.com, sesión 2026-09-10); el resto de
/// estaciones sigue usando el pool único de `platform_tracks` (no se inventa un reparto que
/// no está publicado — p. ej. Barcelona-Sants: se confirmó el reparto por TÚNEL, 7-10 vs.
/// 11-14, pero no línea por línea dentro de cada grupo de 4, así que no se modela aquí).
pub fn corridors(stop_name: &str) -> Option<Vec<Corridor>> {
    let n = stop_name.to_lowercase();
    if n.contains("montcada-bifurcació") || n.contains("montcada bifurcació")
        || n.contains("montcada-bifurcacio") || n.contains("montcada bifurcacio")
    {
        // trenscat.com: vía 1 = general R3 (aislada); vías 2-5 compartidas por R4+R7 hacia
        // Cerdanyola/Manresa. El resto de vías de `platform_tracks` (15 en total, Godot) son
        // desviadas/patio de estacionamiento — no forman parte de estos corredores de PASO,
        // pero sí cuentan en el total (ahí es donde pernoctan/dan la vuelta los trenes).
        Some(vec![
            Corridor { lines: &["R3"], tracks: 1 },
            Corridor { lines: &["R4", "R7"], tracks: 4 },
        ])
    } else {
        None
    }
}

/// Par de nodos en orden canónico (no dirigido).
pub fn unordered(a: NodeIndex, b: NodeIndex) -> (NodeIndex, NodeIndex) {
    if a.index() <= b.index() {
        (a, b)
    } else {
        (b, a)
    }
}
