//! Servidor web local (tokio) para la UI interactiva. Sirve:
//!   * `GET /`                    → página con controles + dashboard inicial + JS.
//!   * `GET /api/render`          → fragmento HTML del dashboard para unos parámetros.
//!   * `GET /api/optimize/start`  → lanza la optimización de una línea en segundo plano.
//!   * `GET /api/optimize/status` → progreso en vivo (JSON) de la optimización.
//!   * `GET /report/optimized/…`  → descarga los CSV/PDF generados.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::gtfs_loader::Network;
use crate::optimizer::{self, PotentialWeights, SystemSearch};
use crate::report::{self, Controls};
use crate::scenario::{self, now_utc_string, SimParams};

#[derive(Debug, Clone, Serialize, Default)]
pub struct LineFile {
    pub line: String,
    pub csv: String,
    pub pdf: String,
    pub offset_min: i64,
}

/// Estado del trabajo de optimización del sistema (compartido con la UI).
#[derive(Debug, Clone, Serialize, Default)]
pub struct OptJob {
    pub running: bool,
    pub done: bool,
    pub iter: usize,
    pub total: usize,
    pub base_v: f64,
    pub current_v: f64,
    pub best_v: f64,
    pub delta_pct: f64,
    pub base_delay: f64,
    pub best_delay: f64,
    pub base_recovery: f64,
    pub best_recovery: f64,
    pub trips: usize,
    pub history: Vec<f64>,
    pub files: Vec<LineFile>,
    pub error: Option<String>,
}

pub struct ServerState {
    pub net: Arc<Network>,
    pub load_ms: f64,
    pub lines: Vec<String>,
    pub opt: Arc<Mutex<OptJob>>,
    /// Caché de análisis de línea (clave = query string) para no recalcular.
    pub line_cache: Mutex<HashMap<String, String>>,
    /// Infraestructura ADIF (CVM + geometría) si está el fichero procesado.
    pub adif: Option<crate::calculator::infrastructure::AdifNet>,
    /// LTV (temporales) — recargable en caliente vía /api/ltv/reload.
    pub ltv: std::sync::RwLock<Option<Arc<crate::calculator::ltv::LtvSet>>>,
}

pub async fn serve(state: Arc<ServerState>, port: u16) -> std::io::Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    println!("\n🌐 Servidor interactiu a  http://127.0.0.1:{port}");
    println!("   (Ctrl+C per aturar)\n");
    loop {
        let (stream, _) = listener.accept().await?;
        let st = state.clone();
        tokio::spawn(async move {
            let _ = handle(stream, st).await;
        });
    }
}

async fn handle(mut stream: TcpStream, state: Arc<ServerState>) -> std::io::Result<()> {
    let mut buf = vec![0u8; 8192];
    let n = stream.read(&mut buf).await?;
    if n == 0 {
        return Ok(());
    }
    let req = String::from_utf8_lossy(&buf[..n]);
    let target = req
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("/");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));

    let (status, ctype, body): (&str, &str, Vec<u8>) = match path {
        "/" => ("200 OK", "text/html", render_page(&state).into_bytes()),
        "/api/render" => ("200 OK", "text/html", render_fragment(&state, query).into_bytes()),
        "/api/mintime" => ("200 OK", "text/html", render_mintime(&state, query).into_bytes()),
        "/api/lines" => ("200 OK", "application/json", lines_json(&state).into_bytes()),
        "/api/stations" => ("200 OK", "application/json", stations_json(&state).into_bytes()),
        "/api/line" => ("200 OK", "application/json", line_json(&state, query).into_bytes()),
        "/api/ltv/status" => ("200 OK", "application/json", ltv_status(&state).into_bytes()),
        "/api/ltv/reload" => ("200 OK", "application/json", ltv_reload(&state).into_bytes()),
        "/api/optimize/start" => {
            ("200 OK", "application/json", start_optimization(&state, query).into_bytes())
        }
        "/api/optimize/status" => {
            let j = state.opt.lock().unwrap().clone();
            ("200 OK", "application/json", serde_json::to_vec(&j).unwrap_or_default())
        }
        "/health" => ("200 OK", "text/plain", b"ok".to_vec()),
        p if p.starts_with("/report/optimized/") => match serve_file(p) {
            Some((ct, bytes)) => ("200 OK", ct, bytes),
            None => ("404 Not Found", "text/plain", b"404".to_vec()),
        },
        _ => ("404 Not Found", "text/plain", b"404".to_vec()),
    };

    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(&body).await?;
    stream.flush().await?;
    Ok(())
}

/// Sirve un archivo generado bajo report/optimized/ (CSV o PDF), con nombre saneado.
fn serve_file(path: &str) -> Option<(&'static str, Vec<u8>)> {
    let name = path.strip_prefix("/report/optimized/")?;
    if name.is_empty()
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        || name.contains("..")
    {
        return None;
    }
    let ct = if name.ends_with(".pdf") {
        "application/pdf"
    } else if name.ends_with(".csv") {
        "text/csv"
    } else {
        return None;
    };
    let bytes = std::fs::read(std::path::Path::new("report/optimized").join(name)).ok()?;
    Some((ct, bytes))
}

fn render_page(state: &ServerState) -> String {
    let net = &state.net;
    let p = SimParams::default();
    let summary = scenario::summary_view(net, state.load_ms);
    let example = scenario::example_view(net, "25412", p.line.as_deref());
    let sim = scenario::sim_view(net, &p);
    let res = scenario::res_view(net, &p);
    let controls = Controls {
        start_h: p.start_sec / 3600,
        dur_h: (p.end_sec - p.start_sec) / 3600,
        line: p.line.clone(),
        block: p.block_min,
        delay: p.delay_min,
        cap: p.platform_capacity,
        headway: p.min_block_headway,
        random: p.random,
    };
    let stations = crate::calculator::infrastructure::station_list(net);
    let calc_panel = crate::calculator::render::panel(&stations);
    report::render_interactive_page(
        &summary, &example, &sim, &res, &state.lines, &controls, &now_utc_string(), &calc_panel,
    )
}

/// Fragmento de resultados del calculador de tiempo mínimo.
fn render_mintime(state: &ServerState, query: &str) -> String {
    let q = parse_query(query);
    let net = &state.net;
    let origin = q.get("origin").cloned().unwrap_or_default();
    let dest = q.get("dest").cloned().unwrap_or_default();
    let dt: f64 = q.get("dt").and_then(|v| v.parse().ok()).unwrap_or(0.1);
    let series: Vec<String> = q
        .get("series")
        .map(|s| s.split(',').filter(|x| !x.is_empty()).map(|x| x.to_string()).collect())
        .unwrap_or_default();
    let apply_ltv = q.get("ltv").map(|v| v == "1").unwrap_or(false);
    let ltv = if apply_ltv { state.ltv.read().ok().and_then(|g| g.clone()) } else { None };
    let view = crate::calculator::compute(
        net, &origin, &dest, &series, dt, state.adif.as_ref(), ltv.as_deref(),
    );
    crate::calculator::render::fragment(&view)
}

// -------------------------------------------------------------------------
// Calculador · análisis de línea completa (JSON)
// -------------------------------------------------------------------------

#[derive(Serialize)]
struct DirOut {
    d0: String,
    d1: String,
    label: String,
    n: usize,
}
#[derive(Serialize)]
struct LineOut {
    line: String,
    directions: Vec<DirOut>,
}

/// Lista de líneas ferroviarias (no bus) con sus sentidos.
fn lines_json(state: &ServerState) -> String {
    let net = &state.net;
    let mut out: Vec<LineOut> = Vec::new();
    for line in &state.lines {
        let dirs = crate::calculator::schedules::line_directions(net, line);
        if dirs.is_empty() {
            continue; // línea sin servicios de tren (p.ej. sólo buses)
        }
        let directions = dirs
            .into_iter()
            .filter(|d| d.n_services >= 2)
            .map(|d| DirOut {
                d0: d.key.0,
                d1: d.key.1,
                label: d.label,
                n: d.n_services,
            })
            .collect::<Vec<_>>();
        if !directions.is_empty() {
            out.push(LineOut { line: line.clone(), directions });
        }
    }
    serde_json::to_string(&out).unwrap_or_else(|_| "[]".into())
}

#[derive(Serialize)]
struct StationOut {
    id: String,
    name: String,
    lat: f64,
    lon: f64,
}

/// Todas las estaciones con coordenadas que participan en algún servicio (para el mapa).
fn stations_json(state: &ServerState) -> String {
    let net = &state.net;
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<StationOut> = Vec::new();
    for svc in &net.services {
        for s in &svc.schedule {
            if !seen.insert(s.stop_id.clone()) {
                continue;
            }
            if let Some(n) = net.node(&s.stop_id) {
                let sn = &net.graph[n];
                if let (Some(lat), Some(lon)) = (sn.lat, sn.lon) {
                    out.push(StationOut {
                        id: sn.stop_id.clone(),
                        name: sn.stop_name.clone(),
                        lat,
                        lon,
                    });
                }
            }
        }
    }
    serde_json::to_string(&out).unwrap_or_else(|_| "[]".into())
}

/// Análisis de línea completa (con caché por query string).
fn line_json(state: &ServerState, query: &str) -> String {
    // Caché.
    if let Ok(cache) = state.line_cache.lock() {
        if let Some(hit) = cache.get(query) {
            return hit.clone();
        }
    }
    let net = &state.net;
    let q = parse_query(query);
    let line = q.get("line").cloned().unwrap_or_default();

    // Sentido: (d0,d1) o el primero disponible.
    let dirs = crate::calculator::schedules::line_directions(net, &line);
    let key = match (q.get("d0"), q.get("d1")) {
        (Some(a), Some(b)) if !a.is_empty() && !b.is_empty() => (a.clone(), b.clone()),
        _ => match dirs.first() {
            Some(d) => d.key.clone(),
            None => {
                return serde_json::json!({"error":"Línia sense sentits al GTFS."}).to_string()
            }
        },
    };

    let series: Vec<String> = q
        .get("series")
        .map(|s| s.split(',').filter(|x| !x.is_empty()).map(|x| x.to_string()).collect())
        .unwrap_or_else(|| vec!["447".into(), "450".into(), "470".into(), "490".into()]);

    let dt: f64 = q.get("dt").and_then(|v| v.parse().ok()).unwrap_or(0.1);

    let dwell = match q.get("dwell").map(|s| s.as_str()) {
        Some("fixed") => {
            let s = q.get("dwell_s").and_then(|v| v.parse().ok()).unwrap_or(30);
            crate::calculator::line_analysis::DwellMode::Fixed(s)
        }
        Some("custom") => {
            let mut m = std::collections::HashMap::new();
            if let Some(c) = q.get("custom") {
                for pair in c.split(',') {
                    if let Some((id, sec)) = pair.split_once(':') {
                        if let Ok(v) = sec.parse::<u32>() {
                            m.insert(id.to_string(), v);
                        }
                    }
                }
            }
            crate::calculator::line_analysis::DwellMode::Custom(m)
        }
        _ => crate::calculator::line_analysis::DwellMode::Auto,
    };

    let apply_ltv = q.get("ltv").map(|v| v == "1").unwrap_or(false);
    let ltv = if apply_ltv { state.ltv.read().ok().and_then(|g| g.clone()) } else { None };
    let analysis = crate::calculator::line_analysis::analyze_line(
        net, &line, &key, &series, &dwell, dt, state.adif.as_ref(), ltv.as_deref(),
    );
    let json = serde_json::to_string(&analysis).unwrap_or_else(|_| "{}".into());

    if analysis.error.is_none() {
        if let Ok(mut cache) = state.line_cache.lock() {
            if cache.len() > 200 {
                cache.clear();
            }
            cache.insert(query.to_string(), json.clone());
        }
    }
    json
}

/// Estado actual de las LTV cargadas.
fn ltv_status(state: &ServerState) -> String {
    match state.ltv.read().ok().and_then(|g| g.clone()) {
        Some(l) => serde_json::json!({"available":true,"snapshot":l.snapshot,"count":l.count(),"source":l.source}).to_string(),
        None => serde_json::json!({"available":false}).to_string(),
    }
}

/// Recarga las LTV desde raw/ltv (ZIP diario) o processed/adif/ltv.json y limpia la caché.
fn ltv_reload(state: &ServerState) -> String {
    let fresh = crate::calculator::ltv::load_default().map(Arc::new);
    let out = match &fresh {
        Some(l) => serde_json::json!({"ok":true,"snapshot":l.snapshot,"count":l.count(),"source":l.source}).to_string(),
        None => serde_json::json!({"ok":false,"reason":"no s'ha trobat cap dada LTV (raw/ltv o processed/adif/ltv.json)"}).to_string(),
    };
    if let Ok(mut g) = state.ltv.write() {
        *g = fresh;
    }
    if let Ok(mut c) = state.line_cache.lock() {
        c.clear(); // los análisis cacheados pueden depender de LTV
    }
    out
}

fn render_fragment(state: &ServerState, query: &str) -> String {
    let net = &state.net;
    let p = params_from_query(&parse_query(query));
    let summary = scenario::summary_view(net, state.load_ms);
    let example = scenario::example_view(net, "25412", p.line.as_deref());
    let sim = scenario::sim_view(net, &p);
    let res = scenario::res_view(net, &p);
    report::render_body(&summary, &example, &sim, &res, &now_utc_string())
}

/// Lanza la optimización del SISTEMA en un hilo aparte (si no hay otra en curso).
fn start_optimization(state: &ServerState, _query: &str) -> String {
    {
        let mut job = state.opt.lock().unwrap();
        if job.running {
            return "{\"started\":false,\"reason\":\"ja hi ha una optimització en curs\"}".into();
        }
        *job = OptJob {
            running: true,
            total: SystemSearch::default().iters,
            ..Default::default()
        };
    }
    let net = state.net.clone();
    let opt = state.opt.clone();
    std::thread::spawn(move || run_optimization(net, opt));
    "{\"started\":true}".into()
}

fn run_optimization(net: Arc<Network>, opt: Arc<Mutex<OptJob>>) {
    let sc = SystemSearch::default();
    let w = PotentialWeights::default();

    let job = opt.clone();
    let cb = move |it: usize, cur: f64, best: f64| {
        let mut j = job.lock().unwrap();
        j.iter = it;
        j.current_v = cur;
        j.best_v = best;
        if it == 0 {
            j.base_v = best;
        }
        j.history.push(best);
    };

    let res = optimizer::optimize_system(&net, sc, w, &cb);

    let mut j = opt.lock().unwrap();
    match res {
        Some(r) => {
            let service_id = net.dominant_service().unwrap_or_default();
            let dir = std::path::Path::new("report").join("optimized");
            let files = crate::exporter::export_system(&net, &r, &service_id, sc.window, &dir);
            j.base_v = r.base_v;
            j.best_v = r.best_v;
            j.delta_pct = r.delta_pct;
            j.base_delay = r.base_delay;
            j.best_delay = r.best_delay;
            j.base_recovery = r.base_recovery_min;
            j.best_recovery = r.best_recovery_min;
            j.trips = r.trips;
            j.files = files
                .into_iter()
                .map(|(line, csv, pdf)| LineFile {
                    offset_min: r.offsets.get(&line).copied().unwrap_or(0) / 60,
                    line,
                    csv: file_url(&csv),
                    pdf: file_url(&pdf),
                })
                .collect();
        }
        None => j.error = Some("No hi ha prou trens per optimitzar.".into()),
    }
    j.running = false;
    j.done = true;
}

fn file_url(p: &std::path::Path) -> String {
    format!("/report/optimized/{}", p.file_name().unwrap().to_string_lossy())
}

fn params_from_query(q: &HashMap<String, String>) -> SimParams {
    let get_u = |k: &str, def: u32| q.get(k).and_then(|v| v.parse().ok()).unwrap_or(def);
    let start_h = get_u("start_h", 7).min(23);
    let dur_h = get_u("dur_h", 2).clamp(1, 6);
    let line = q.get("line").filter(|v| !v.is_empty()).cloned();
    SimParams {
        start_sec: start_h * 3600,
        end_sec: (start_h + dur_h) * 3600,
        line,
        block_min: get_u("block", 12).min(60),
        delay_min: get_u("delay", 8).min(60),
        platform_capacity: get_u("cap", 4).clamp(1, 12),
        min_block_headway: get_u("headway", 120).clamp(30, 600),
        base_arrival_rate: 4.0,
        random: q.get("random").map(|v| v == "1").unwrap_or(false),
    }
}

fn parse_query(q: &str) -> HashMap<String, String> {
    q.split('&')
        .filter(|s| !s.is_empty())
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            Some((url_decode(k), url_decode(v)))
        })
        .collect()
}

fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                    out.push(h * 16 + l);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
