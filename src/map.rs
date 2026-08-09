//! Mapa geográfico de la red (SVG) construido con las coordenadas lat/lon del GTFS.
//! Dibuja las estaciones y los cantones, y anima trenes moviéndose a lo largo de sus
//! rutas reales usando SMIL (`animateMotion`), de modo que funciona sin JavaScript y
//! también en el dashboard estático offline.

use std::collections::HashMap;
use std::f64::consts::PI;

use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;

use crate::gtfs_loader::Network;
use crate::scenario::SimParams;

const W: f64 = 1000.0;
const H: f64 = 720.0;
const PAD: f64 = 28.0;
/// Duración del bucle de animación (segundos) = toda la ventana comprimida.
const LOOP_SECS: f64 = 45.0;
/// Máximo de trenes animados (para no recargar el SVG).
const MAX_TRAINS: usize = 30;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Color determinista por línea (hue a partir del nombre).
fn line_color(line: &str) -> String {
    let h: u32 = line.bytes().fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    format!("hsl({}, 70%, 60%)", h % 360)
}

pub fn network_map_svg(net: &Network, params: &SimParams) -> String {
    // 1. Coordenadas de los nodos.
    let mut coords: HashMap<NodeIndex, (f64, f64)> = HashMap::new();
    for idx in net.graph.node_indices() {
        let n = &net.graph[idx];
        if let (Some(lat), Some(lon)) = (n.lat, n.lon) {
            coords.insert(idx, (lat, lon));
        }
    }
    if coords.len() < 2 {
        return "<p class=\"muted\">Sense coordenades a l'stops.txt per dibuixar el mapa.</p>".into();
    }

    // 2. Proyección equirectangular con corrección de longitud por latitud media.
    let mlat = coords.values().map(|(la, _)| *la).sum::<f64>() / coords.len() as f64;
    let k = (mlat * PI / 180.0).cos();
    let proj: HashMap<NodeIndex, (f64, f64)> =
        coords.iter().map(|(&i, &(la, lo))| (i, (lo * k, la))).collect();
    let (mut xmin, mut xmax, mut ymin, mut ymax) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for &(x, y) in proj.values() {
        xmin = xmin.min(x);
        xmax = xmax.max(x);
        ymin = ymin.min(y);
        ymax = ymax.max(y);
    }
    let sx = (W - 2.0 * PAD) / (xmax - xmin).max(1e-9);
    let sy = (H - 2.0 * PAD) / (ymax - ymin).max(1e-9);
    let s = sx.min(sy);
    // Centrado.
    let ox = PAD + ((W - 2.0 * PAD) - (xmax - xmin) * s) / 2.0;
    let oy = PAD + ((H - 2.0 * PAD) - (ymax - ymin) * s) / 2.0;
    let to_screen = |i: NodeIndex| -> Option<(f64, f64)> {
        proj.get(&i).map(|&(x, y)| {
            (ox + (x - xmin) * s, oy + (ymax - y) * s) // y invertida
        })
    };

    // 3. Cantones (aristas).
    let mut edges = String::new();
    for e in net.graph.edge_references() {
        if let (Some((x1, y1)), Some((x2, y2))) = (to_screen(e.source()), to_screen(e.target())) {
            edges.push_str(&format!(
                "<line x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" class=\"mapedge\"/>",
                x1, y1, x2, y2
            ));
        }
    }

    // 4. Estaciones (nodos). Las clave, más grandes y con etiqueta.
    let key = ["clot", "arc de triomf", "sants", "passeig de gràcia", "plaça de catalunya", "estació de frança"];
    let mut stations = String::new();
    let mut labels = String::new();
    for idx in net.graph.node_indices() {
        let Some((x, y)) = to_screen(idx) else { continue };
        let name = &net.graph[idx].stop_name;
        let is_key = key.iter().any(|k| name.to_lowercase().contains(k));
        if is_key {
            stations.push_str(&format!(
                "<circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"5\" class=\"mapkey\"><title>{}</title></circle>",
                x, y, esc(name)
            ));
            labels.push_str(&format!(
                "<text x=\"{:.1}\" y=\"{:.1}\" class=\"maplabel\">{}</text>",
                x + 7.0, y + 3.0, esc(name)
            ));
        } else {
            stations.push_str(&format!(
                "<circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"2.2\" class=\"mapstop\"><title>{}</title></circle>",
                x, y, esc(name)
            ));
        }
    }

    // 5. Trenes animados a lo largo de sus rutas reales.
    let service_id = net.dominant_service().unwrap_or_default();
    let span = (params.end_sec.saturating_sub(params.start_sec)).max(1) as f64;
    let mut participants: Vec<usize> = net
        .services
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            s.service_id == service_id
                && params.line.as_ref().map_or(true, |l| &s.route_short_name == l)
                && matches!(s.first_time(), Some(t) if t >= params.start_sec && t <= params.end_sec)
        })
        .map(|(i, _)| i)
        .collect();
    // Muestreo uniforme hasta MAX_TRAINS.
    let stepn = (participants.len() / MAX_TRAINS).max(1);
    participants = participants.into_iter().step_by(stepn).take(MAX_TRAINS).collect();

    let mut trains = String::new();
    for idx in participants {
        let svc = &net.services[idx];
        // Puntos (x,y) y tiempos de las paradas con coordenadas.
        let mut pts: Vec<(f64, f64, u32)> = Vec::new();
        for st in &svc.schedule {
            if let Some(&node) = net.node_of_stop.get(&st.stop_id) {
                if let Some((x, y)) = to_screen(node) {
                    pts.push((x, y, st.arrival_sec));
                }
            }
        }
        if pts.len() < 2 {
            continue;
        }
        // Path y distancias acumuladas.
        let mut path = format!("M {:.1} {:.1}", pts[0].0, pts[0].1);
        let mut cum = vec![0.0f64];
        for w in pts.windows(2) {
            path.push_str(&format!(" L {:.1} {:.1}", w[1].0, w[1].1));
            let d = ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt();
            cum.push(cum.last().unwrap() + d);
        }
        let total = *cum.last().unwrap();
        if total < 1.0 {
            continue;
        }
        // keyTimes (tiempo normalizado en la ventana) y keyPoints (distancia normalizada).
        // El tren espera en el origen hasta su hora de salida y en el destino tras llegar.
        let f = |t: u32| ((t as f64 - params.start_sec as f64) / span).clamp(0.0, 1.0);
        let mut kt = vec![0.0];
        let mut kp = vec![0.0];
        for (i, p) in pts.iter().enumerate() {
            kt.push(f(p.2));
            kp.push(cum[i] / total);
        }
        kt.push(1.0);
        kp.push(1.0);
        // keyTimes debe ser no decreciente.
        for i in 1..kt.len() {
            if kt[i] < kt[i - 1] {
                kt[i] = kt[i - 1];
            }
        }
        let kt_s = kt.iter().map(|v| format!("{:.4}", v)).collect::<Vec<_>>().join(";");
        let kp_s = kp.iter().map(|v| format!("{:.4}", v)).collect::<Vec<_>>().join(";");
        let color = line_color(&svc.route_short_name);
        trains.push_str(&format!(
            "<circle r=\"3.4\" fill=\"{color}\" stroke=\"#0e1116\" stroke-width=\"0.6\"><title>Tren {} ({})</title>\
             <animateMotion dur=\"{dur}s\" repeatCount=\"indefinite\" calcMode=\"linear\" \
             keyTimes=\"{kt_s}\" keyPoints=\"{kp_s}\" path=\"{path}\"/></circle>",
            esc(&svc.train_number),
            esc(&svc.route_short_name),
            color = color,
            dur = LOOP_SECS,
            kt_s = kt_s,
            kp_s = kp_s,
            path = path,
        ));
    }

    format!(
        r##"<svg viewBox="0 0 {W} {H}" preserveAspectRatio="xMidYMid meet" class="map" role="img" aria-label="Mapa de la xarxa Rodalies">
  <style>
    .mapedge {{ stroke:var(--border); stroke-width:1.1; }}
    .mapstop {{ fill:var(--muted); }}
    .mapkey  {{ fill:var(--accent); }}
    .maplabel {{ fill:var(--text); font-size:11px; font-family:'Segoe UI',sans-serif; }}
  </style>
  {edges}
  {stations}
  {labels}
  {trains}
</svg>"##,
        W = W as u32,
        H = H as u32,
        edges = edges,
        stations = stations,
        labels = labels,
        trains = trains,
    )
}
