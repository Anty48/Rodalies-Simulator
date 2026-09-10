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
    ("Cornellà", 2),
    ("Cubelles", 2),
    ("Cunit", 2),
    ("El Masnou", 3),
    ("El Papiol", 2),
    ("El Prat de Llobregat", 4),
    ("El Vendrell", 4),
    ("Els Monjos", 2),
    ("Figaró", 2),
    ("Garraf", 2),
    ("Gavà", 4),
    ("Gelida", 2),
    ("Granollers Centre", 9),
    ("Granollers-Canovelles", 3),
    ("Gualba", 2),
    ("Hostalric", 2),
    ("L'Arboç", 3),
    ("L'Hospitalet de Llobregat", 20),
    ("La Farga de Bebié", 2),
    ("La Garriga", 3),
    ("La Granada", 2),
    ("La Llagosta", 2),
    ("La Molina", 3),
    ("La Tor de Querol-Enveig", 3),
    ("Lavern-Subirats", 2),
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
    ("Sant Vicenç de Calders", 9),
    ("Sant Vicenç de Castellet", 3),
    ("Santa Susanna", 2),
    ("Segur de Calafell", 2),
    ("Sitges", 3),
    ("Terrassa Est", 2),
    ("Terrassa Estació del Nord", 5),
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

/// Configuración de vía única por ANCLA+TERMINAL: `(línea, "estación donde empieza
/// (subcadena)", "terminal del extremo de vía única (subcadena)")`. Se recorre un trayecto
/// SOLO si termina en ese extremo — así no se marca por error el tramo de vuelta hacia el
/// núcleo urbano (doble vía) cuando la estación de inicio aparece cerca del final de un
/// trayecto en sentido contrario. Válido para líneas con patrones de inicio/fin estables (sin
/// patrones truncados que no lleguen a tocar ninguno de los dos extremos).
const SINGLE_TRACK: &[(&str, &str, &str)] = &[];

/// Vía única por ZONA: `(línea, [estaciones de la zona (subcadena)])`. Un cantón se marca
/// vía única si AMBOS extremos están en la lista de su línea. Más robusto que ancla+terminal
/// cuando el GTFS solo tiene patrones truncados que no llegan a tocar la estación de anclaje
/// real (caso actual: R3 circula hoy solo dentro de la zona norte, sin ningún viaje que pase
/// por Montcada Bifurcació — probablemente por las obras de remodelación de la bifurcación).
/// Lista de estaciones: trenscat.com + confirmación del usuario (toda la R3 desde Montcada
/// Bif es vía única con apartaderos; sesión 2026-09-10).
const SINGLE_TRACK_ZONE: &[(&str, &[&str])] = &[
    (
        "R3",
        &[
            "montcada bifurcació", "montcada bifurcacio",
            "granollers-canovelles", "montmeló nord", "montmelo nord",
            "les franqueses del vallès", "les franqueses del valles", "llerona",
            "la garriga", "figaró", "figaro",
            "sant martí de centelles", "sant marti de centelles",
            "centelles", "balenyà", "balenya",
            "vic", "ripoll", "ribes de freser", "puigcerdà", "puigcerda",
            "la tor de querol",
        ],
    ),
    (
        // R3a: continuación física de la vía única de R3 más al norte.
        "R3a",
        &["vic", "ripoll", "ribes de freser", "puigcerdà", "puigcerda", "la tor de querol"],
    ),
    (
        // R13: ramal interior Lleida—Reus (vía única confirmada; PAET en Vimbodí-Vinaixa y
        // Vinaixa-La Floresta). El servicio activo varía (hoy: Les Borges Blanques↔Lleida-
        // Pirineus), así que se usa zona en vez de ancla+terminal fijos.
        "R13",
        &[
            "lleida", "vinaixa", "les borges blanques", "juneda",
            "puigverd de lleida", "la floresta", "vimbodí", "vimbodi",
        ],
    ),
];

/// Pares de estaciones (por nombre, subcadena) fijados directamente como vía única,
/// independientes de si el GTFS tiene ahora mismo algún viaje activo para esa línea (p. ej.
/// el ramal de R7 a Cerdanyola Universitat, con servicio reducido/suprimido según el feed).
/// Confirmado por fuente (cerdanyola.info, sesión 2026-09-10): ramal de vía única real.
const SINGLE_TRACK_FIXED: &[(&str, &str)] = &[
    ("cerdanyola del vallès", "cerdanyola universitat"),
];

/// Cizallamientos: pares de cantones (cada uno como sus dos estaciones) que se cruzan
/// físicamente — ocupar uno obliga a retener el otro en rojo. Cerdanyola (confirmado,
/// cerdanyola.info): el ramal único de R7 hacia Cerdanyola Universitat invade la vía
/// contraria de R4 a la altura de Cerdanyola del Vallès.
pub const SHEAR_CONFLICTS: &[((&str, &str), (&str, &str))] = &[
    (
        ("montcada i reixac-santa maria", "cerdanyola del vallès"), // cantón de R4
        ("cerdanyola del vallès", "cerdanyola universitat"),        // ramal único de R7
    ),
];

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
    for (line, zone) in SINGLE_TRACK_ZONE {
        let in_zone = |stop_id: &str| {
            let name = net.stop_name(stop_id).to_lowercase();
            zone.iter().any(|z| name.contains(z))
        };
        for svc in net.services.iter().filter(|s| &s.route_short_name == line && !s.is_bus) {
            for w in svc.schedule.windows(2) {
                if in_zone(&w[0].stop_id) && in_zone(&w[1].stop_id) {
                    if let (Some(a), Some(b)) = (net.node(&w[0].stop_id), net.node(&w[1].stop_id)) {
                        set.insert(unordered(a, b));
                    }
                }
            }
        }
    }
    for (a_sub, b_sub) in SINGLE_TRACK_FIXED {
        if let (Some(a), Some(b)) = (node_by_name(net, a_sub), node_by_name(net, b_sub)) {
            set.insert(unordered(a, b));
        }
    }
    set
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
