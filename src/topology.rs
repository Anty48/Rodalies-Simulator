//! Datos operativos de infraestructura que el GTFS no aporta: número de vías por
//! estación y tramos de vía única. Derivados de conocimiento operativo y del simulador
//! de referencia (Godot: `estaciones.json` `vias`, `lineas.json` `via_unica_desde`).

use std::collections::HashSet;

use petgraph::graph::NodeIndex;

use crate::gtfs_loader::Network;

/// Nº de vías/andenes de una estación (capacidad de nodo en modo estricto).
/// Los grandes nudos tienen muchas vías; el resto, 2 por defecto (vía doble típica).
pub fn platform_tracks(stop_name: &str) -> u32 {
    let n = stop_name.to_lowercase();
    let has = |k: &str| n.contains(k);
    if has("barcelona-sants") || has("barcelona sants") {
        14
    } else if has("l'hospitalet") || has("hospitalet de llobregat") {
        8
    } else if has("estació de frança") || has("estacio de franca") {
        7
    } else if has("tarragona") {
        5
    } else if has("el clot") || has("girona") || has("granollers") || has("manresa")
        || has("sant vicenç de calders") || has("sant vicenc de calders")
        || has("mataró") || has("mataro") || has("figueres") || has("vilanova")
    {
        4
    } else if has("plaça de catalunya") || has("placa de catalunya")
        || has("passeig de gràcia") || has("arc de triomf") || has("sagrera")
    {
        2
    } else {
        2
    }
}

/// Configuración de vía única: `(línea, "a partir de esta estación (subcadena)")`.
/// Desde esa estación hacia el final del recorrido la línea es de vía única (un único
/// tren por tramo, en cualquiera de los dos sentidos — testigo/bastón piloto).
const SINGLE_TRACK: &[(&str, &str)] = &[
    // R3 al nord de Montcada Bifurcació (cap a Vic / Puigcerdà / La Tor de Querol).
    ("R3", "montcada bifurcació"),
];

/// Conjunto de segmentos (pares de nodos NO dirigidos) que son de vía única.
/// Se calcula recorriendo los trayectos de cada línea configurada y marcando todos
/// los tramos desde la estación indicada hasta el final.
pub fn single_track_pairs(net: &Network) -> HashSet<(NodeIndex, NodeIndex)> {
    let mut set = HashSet::new();
    for (line, from_sub) in SINGLE_TRACK {
        let from_sub = from_sub.to_lowercase();
        for svc in net.services.iter().filter(|s| &s.route_short_name == line && !s.is_bus) {
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
            // Un trayecto (el más largo) por sentido basta, pero recorrer todos es barato
            // y cubre todas las ramas (Vic, Puigcerdà, La Tor de Querol…).
        }
    }
    set
}

/// Par de nodos en orden canónico (no dirigido).
pub fn unordered(a: NodeIndex, b: NodeIndex) -> (NodeIndex, NodeIndex) {
    if a.index() <= b.index() {
        (a, b)
    } else {
        (b, a)
    }
}
