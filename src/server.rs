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
    /// Red cargada desde `data/gtfs`. `RwLock` porque `/api/gtfs/update` la recarga en
    /// caliente (descarga + filtra el feed nuevo, sin reiniciar el servidor).
    pub net: std::sync::RwLock<Arc<Network>>,
    pub load_ms: f64,
    pub lines: std::sync::RwLock<Vec<String>>,
    pub opt: Arc<Mutex<OptJob>>,
    /// Estado del trabajo de actualización del GTFS (descarga/filtrado en segundo plano).
    pub gtfs_job: Arc<Mutex<GtfsJob>>,
    /// Caché de análisis de línea (clave = query string) para no recalcular.
    pub line_cache: Mutex<HashMap<String, String>>,
    /// Infraestructura ADIF (CVM + geometría) si está el fichero procesado.
    pub adif: Option<crate::calculator::infrastructure::AdifNet>,
    /// LTV (temporales) — recargable en caliente vía /api/ltv/reload.
    pub ltv: std::sync::RwLock<Option<Arc<crate::calculator::ltv::LtvSet>>>,
}

/// Estado del trabajo de actualización del GTFS (compartido con la UI, mismo patrón que
/// `OptJob`: se arranca en un hilo aparte y la web hace polling del progreso).
#[derive(Debug, Clone, Serialize, Default)]
pub struct GtfsJob {
    pub running: bool,
    pub done: bool,
    pub step: String,
    pub stats: Option<crate::gtfs_update::UpdateStats>,
    pub error: Option<String>,
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

/// Índice justo después de la primera línea en blanco `\r\n\r\n` (fin de cabeceras), si está
/// completa en `buf`.
fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

async fn handle(mut stream: TcpStream, state: Arc<ServerState>) -> std::io::Result<()> {
    let mut buf = vec![0u8; 8192];
    let n = stream.read(&mut buf).await?;
    if n == 0 {
        return Ok(());
    }
    let header_end = find_header_end(&buf[..n]).unwrap_or(n);
    let header_str = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let mut header_lines = header_str.lines();
    let request_line = header_lines.next().unwrap_or("");
    let mut rl_parts = request_line.split_whitespace();
    let method = rl_parts.next().unwrap_or("GET");
    let target = rl_parts.next().unwrap_or("/");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let is_post = method == "POST";

    // Cuerpo (solo relevante para /api/gtfs/upload): lo que ya llegó en el primer read tras
    // la cabecera, más lecturas adicionales hasta completar Content-Length (fichero GTFS
    // subido, puede pesar varios MB — no cabe en un único read de 8 KiB).
    const MAX_BODY: usize = 64 * 1024 * 1024; // 64 MiB, sobrado para un ZIP GTFS
    let content_length: usize = header_lines
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            if k.trim().eq_ignore_ascii_case("content-length") {
                v.trim().parse().ok()
            } else {
                None
            }
        })
        .unwrap_or(0)
        .min(MAX_BODY);
    let mut body: Vec<u8> = if header_end < n { buf[header_end..n].to_vec() } else { Vec::new() };
    while body.len() < content_length {
        let mut chunk = vec![0u8; 65536];
        let r = stream.read(&mut chunk).await?;
        if r == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..r]);
    }

    let (status, ctype, body): (&str, &str, Vec<u8>) = match path {
        "/" => ("200 OK", "text/html", render_page(&state).into_bytes()),
        "/api/render" => ("200 OK", "text/html", render_fragment(&state, query).into_bytes()),
        "/api/mintime" => ("200 OK", "text/html", render_mintime(&state, query).into_bytes()),
        "/api/lines" => ("200 OK", "application/json", lines_json(&state).into_bytes()),
        "/api/stations" => ("200 OK", "application/json", stations_json(&state).into_bytes()),
        "/api/game/network" => {
            let net = state.net.read().unwrap();
            let lines = state.lines.read().unwrap();
            (
                "200 OK",
                "application/json",
                crate::game::network_json(&net, &lines, &now_utc_string()).into_bytes(),
            )
        }
        "/api/game/schedule" => {
            let q = parse_query(query);
            let optimized = q.get("source").map(|s| s == "optimized").unwrap_or(false);
            let line = q.get("line").map(|s| s.as_str()).filter(|s| !s.is_empty());
            let net = state.net.read().unwrap();
            (
                "200 OK",
                "application/json",
                crate::game::schedule_json(&net, optimized, line).into_bytes(),
            )
        }
        "/api/gtfs/update" => ("200 OK", "application/json", start_gtfs_update(&state).into_bytes()),
        "/api/gtfs/status" => {
            let j = state.gtfs_job.lock().unwrap().clone();
            ("200 OK", "application/json", serde_json::to_vec(&j).unwrap_or_default())
        }
        "/api/gtfs/upload" if is_post => {
            ("200 OK", "application/json", start_gtfs_upload(&state, &body).into_bytes())
        }
        "/game" | "/game/" => match serve_game_file("index.html") {
            Some((ct, bytes)) => ("200 OK", ct, bytes),
            None => ("404 Not Found", "text/plain", b"404".to_vec()),
        },
        p if p.starts_with("/game/") => match serve_game_file(p.trim_start_matches("/game/")) {
            Some((ct, bytes)) => ("200 OK", ct, bytes),
            None => ("404 Not Found", "text/plain", b"404".to_vec()),
        },
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
        p if p.starts_with("/reference/trenscat/") => match serve_trenscat_file(p) {
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

/// Sirve un diagrama de vías descargado en `reference/trenscat/<stop_id>/<fichero>` (nombre
/// saneado; solo imágenes .gif/.jpg/.png, sin subcarpetas más allá del stop_id).
fn serve_trenscat_file(path: &str) -> Option<(&'static str, Vec<u8>)> {
    let rel = path.strip_prefix("/reference/trenscat/")?;
    if rel.is_empty()
        || rel.contains("..")
        || !rel.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '/'))
    {
        return None;
    }
    let ct = if rel.ends_with(".gif") {
        "image/gif"
    } else if rel.ends_with(".jpg") || rel.ends_with(".jpeg") {
        "image/jpeg"
    } else if rel.ends_with(".png") {
        "image/png"
    } else {
        return None;
    };
    let bytes = std::fs::read(std::path::Path::new("reference/trenscat").join(rel)).ok()?;
    Some((ct, bytes))
}

/// Sirve los archivos estáticos del juego web desde `game/web/` (nombre saneado).
fn serve_game_file(name: &str) -> Option<(&'static str, Vec<u8>)> {
    if name.is_empty()
        || name.contains("..")
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '/'))
    {
        return None;
    }
    let ct = if name.ends_with(".html") {
        "text/html"
    } else if name.ends_with(".js") {
        "text/javascript"
    } else if name.ends_with(".css") {
        "text/css"
    } else if name.ends_with(".json") {
        "application/json"
    } else if name.ends_with(".svg") {
        "image/svg+xml"
    } else {
        "application/octet-stream"
    };
    let bytes = std::fs::read(std::path::Path::new("game/web").join(name)).ok()?;
    Some((ct, bytes))
}

fn render_page(state: &ServerState) -> String {
    let net_guard = state.net.read().unwrap();
    let net: &Network = &net_guard;
    let lines = state.lines.read().unwrap();
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
        &summary, &example, &sim, &res, &lines, &controls, &now_utc_string(), &calc_panel,
    )
}

/// Fragmento de resultados del calculador de tiempo mínimo.
fn render_mintime(state: &ServerState, query: &str) -> String {
    let q = parse_query(query);
    let net_guard = state.net.read().unwrap();
    let net: &Network = &net_guard;
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
    let net_guard = state.net.read().unwrap();
    let net: &Network = &net_guard;
    let lines = state.lines.read().unwrap();
    let mut out: Vec<LineOut> = Vec::new();
    for line in lines.iter() {
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
    let net_guard = state.net.read().unwrap();
    let net: &Network = &net_guard;
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
    let net_guard = state.net.read().unwrap();
    let net: &Network = &net_guard;
    let q = parse_query(query);
    let line = q.get("line").cloned().unwrap_or_default();

    // Sentido: (d0,d1) o el primero disponible.
    let dirs = crate::calculator::schedules::line_directions(net, &line);
    let key = match (q.get("d0"), q.get("d1")) {
        (Some(a), Some(b)) if !a.is_empty() && !b.is_empty() => (a.clone(), b.clone()),
        _ => match dirs.first() {
            Some(d) => d.key.clone(),
            None => {
                return serde_json::json!({"error":"Línea sin sentidos en el GTFS."}).to_string()
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
    let net_guard = state.net.read().unwrap();
    let net: &Network = &net_guard;
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
    let net = state.net.read().unwrap().clone();
    let opt = state.opt.clone();
    std::thread::spawn(move || run_optimization(net, opt));
    "{\"started\":true}".into()
}

/// Lanza la actualización del GTFS (descarga oficial de Renfe) en un hilo aparte. Mismo
/// patrón de polling que el optimizador (`/api/gtfs/status`).
fn start_gtfs_update(state: &Arc<ServerState>) -> String {
    {
        let mut job = state.gtfs_job.lock().unwrap();
        if job.running {
            return "{\"started\":false,\"reason\":\"ya hay una actualización en curso\"}".into();
        }
        *job = GtfsJob { running: true, step: "Iniciando…".into(), ..Default::default() };
    }
    let st = state.clone();
    std::thread::spawn(move || {
        let job = st.gtfs_job.clone();
        let progress = {
            let job = job.clone();
            move |msg: &str| {
                if let Ok(mut j) = job.lock() {
                    j.step = msg.to_string();
                }
            }
        };
        let result = crate::gtfs_update::update_from_renfe(progress);
        finish_gtfs_job(&st, result);
    });
    "{\"started\":true}".into()
}

/// Igual que `start_gtfs_update` pero partiendo de un ZIP subido por el usuario (fallback si
/// la descarga automática no es viable, p. ej. si Renfe cambia la URL o exige sesión).
fn start_gtfs_upload(state: &Arc<ServerState>, body: &[u8]) -> String {
    if body.is_empty() {
        return "{\"started\":false,\"reason\":\"cuerpo vacío\"}".into();
    }
    {
        let mut job = state.gtfs_job.lock().unwrap();
        if job.running {
            return "{\"started\":false,\"reason\":\"ya hay una actualización en curso\"}".into();
        }
        *job = GtfsJob { running: true, step: "Procesando el fichero subido…".into(), ..Default::default() };
    }
    let st = state.clone();
    let bytes = body.to_vec();
    std::thread::spawn(move || {
        let job = st.gtfs_job.clone();
        let progress = {
            let job = job.clone();
            move |msg: &str| {
                if let Ok(mut j) = job.lock() {
                    j.step = msg.to_string();
                }
            }
        };
        let result = crate::gtfs_update::update_from_bytes(&bytes, progress);
        finish_gtfs_job(&st, result);
    });
    "{\"started\":true}".into()
}

/// Tras filtrar el GTFS con éxito: recarga la red, refresca la lista de líneas y limpia la
/// caché de análisis (dependía del GTFS anterior). Deja el resultado en `gtfs_job`.
fn finish_gtfs_job(state: &Arc<ServerState>, result: Result<crate::gtfs_update::UpdateStats, String>) {
    match result {
        Ok(stats) => match crate::gtfs_loader::load(std::path::Path::new("data/gtfs")) {
            Ok(net) => {
                let new_lines = scenario::distinct_lines(&net);
                if let Ok(mut g) = state.net.write() {
                    *g = Arc::new(net);
                }
                if let Ok(mut g) = state.lines.write() {
                    *g = new_lines;
                }
                if let Ok(mut c) = state.line_cache.lock() {
                    c.clear();
                }
                let mut j = state.gtfs_job.lock().unwrap();
                j.stats = Some(stats);
                j.step = "Actualización completa.".into();
            }
            Err(e) => {
                let mut j = state.gtfs_job.lock().unwrap();
                j.error = Some(format!("El GTFS se filtró pero no se pudo recargar: {e}"));
            }
        },
        Err(e) => {
            let mut j = state.gtfs_job.lock().unwrap();
            j.error = Some(e);
        }
    }
    let mut j = state.gtfs_job.lock().unwrap();
    j.running = false;
    j.done = true;
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
