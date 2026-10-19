//! Servidor web local (tokio) para la UI interactiva. Sirve:
//!   * `GET /`                    → menú principal (el juego como pieza central + lanzadores).
//!   * `GET /panel`               → página con controles + dashboard inicial + JS.
//!   * `GET /api/render`          → fragmento HTML del dashboard para unos parámetros.
//!   * `GET /api/optimize/start`  → lanza la optimización de una línea en segundo plano.
//!   * `GET /api/optimize/status` → progreso en vivo (JSON) de la optimización.
//!   * `GET /report/optimized/…`  → descarga los CSV/PDF generados.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
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
    /// Catálogo del creador de horarios cadenciados, cacheado por red (puntero del `Arc<Network>`:
    /// una recarga del GTFS lo invalida sola).
    pub cadence_cat: Mutex<Option<(usize, Arc<crate::cadence::CadenceCatalog>)>>,
    /// Mallas cadenciadas activas (id = hash de la especificación → trenes). El navegador las vuelve
    /// a enviar antes de cada run (`/api/cadence/load`), así que se pueden perder sin problema.
    pub cadences: Mutex<HashMap<String, Arc<Vec<crate::game::GameTrain>>>>,
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
    start_cloud_autosync();
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
        "/" => ("200 OK", "text/html", report::render_menu_page().into_bytes()),
        "/info" | "/informacion" => ("200 OK", "text/html", report::render_info_page().into_bytes()),
        "/numeros" | "/numbers" => ("200 OK", "text/html", report::render_numbers_page().into_bytes()),
        "/panel" | "/panel/" => ("200 OK", "text/html", render_page(&state).into_bytes()),
        "/api/render" => ("200 OK", "text/html", render_fragment(&state, query).into_bytes()),
        "/api/mintime" => ("200 OK", "text/html", render_mintime(&state, query).into_bytes()),
        "/api/lines" => ("200 OK", "application/json", lines_json(&state).into_bytes()),
        "/api/stations" => ("200 OK", "application/json", stations_json(&state).into_bytes()),
        "/api/game/network" => {
            let net = state.net.read().unwrap();
            let lines = state.lines.read().unwrap();
            let ltv_guard = state.ltv.read().unwrap();
            (
                "200 OK",
                "application/json",
                crate::game::network_json(&net, &lines, &now_utc_string(), state.adif.as_ref(), ltv_guard.as_deref())
                    .into_bytes(),
            )
        }
        "/api/game/schedule" => {
            let q = parse_query(query);
            let optimized = q.get("source").map(|s| s == "optimized").unwrap_or(false);
            let line = q.get("line").map(|s| s.as_str()).filter(|s| !s.is_empty());
            let net = state.net.read().unwrap();
            let body = match cadence_param(&state, &q) {
                Some(Ok(trains)) => serde_json::json!({
                    "source": "cadence",
                    "service_id": "cadenciado",
                    "n_trains": trains.len(),
                    "trains": &*trains,
                })
                .to_string(),
                Some(Err(e)) => serde_json::json!({ "error": e }).to_string(),
                None => crate::game::schedule_json(&net, optimized, line),
            };
            ("200 OK", "application/json", body.into_bytes())
        }
        // Creador de horarios cadenciados: página, catálogo de recorridos y carga de una malla.
        "/horarios" | "/cadencia" => ("200 OK", "text/html", report::render_cadence_page().into_bytes()),
        "/api/cadence/catalog" => {
            let cat = cadence_catalog(&state);
            ("200 OK", "application/json", serde_json::to_vec(&cat.out).unwrap_or_default())
        }
        "/api/cadence/load" if is_post => ("200 OK", "application/json", cadence_load(&state, &body).into_bytes()),
        // Incidencias (creador compartido por el modo visual y el headless).
        "/api/incidents/templates" if is_post => {
            ("200 OK", "application/json", save_incident_templates(&body).into_bytes())
        }
        "/api/incidents/templates" => {
            ("200 OK", "application/json", incident_templates_json().into_bytes())
        }
        // Simulación HEADLESS (solo números) del motor unificado con las plantillas activas.
        "/api/simulate" => ("200 OK", "application/json", simulate_json(&state, query).into_bytes()),
        // Trace de la jornada (posición/velocidad por tren) para el REPRODUCTOR visual.
        "/api/game/trace" => ("200 OK", "application/json", trace_json(&state, query).into_bytes()),
        "/api/gtfs/update" => ("200 OK", "application/json", start_gtfs_update(&state).into_bytes()),
        "/api/gtfs/status" => {
            let j = state.gtfs_job.lock().unwrap().clone();
            ("200 OK", "application/json", serde_json::to_vec(&j).unwrap_or_default())
        }
        // Exportación de los resultados de /numeros (xlsx|pdf, con o sin horarios usados).
        "/api/runs/export" if is_post => export_runs(&state, query, &body),
        "/api/recovery/fit" if is_post => ("200 OK", "application/json", recovery_fit_json(&body).into_bytes()),
        "/incidencias" | "/incidents" => ("200 OK", "text/html", report::render_incidents_page().into_bytes()),
        "/api/gtfs/info" => ("200 OK", "application/json", gtfs_info_json(&state).into_bytes()),
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
        // Descarga las LTV vigentes de ADIF (scripts/fetch_adif_ltv.py) y las recarga.
        "/api/ltv/fetch" => ("200 OK", "application/json", ltv_fetch(&state).into_bytes()),
        // Parámetros de realismo (página /configuracion) y supuestos del modelo (/supuestos).
        "/configuracion" | "/config" => ("200 OK", "text/html", report::render_config_page().into_bytes()),
        "/supuestos" | "/assumptions" => ("200 OK", "text/html", report::render_assumptions_page().into_bytes()),
        "/mercancias" | "/freight" => ("200 OK", "text/html", report::render_freight_page().into_bytes()),
        "/api/realism" if is_post => ("200 OK", "application/json", realism_save(&body).into_bytes()),
        "/api/realism" => ("200 OK", "application/json", realism_json().into_bytes()),
        "/api/realism/reset" => ("200 OK", "application/json", realism_reset().into_bytes()),
        "/api/assumptions" => (
            "200 OK",
            "application/json",
            serde_json::to_vec(&crate::realism::assumptions()).unwrap_or_default(),
        ),
        // Escenarios de mercancías (lista completa, como las plantillas de incidencia).
        "/api/freight/scenarios" if is_post => ("200 OK", "application/json", freight_save(&body).into_bytes()),
        "/api/freight/scenarios" => (
            "200 OK",
            "application/json",
            serde_json::to_vec(&crate::freight::load_scenarios()).unwrap_or_default(),
        ),
        // Datos reales de Renfe (GTFS-RT): página, estado, recogida automática, análisis y comparación.
        "/datos" | "/data" => ("200 OK", "text/html", report::render_data_page().into_bytes()),
        "/api/gtfsrt/status" => ("200 OK", "application/json", gtfsrt_status().into_bytes()),
        "/api/gtfsrt/autostart" => ("200 OK", "application/json", gtfsrt_autostart("status").into_bytes()),
        "/api/gtfsrt/install" if is_post => ("200 OK", "application/json", gtfsrt_autostart("install").into_bytes()),
        "/api/gtfsrt/uninstall" if is_post => ("200 OK", "application/json", gtfsrt_autostart("uninstall").into_bytes()),
        "/api/gtfsrt/start" if is_post => ("200 OK", "application/json", gtfsrt_start().into_bytes()),
        "/api/gtfsrt/stop" if is_post => ("200 OK", "application/json", gtfsrt_stop().into_bytes()),
        "/api/gtfsrt/days" => ("200 OK", "application/json", gtfsrt_days().into_bytes()),
        "/api/gtfsrt/analyze" if is_post => ("200 OK", "application/json", gtfsrt_analyze(day_param(query)).into_bytes()),
        // Incidencias publicadas (RSS de la Generalitat + avisos de Renfe) asociadas a cada día.
        "/api/incidents" => ("200 OK", "application/json", incidents_list(day_param(query)).into_bytes()),
        "/api/incidents/capture" if is_post => ("200 OK", "application/json", incidents_capture().into_bytes()),
        // Recolector en la NUBE (Cloudflare Workers, plan gratuito; ver cloud.rs).
        "/api/cloud/info" => ("200 OK", "application/json", crate::cloud::info_json().into_bytes()),
        "/api/cloud/login" if is_post => ("200 OK", "application/json", crate::cloud::login().into_bytes()),
        "/api/cloud/whoami" => ("200 OK", "application/json", crate::cloud::whoami().into_bytes()),
        "/api/cloud/deploy" if is_post => ("200 OK", "application/json", cloud_then_recheck(crate::cloud::deploy()).into_bytes()),
        "/api/cloud/status" => ("200 OK", "application/json", crate::cloud::status().into_bytes()),
        "/api/cloud/start" if is_post => ("200 OK", "application/json", cloud_then_recheck(crate::cloud::service("start")).into_bytes()),
        "/api/cloud/stop" if is_post => ("200 OK", "application/json", cloud_then_recheck(crate::cloud::service("stop")).into_bytes()),
        "/api/cloud/sync" if is_post => ("200 OK", "application/json", crate::cloud::sync().into_bytes()),
        "/api/cloud/forget" if is_post => ("200 OK", "application/json", crate::cloud::forget().into_bytes()),
        "/api/compare" => ("200 OK", "application/json", compare_json(&state, query).into_bytes()),
        "/api/freight/preview" => ("200 OK", "application/json", freight_preview(&state, query).into_bytes()),
        "/api/freight/materialize" if is_post => {
            ("200 OK", "application/json", freight_materialize(&state, query).into_bytes())
        }
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
    } else if name.ends_with(".gif") {
        "image/gif"
    } else {
        "application/octet-stream"
    };
    let bytes = std::fs::read(std::path::Path::new("game/web").join(name)).ok()?;
    Some((ct, bytes))
}

// --------------------------------------------------------------------------
// Incidencias: plantillas persistentes (creador compartido visual + headless)
// --------------------------------------------------------------------------

const INCIDENTS_PATH: &str = "report/incidents.json";

/// Plantilla tal y como la ve/edita el frontend. `effect` = "red_one" | "red_both" | "single".
/// Tasa: probabilidad `prob` de que ocurra cada `every_h` horas. `id` = identificador ESTABLE (la
/// selección por modo y la edición se refieren a él, no a la posición en la lista). Ficheros viejos
/// con `daily_prob` se leen como `prob` cada 24 h (misma semántica que antes).
#[derive(Serialize, Deserialize, Clone)]
struct TemplateDto {
    #[serde(default)]
    id: String,
    motivo: String,
    effect: String,
    dur_min: f64,
    #[serde(alias = "daily_prob")]
    prob: f64,
    #[serde(default = "every_h_default")]
    every_h: f64,
    enabled: bool,
}

fn every_h_default() -> f64 {
    24.0
}

fn effect_str(e: crate::engine::IncidentEffect) -> &'static str {
    match e {
        crate::engine::IncidentEffect::RedSignal { both: true } => "red_both",
        crate::engine::IncidentEffect::RedSignal { both: false } => "red_one",
        crate::engine::IncidentEffect::SingleTrack => "single",
    }
}

fn to_template(d: &TemplateDto) -> crate::engine::IncidentTemplate {
    let effect = match d.effect.as_str() {
        "red_both" => crate::engine::IncidentEffect::RedSignal { both: true },
        "single" => crate::engine::IncidentEffect::SingleTrack,
        _ => crate::engine::IncidentEffect::RedSignal { both: false },
    };
    crate::engine::IncidentTemplate {
        motivo: d.motivo.clone(),
        effect,
        dur_min: d.dur_min,
        prob: d.prob.clamp(0.0, 1.0),
        every_h: d.every_h.max(crate::engine::MIN_EVERY_H),
        enabled: d.enabled,
    }
}

/// Id nuevo, único en la lista (`tpl` + base36 del instante + índice).
fn new_template_id(i: usize) -> String {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let mut x = n + i as u128;
    let mut s = String::new();
    while x > 0 {
        let d = (x % 36) as u32;
        s.insert(0, std::char::from_digit(d, 36).unwrap_or('0'));
        x /= 36;
    }
    format!("tpl{s}")
}

/// Garantiza ids no vacíos y únicos; devuelve `true` si tuvo que asignar alguno.
fn ensure_ids(v: &mut [TemplateDto]) -> bool {
    let mut seen = std::collections::HashSet::new();
    let mut changed = false;
    for (i, d) in v.iter_mut().enumerate() {
        let ok = !d.id.is_empty() && d.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if !ok || !seen.insert(d.id.clone()) {
            d.id = new_template_id(i);
            seen.insert(d.id.clone());
            changed = true;
        }
    }
    changed
}

fn write_incident_dtos(v: &[TemplateDto]) -> bool {
    std::fs::create_dir_all("report").ok();
    std::fs::write(INCIDENTS_PATH, serde_json::to_string_pretty(v).unwrap_or_default()).is_ok()
}

/// Carga las plantillas de disco; si no existen, siembra las de por defecto (eliminables). Si el
/// fichero es antiguo (sin ids / con `daily_prob`), se migra y se reescribe una vez.
fn load_incident_dtos() -> Vec<TemplateDto> {
    let mut v: Vec<TemplateDto> = std::fs::read_to_string(INCIDENTS_PATH)
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<TemplateDto>>(&s).ok())
        .unwrap_or_else(|| {
            crate::engine::default_templates()
                .iter()
                .map(|t| TemplateDto {
                    id: String::new(),
                    motivo: t.motivo.clone(),
                    effect: effect_str(t.effect).to_string(),
                    dur_min: t.dur_min,
                    prob: t.prob,
                    every_h: t.every_h,
                    enabled: t.enabled,
                })
                .collect()
        });
    if ensure_ids(&mut v) {
        write_incident_dtos(&v);
    }
    v
}

fn incident_templates_json() -> String {
    serde_json::to_string(&load_incident_dtos()).unwrap_or_else(|_| "[]".into())
}

/// Plantillas ACTIVAS para una ejecución. Si la query trae `sel=id1,id2` (ids ESTABLES), se usa esa
/// selección (independiente por modo: visual vs números); si no, se respeta el `enabled` guardado.
/// Así las plantillas son compartidas pero la SELECCIÓN de cada modo no contamina a la otra.
fn active_templates(q: &HashMap<String, String>) -> Vec<crate::engine::IncidentTemplate> {
    let dtos = load_incident_dtos();
    let sel: Option<std::collections::HashSet<&str>> =
        q.get("sel").map(|s| s.split(',').map(str::trim).filter(|x| !x.is_empty()).collect());
    dtos.iter()
        .map(|d| {
            let mut t = to_template(d);
            if let Some(ref set) = sel {
                t.enabled = set.contains(d.id.as_str());
            }
            t
        })
        .collect()
}

/// Variabilidad de velocidad de la run: `vvar=<p en %>` (p. ej. `vvar=3`). Ausente o ≤ 0 = apagada
/// (todos a Vmax). Se acota a `SPEED_VAR_MAX` en el motor. Lo comparten el modo visual y /numeros.
fn speed_var_param(q: &HashMap<String, String>) -> Option<f64> {
    q.get("vvar").and_then(|s| s.parse::<f64>().ok()).filter(|p| p.is_finite() && *p > 0.0).map(|p| p / 100.0)
}

/// Modo SIN HORARIOS (`free=1`, solo el visual): los trenes conservan solo la salida de cabecera.
fn free_run_param(q: &HashMap<String, String>) -> bool {
    q.get("free").map(|v| v == "1").unwrap_or(false)
}

/// Catálogo de recorridos del creador de horarios (cacheado mientras no cambie la red).
fn cadence_catalog(state: &ServerState) -> Arc<crate::cadence::CadenceCatalog> {
    let net = state.net.read().unwrap().clone();
    let key = Arc::as_ptr(&net) as usize;
    let mut guard = state.cadence_cat.lock().unwrap();
    if let Some((k, c)) = guard.as_ref() {
        if *k == key {
            return c.clone();
        }
    }
    let c = Arc::new(crate::cadence::build_catalog(&net));
    *guard = Some((key, c.clone()));
    c
}

/// Carga (genera) una malla cadenciada a partir de la especificación del creador y la deja activa
/// con un id estable (hash del cuerpo). Devuelve el id, el nº de trenes, el recuento por línea y
/// los avisos (recorridos que ya no existen en el GTFS cargado).
fn cadence_load(state: &ServerState, body: &[u8]) -> String {
    use std::hash::{Hash, Hasher};
    let spec: crate::cadence::Spec = match serde_json::from_slice(body) {
        Ok(s) => s,
        Err(e) => return serde_json::json!({ "ok": false, "error": format!("Horario inválido: {e}") }).to_string(),
    };
    let cat = cadence_catalog(state);
    let (trains, warnings) = crate::cadence::generate(&cat, &spec);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    body.hash(&mut h);
    let id = format!("{:016x}", h.finish());
    let mut per_line: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for t in &trains {
        *per_line.entry(t.line.as_str()).or_default() += 1;
    }
    let empty = trains.is_empty();
    let out = serde_json::json!({
        "ok": !empty,
        "id": id,
        "name": spec.name,
        "n_trains": trains.len(),
        "per_line": per_line,
        "first_dep": trains.iter().map(|t| t.stops[0].d).min(),
        "warnings": warnings,
        "error": if empty { Some("El horario no genera ningún tren: coloca al menos una línea en un reloj.") } else { None },
    })
    .to_string();
    let mut map = state.cadences.lock().unwrap();
    if map.len() > 32 {
        map.clear(); // uso local: basta con no crecer sin límite
    }
    map.insert(id, Arc::new(trains));
    out
}

/// Malla cadenciada pedida por `cad=<id>` (None = horario GTFS). Error si el id ya no está cargado.
fn cadence_param(
    state: &ServerState,
    q: &HashMap<String, String>,
) -> Option<Result<Arc<Vec<crate::game::GameTrain>>, String>> {
    let id = q.get("cad").filter(|s| !s.is_empty())?;
    Some(state.cadences.lock().unwrap().get(id).cloned().ok_or_else(|| {
        "El horario cadenciado no está cargado en el servidor: vuelve a elegirlo y confirma.".to_string()
    }))
}

/// Construye el motor de una run: horario GTFS o malla cadenciada (`cad=`), según la query, con los
/// parámetros de realismo guardados y el escenario de mercancías elegido (`frt=<id>`, `none` = sin).
fn engine_for(
    state: &ServerState,
    net: &Network,
    q: &HashMap<String, String>,
    cfg: crate::engine::EngineCfg,
    ltv: Option<&crate::calculator::ltv::LtvSet>,
    templates: &[crate::engine::IncidentTemplate],
) -> Result<(crate::engine::Engine, FreightInfo), String> {
    let rc = crate::realism::RealismCfg::load();
    let gtfs;
    let trains: &[crate::game::GameTrain] = match cadence_param(state, q) {
        Some(Ok(t)) => {
            gtfs = t;
            &gtfs
        }
        Some(Err(e)) => return Err(e),
        None => {
            gtfs = Arc::new(crate::game::build_schedule(net, false, None));
            &gtfs
        }
    };
    let (freight, errors) = freight_for(net, q, cfg.seed, trains);
    let eng = crate::engine::Engine::with_schedule_ext(
        net,
        cfg,
        state.adif.as_ref(),
        ltv,
        templates,
        None,
        trains,
        &rc,
        &freight,
    );
    let list = freight
        .iter()
        .map(|f| {
            serde_json::json!({
                "id": f.gt.train,
                "corridor": f.corridor,
                "from": f.gt.from,
                "to": f.gt.to,
                "dep": f.gt.stops.first().map(|s| s.d).unwrap_or(0),
                "stops": f.gt.stops.iter().filter(|s| net.node(&s.s).is_some()).map(|s| s.s.clone()).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok((eng, FreightInfo { list, errors }))
}

/// Mercancías de una run, para devolverlos al frontend (visor y /numeros).
struct FreightInfo {
    list: Vec<serde_json::Value>,
    errors: Vec<String>,
}

/// Mercancías de la run según `frt=<id>` (escenario guardado). La ventana de generación empieza en
/// la primera salida de viajeros (antes el motor aún no corre) y acaba a medianoche.
fn freight_for(
    net: &Network,
    q: &HashMap<String, String>,
    seed: u64,
    trains: &[crate::game::GameTrain],
) -> (Vec<crate::freight::FreightTrain>, Vec<String>) {
    let id = q.get("frt").map(String::as_str).unwrap_or("none");
    let Some(sc) = crate::freight::scenario_by_id(id) else { return (Vec::new(), Vec::new()) };
    let start = trains.iter().filter_map(|t| t.stops.first().map(|s| s.d)).min().unwrap_or(5 * 3600) as f64;
    let plan = crate::freight::plan(&sc, seed, start + 300.0, 24.0 * 3600.0);
    crate::freight::build_trains(net, &plan)
}

/// Guarda la lista COMPLETA de plantillas (el frontend envía todo el conjunto al añadir/borrar/toggle).
fn save_incident_templates(body: &[u8]) -> String {
    match serde_json::from_slice::<Vec<TemplateDto>>(body) {
        Ok(mut v) => {
            ensure_ids(&mut v);
            let ok = write_incident_dtos(&v);
            // Devuelve la lista con ids definitivos (el cliente los adopta).
            serde_json::json!({ "ok": ok, "n": v.len(), "templates": v }).to_string()
        }
        Err(e) => serde_json::json!({ "ok": false, "error": e.to_string() }).to_string(),
    }
}

/// Corre el motor unificado HEADLESS con las plantillas activas y una semilla → métricas + la
/// tirada de incidencias del día. Es el "modo sin nada visual" (solo números por run). Bloquea
/// la petición mientras corre (~s); uso local de un solo usuario.
fn simulate_json(state: &ServerState, query: &str) -> String {
    let q = parse_query(query);
    let seed: u64 = q.get("seed").and_then(|s| s.parse().ok()).unwrap_or(1);
    let templates: Vec<crate::engine::IncidentTemplate> = active_templates(&q);

    let net_guard = state.net.read().unwrap();
    let ltv_guard = state.ltv.read().unwrap();
    let mut cfg = crate::engine::EngineCfg::day();
    cfg.seed = seed;
    cfg.speed_var = speed_var_param(&q);
    let (mut eng, frt) = match engine_for(state, &net_guard, &q, cfg, ltv_guard.as_deref(), &templates) {
        Ok(e) => e,
        Err(e) => return serde_json::json!({ "error": e }).to_string(),
    };
    let m = eng.run();
    let hm = |s: f64| {
        let t = s.max(0.0) as u32;
        format!("{:02}:{:02}", t / 3600, (t % 3600) / 60)
    };
    let incidents: Vec<_> = eng
        .incidents()
        .iter()
        .map(|i| {
            serde_json::json!({
                "motivo": i.motivo,
                "effect": effect_str(i.effect),
                "from": i.from,
                "to": i.to,
                // Nombres de estación legibles + horas HH:MM (para el log de /numeros: dónde y cuándo).
                "from_name": net_guard.stop_name(&i.from),
                "to_name": net_guard.stop_name(&i.to),
                "t0": i.t0,
                "t1": i.t1,
                "t0_hm": hm(i.t0),
                "t1_hm": hm(i.t1),
                "impact_min": crate::recovery::incident_impact_min(eng.timeline(), i.t0, i.t1),
            })
        })
        .collect();
    let (track, analysis) = delay_track_json(&eng);
    serde_json::json!({
        "seed": seed,
        "speed_var_pct": cfg.speed_var.map(|p| p * 100.0),
        "n_trains": m.n_trains,
        "sim_hours": m.sim_seconds as f64 / 3600.0,
        "train_min_delay": m.train_min_delay,
        "max_delay_min": m.max_delay_s / 60.0,
        "peak_waiting": m.peak_waiting,
        "final_waiting": m.final_waiting,
        "pax_min": m.pax_min,
        "incidents": incidents,
        "track": track,
        "analysis": analysis,
        "realism": eng.realism_stats(),
        "freight": frt.list.len(),
        "freight_errors": frt.errors,
    })
    .to_string()
}

/// Análisis de un tramo MANUAL (herramienta de selección del seguimiento de retrasos). Cuerpo
/// JSON `{total:[min…], active:[trenes…], i0, i1}` = las series de la run que ya tiene el navegador
/// + el tramo marcado. Misma matemática que la detección automática (`recovery::analyze_manual`).
fn recovery_fit_json(body: &[u8]) -> String {
    #[derive(serde::Deserialize)]
    struct Req {
        total: Vec<f64>,
        active: Vec<u32>,
        i0: usize,
        i1: usize,
    }
    match serde_json::from_slice::<Req>(body) {
        Ok(r) => match crate::recovery::analyze_manual(&r.total, &r.active, r.i0, r.i1) {
            Ok(p) => serde_json::json!({ "ok": true, "peak": p }).to_string(),
            Err(e) => serde_json::json!({ "ok": false, "error": e }).to_string(),
        },
        Err(e) => serde_json::json!({ "ok": false, "error": format!("Petición inválida: {e}") }).to_string(),
    }
}

/// Seguimiento de retrasos de una jornada ya corrida: series por minuto (retraso medio por tren y
/// acumulado de la red) + análisis de recuperación de los picos de cada serie (recovery.rs).
fn delay_track_json(eng: &crate::engine::Engine) -> (serde_json::Value, serde_json::Value) {
    let tr = crate::recovery::delay_track(eng.timeline(), eng.timeline_active());
    // Se ajusta el retraso MEDIO; el acumulado (= N·medio) se contrasta por integrales en cada pico.
    let analysis = serde_json::json!({
        "mean": crate::recovery::analyze_track(eng.timeline(), eng.timeline_active()),
    });
    (serde_json::to_value(&tr).unwrap_or_default(), analysis)
}

/// Cuántos tramos guarda el ranking de los más problemáticos de una run visual.
const HOTSPOTS_TOP: usize = 15;

/// Trace de la jornada (posición/velocidad por tren, muestreado) para el reproductor visual.
/// Es el MISMO motor que el modo headless: lo que se ve = lo que se simula en masa.
fn trace_json(state: &ServerState, query: &str) -> String {
    let q = parse_query(query);
    let seed: u64 = q.get("seed").and_then(|s| s.parse().ok()).unwrap_or(1);
    let templates: Vec<crate::engine::IncidentTemplate> = active_templates(&q);
    let net_guard = state.net.read().unwrap();
    let ltv_guard = state.ltv.read().unwrap();
    let mut cfg = crate::engine::EngineCfg::day();
    cfg.seed = seed;
    cfg.speed_var = speed_var_param(&q);
    cfg.free_run = free_run_param(&q);
    let (mut eng, frt) = match engine_for(state, &net_guard, &q, cfg, ltv_guard.as_deref(), &templates) {
        Ok(e) => e,
        Err(e) => return serde_json::json!({ "error": e }).to_string(),
    };
    // Incidencia MANUAL "lanzar ahora": params opcionales from/to (+ eff/t0/dur/motivo).
    if let (Some(from), Some(to)) = (q.get("from"), q.get("to")) {
        let effect = match q.get("eff").map(|s| s.as_str()) {
            Some("red_both") => crate::engine::IncidentEffect::RedSignal { both: true },
            Some("single") => crate::engine::IncidentEffect::SingleTrack,
            _ => crate::engine::IncidentEffect::RedSignal { both: false },
        };
        let t0 = q.get("t0").and_then(|s| s.parse().ok()).unwrap_or(0.0);
        let dur = q.get("dur").and_then(|s| s.parse().ok()).unwrap_or(15.0);
        let motivo = q.get("motivo").cloned().unwrap_or_else(|| "Incidencia".into());
        eng.inject_incident(from.clone(), to.clone(), effect, t0, dur, motivo);
    }
    let trace = eng.run_trace(5.0);
    // Incidencias del día (roll + manual) con nombres y horas, para mostrarlas en el juego.
    let incidents: Vec<_> = eng
        .incidents()
        .iter()
        .map(|i| {
            serde_json::json!({
                "motivo": i.motivo,
                "effect": effect_str(i.effect),
                "from": i.from,
                "to": i.to,
                "from_name": net_guard.stop_name(&i.from),
                "to_name": net_guard.stop_name(&i.to),
                "t0": i.t0,
                "t1": i.t1,
                "impact_min": crate::recovery::incident_impact_min(eng.timeline(), i.t0, i.t1),
            })
        })
        .collect();
    let (track, analysis) = delay_track_json(&eng);
    // Ranking de los tramos que más tiempo han retenido trenes (solo el visual lo mide).
    let hotspots: Vec<_> = eng
        .hold_ranking(HOTSPOTS_TOP)
        .into_iter()
        .map(|h| {
            serde_json::json!({
                "from": h.from,
                "to": h.to,
                "from_name": net_guard.stop_name(&h.from),
                "to_name": net_guard.stop_name(&h.to),
                "held_s": h.held_s.round(),
                "events": h.events,
                "max_s": h.max_s.round(),
                "lines": h.lines.iter().map(|(l, s)| serde_json::json!([l, s.round()])).collect::<Vec<_>>(),
            })
        })
        .collect();
    let mut v = serde_json::to_value(&trace).unwrap_or_else(|_| serde_json::json!({}));
    if let Some(obj) = v.as_object_mut() {
        obj.insert("hotspots".into(), serde_json::json!(hotspots));
        let (held_s, held_n) = eng.hold_total();
        obj.insert("held_total_s".into(), serde_json::json!(held_s.round()));
        obj.insert("held_total_events".into(), serde_json::json!(held_n));
        obj.insert("free_run".into(), serde_json::json!(cfg.free_run));
        obj.insert("incidents".into(), serde_json::json!(incidents));
        obj.insert("track".into(), track);
        obj.insert("analysis".into(), analysis);
        obj.insert("realism".into(), serde_json::json!(eng.realism_stats()));
        obj.insert("freight".into(), serde_json::json!(frt.list));
        obj.insert("freight_errors".into(), serde_json::json!(frt.errors));
    }
    v.to_string()
}

fn render_page(state: &ServerState) -> String {
    let net_guard = state.net.read().unwrap();
    let net: &Network = &net_guard;
    let lines = state.lines.read().unwrap();
    let p = SimParams::default();
    let summary = scenario::summary_view(net, state.load_ms);
    let example = scenario::example_view(net, "25412", p.line.as_deref());
    let ltv_guard = state.ltv.read().unwrap();
    let sim = scenario::sim_view(net, state.adif.as_ref(), ltv_guard.as_deref(), &p);
    let res = scenario::res_view(net, state.adif.as_ref(), ltv_guard.as_deref(), &p);
    let controls = Controls {
        start_h: p.start_sec / 3600,
        dur_h: (p.end_sec - p.start_sec) / 3600,
        line: p.line.clone(),
        block: p.block_min,
        delay: p.delay_min,
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
        // La R3a se pliega sobre el R3 (tramo aislado en obras): no se ofrece como línea propia.
        if crate::topology::display_line(line) != line {
            continue;
        }
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

/// Ejecuta `scripts/fetch_adif_ltv.py` (descarga oficial ADIF del día) y recarga las LTV. Bloquea
/// la petición mientras descarga (uso local de un solo usuario).
fn ltv_fetch(state: &ServerState) -> String {
    let script = "scripts/fetch_adif_ltv.py";
    let mut last_err = String::new();
    for py in ["python", "py", "python3"] {
        match std::process::Command::new(py).arg(script).output() {
            Ok(out) if out.status.success() => {
                let log = String::from_utf8_lossy(&out.stdout).lines().last().unwrap_or("").to_string();
                let reload: serde_json::Value = serde_json::from_str(&ltv_reload(state)).unwrap_or_default();
                return serde_json::json!({ "ok": true, "log": log, "reload": reload }).to_string();
            }
            Ok(out) => {
                last_err = String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("error").to_string();
            }
            Err(e) => last_err = e.to_string(),
        }
    }
    serde_json::json!({ "ok": false, "error": format!("No se pudieron descargar las LTV: {last_err}") }).to_string()
}

fn realism_json() -> String {
    serde_json::json!({
        "current": crate::realism::RealismCfg::load(),
        "defaults": crate::realism::RealismCfg::default(),
    })
    .to_string()
}

fn realism_save(body: &[u8]) -> String {
    match serde_json::from_slice::<crate::realism::RealismCfg>(body) {
        Ok(c) => {
            let c = c.sanitized();
            let ok = c.save();
            serde_json::json!({ "ok": ok, "current": c }).to_string()
        }
        Err(e) => serde_json::json!({ "ok": false, "error": e.to_string() }).to_string(),
    }
}

fn realism_reset() -> String {
    let c = crate::realism::RealismCfg::default();
    let ok = c.save();
    serde_json::json!({ "ok": ok, "current": c }).to_string()
}

/// Guarda la lista COMPLETA de escenarios de mercancías (ids estables; se generan si faltan).
fn freight_save(body: &[u8]) -> String {
    match serde_json::from_slice::<Vec<crate::freight::FreightScenario>>(body) {
        Ok(mut v) => {
            let stamp: String = now_utc_string().chars().filter(|c| c.is_ascii_digit()).collect();
            for (i, sc) in v.iter_mut().enumerate() {
                if sc.id.trim().is_empty() || sc.id == "none" {
                    sc.id = format!("esc{}_{}", i + 1, stamp);
                }
                for (k, c) in sc.corridors.iter_mut().enumerate() {
                    if c.id.trim().is_empty() {
                        c.id = format!("c{}", k + 1);
                    }
                }
            }
            let ok = crate::freight::save_scenarios(&v);
            serde_json::json!({ "ok": ok, "scenarios": v }).to_string()
        }
        Err(e) => serde_json::json!({ "ok": false, "error": e.to_string() }).to_string(),
    }
}

/// Trenes de mercancías que tendría una run (`frt=<id>&seed=N`), con nombre de estaciones y si se
/// han podido enrutar: lo que se ve antes de lanzar.
fn freight_preview(state: &ServerState, query: &str) -> String {
    let q = parse_query(query);
    let seed: u64 = q.get("seed").and_then(|s| s.parse().ok()).unwrap_or(1);
    let net_guard = state.net.read().unwrap();
    let net: &Network = &net_guard;
    let gtfs = crate::game::build_schedule(net, false, None);
    let (trains, errors) = freight_for(net, &q, seed, &gtfs);
    let list: Vec<_> = trains
        .iter()
        .map(|f| {
            serde_json::json!({
                "id": f.gt.train,
                "corridor": f.corridor,
                "from": net.stop_name(&f.gt.from),
                "to": net.stop_name(&f.gt.to),
                "dep": crate::freight::hhmm(f.gt.stops.first().map(|s| s.d).unwrap_or(0) as f64),
                "length_m": f.stock.length_m,
                "vmax_kmh": f.stock.vmax_kmh,
            })
        })
        .collect();
    serde_json::json!({ "seed": seed, "trains": list, "errors": errors }).to_string()
}

/// Convierte el día generado de un escenario (`frt`, `seed`) en un escenario FIJO guardado (los
/// mismos trenes a la misma hora en todas las runs). `name` = nombre del nuevo escenario.
fn freight_materialize(state: &ServerState, query: &str) -> String {
    let q = parse_query(query);
    let seed: u64 = q.get("seed").and_then(|s| s.parse().ok()).unwrap_or(1);
    let id = q.get("frt").cloned().unwrap_or_default();
    let Some(sc) = crate::freight::scenario_by_id(&id) else {
        return serde_json::json!({ "ok": false, "error": "Escenario desconocido" }).to_string();
    };
    let net_guard = state.net.read().unwrap();
    let gtfs = crate::game::build_schedule(&net_guard, false, None);
    let start = gtfs.iter().filter_map(|t| t.stops.first().map(|s| s.d)).min().unwrap_or(5 * 3600) as f64;
    let trains = crate::freight::plan(&sc, seed, start + 300.0, 24.0 * 3600.0);
    let name = q
        .get("name")
        .cloned()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("{} · semilla {seed}", sc.name));
    let mut all = crate::freight::load_scenarios();
    let new_id = format!("fijo_{}_{}", id, seed);
    all.retain(|s| s.id != new_id);
    all.push(crate::freight::FreightScenario {
        id: new_id.clone(),
        name,
        kind: "fixed".into(),
        corridors: Vec::new(),
        trains,
        avoid_peak: sc.avoid_peak,
        note: format!("Día generado de «{}» con la semilla {seed}.", sc.name),
    });
    let ok = crate::freight::save_scenarios(&all);
    serde_json::json!({ "ok": ok, "id": new_id, "scenarios": all }).to_string()
}

// ---- Datos reales de Renfe (GTFS-RT): recolector, recogida automática, análisis y comparación ----

/// Puerto de control del recolector (`scripts/collect_gtfsrt.py`): instancia única + ping/stop.
const GTFSRT_CTRL: &str = "127.0.0.1:47231";

/// Pregunta al recolector en marcha por su estado. `None` = no hay ninguno escuchando.
fn gtfsrt_ctrl(cmd: &str) -> Option<String> {
    use std::io::{Read, Write};
    let addr: std::net::SocketAddr = GTFSRT_CTRL.parse().ok()?;
    let mut s = std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(400)).ok()?;
    s.set_read_timeout(Some(std::time::Duration::from_millis(800))).ok();
    s.write_all(cmd.as_bytes()).ok()?;
    let mut out = String::new();
    s.read_to_string(&mut out).ok();
    Some(out.trim().to_string())
}

/// Estado del recolector del PC (puerto de control): si graba, si está en espera de la nube, último
/// guardado y siguiente lectura. Lo grabado por día lo da `/api/gtfsrt/days`.
fn gtfsrt_status() -> String {
    let running: Option<serde_json::Value> = gtfsrt_ctrl("ping").and_then(|s| serde_json::from_str(&s).ok());
    serde_json::json!({ "running": running }).to_string()
}

/// Ejecuta `scripts/gtfsrt_autostart.ps1 -Action <a>` (tarea programada de Windows) → su JSON.
fn gtfsrt_autostart(action: &str) -> String {
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "scripts/gtfsrt_autostart.ps1", "-Action", action])
        .output();
    match out {
        Ok(o) => {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if s.starts_with('{') {
                s
            } else {
                serde_json::json!({ "ok": false, "error": format!("{s} {}", String::from_utf8_lossy(&o.stderr).trim()) }).to_string()
            }
        }
        Err(e) => serde_json::json!({ "ok": false, "error": format!("No se pudo ejecutar PowerShell: {e}") }).to_string(),
    }
}

/// Arranca el recolector ahora (sin consola, independiente del servidor). Si ya hay uno, no hace nada.
fn gtfsrt_start() -> String {
    if gtfsrt_ctrl("ping").is_some() {
        return serde_json::json!({ "ok": true, "already": true }).to_string();
    }
    let script = "scripts/collect_gtfsrt.py";
    for py in ["pythonw", "python", "py"] {
        if std::process::Command::new(py)
            .args([script, "--every", "20"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .is_ok()
        {
            return serde_json::json!({ "ok": true, "python": py }).to_string();
        }
    }
    serde_json::json!({ "ok": false, "error": "No se encuentra Python para arrancar el recolector." }).to_string()
}

fn gtfsrt_stop() -> String {
    match gtfsrt_ctrl("stop") {
        Some(_) => serde_json::json!({ "ok": true }).to_string(),
        None => serde_json::json!({ "ok": true, "was_running": false }).to_string(),
    }
}

/// Ejecuta un script de `scripts/` con el Python que haya → su salida estándar, o la última línea
/// de error.
fn py_script(args: &[&str]) -> Result<String, String> {
    let mut last_err = String::from("no se encuentra Python");
    for py in ["python", "py", "python3"] {
        match std::process::Command::new(py).args(args).output() {
            Ok(o) if o.status.success() => return Ok(String::from_utf8_lossy(&o.stdout).trim().to_string()),
            Ok(o) => last_err = String::from_utf8_lossy(&o.stderr).trim().lines().last().unwrap_or("").to_string(),
            Err(e) => last_err = e.to_string(),
        }
    }
    Err(last_err)
}

/// `day=AAAA-MM-DD` validado (se pasa a scripts y nombres de fichero: nada de rutas).
fn day_param(query: &str) -> Option<String> {
    parse_query(query).get("day").filter(|d| {
        d.len() == 10 && d.bytes().enumerate().all(|(i, b)| if i == 4 || i == 7 { b == b'-' } else { b.is_ascii_digit() })
    }).cloned()
}

/// Incidencias de un día (`scripts/incidents.py list`): las publicadas ese día con su hora + los avisos
/// de Renfe grabados ese día. Sin día = hoy.
fn incidents_list(day: Option<String>) -> String {
    let mut args = vec!["scripts/incidents.py", "list"];
    if let Some(d) = day.as_deref() {
        args.extend(["--day", d]);
    }
    py_script(&args).unwrap_or_else(|e| serde_json::json!({ "items": [], "error": format!("No se pudieron leer las incidencias: {e}") }).to_string())
}

/// Captura AHORA las incidencias publicadas en los canales oficiales y las asocia a su día.
fn incidents_capture() -> String {
    match py_script(&["scripts/incidents.py", "capture"]) {
        Ok(s) => serde_json::from_str::<serde_json::Value>(&s)
            .map(|v| serde_json::json!({ "ok": true, "result": v }).to_string())
            .unwrap_or_else(|_| serde_json::json!({ "ok": false, "error": s }).to_string()),
        Err(e) => serde_json::json!({ "ok": false, "error": e }).to_string(),
    }
}

/// Resumen por día de los datos grabados (PC + nube): cobertura, puntualidad y valoración
/// (`scripts/gtfsrt_days.py`, con caché: solo recalcula los días nuevos o cambiados).
fn gtfsrt_days() -> String {
    match py_script(&["scripts/gtfsrt_days.py"]) {
        Ok(_) => std::fs::read_to_string("processed/gtfsrt/days.json")
            .unwrap_or_else(|_| serde_json::json!({ "days": [] }).to_string()),
        Err(e) => serde_json::json!({ "days": [], "error": format!("No se pudo resumir los días: {e}") }).to_string(),
    }
}

/// Ejecuta `scripts/analyze_gtfsrt.py` (tiempos de tramo/parada, retrasos y cancelaciones reales)
/// de un día (`observed_<día>.json`, cacheado) o de todos (`observed.json`, el que usa `compare`), y
/// devuelve el resumen por línea.
fn gtfsrt_analyze(day: Option<String>) -> String {
    let mut args = vec!["scripts/analyze_gtfsrt.py", "--min-n", "2"];
    if let Some(d) = day.as_deref() {
        args.extend(["--day", d]);
    }
    if let Err(e) = py_script(&args) {
        return serde_json::json!({ "ok": false, "error": e }).to_string();
    }
    let path = match day.as_deref() {
        Some(d) => format!("processed/gtfsrt/observed_{d}.json"),
        None => crate::compare::OBSERVED_PATH.to_string(),
    };
    let o: serde_json::Value =
        std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    serde_json::json!({
        "ok": true,
        "observed": {
            "days": o["days"],
            "polls": o["polls"],
            "trips": o["trips"],
            "segments": o["segments"].as_object().map(|m| m.len()).unwrap_or(0),
            "delays_by_line": o["delays_by_line"],
            "canceled_by_line": o["canceled_by_line"],
        },
    })
    .to_string()
}

/// Tras desplegar, arrancar o parar la nube, el recolector del PC lo comprueba YA (no a los 2 min)
/// para que nunca graben los dos a la vez. Se repite a los 8 s: al arrancar, la nube tarda ~1 s en
/// dar su primera señal de vida.
fn cloud_then_recheck(result: String) -> String {
    gtfsrt_ctrl("recheck");
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(8));
        gtfsrt_ctrl("recheck");
    });
    result
}

/// Al abrir el simulador, y cada 15 min mientras esté abierto, descarga los datos nuevos de la nube
/// (en segundo plano; `cloud::sync` no duplica ni hace dos descargas a la vez).
fn start_cloud_autosync() {
    std::thread::spawn(|| loop {
        if crate::cloud::CloudCfg::load().deployed {
            let r: serde_json::Value = serde_json::from_str(&crate::cloud::sync()).unwrap_or_default();
            if let Some(n) = r["downloaded"].as_u64().filter(|&n| n > 0) {
                println!("☁ Dades noves del núvol: {n} sondeigs, {} avisos", r["alerts"].as_u64().unwrap_or(0));
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(15 * 60));
    });
}

/// Comparación simulador (antes / ahora) frente a Renfe (horario y datos reales), como `compare`.
fn compare_json(state: &ServerState, query: &str) -> String {
    let q = parse_query(query);
    let seed: u64 = q.get("seed").and_then(|s| s.parse().ok()).unwrap_or(1);
    let net_guard = state.net.read().unwrap();
    let ltv_guard = state.ltv.read().unwrap();
    let rep = crate::compare::compare(&net_guard, state.adif.as_ref(), ltv_guard.as_deref(), seed);
    serde_json::to_string(&rep).unwrap_or_else(|_| "{}".into())
}

fn render_fragment(state: &ServerState, query: &str) -> String {
    let net_guard = state.net.read().unwrap();
    let net: &Network = &net_guard;
    let p = params_from_query(&parse_query(query));
    let ltv_guard = state.ltv.read().unwrap();
    let summary = scenario::summary_view(net, state.load_ms);
    let example = scenario::example_view(net, "25412", p.line.as_deref());
    let sim = scenario::sim_view(net, state.adif.as_ref(), ltv_guard.as_deref(), &p);
    let res = scenario::res_view(net, state.adif.as_ref(), ltv_guard.as_deref(), &p);
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

/// Genera el fichero de resultados de las runs (`fmt=xlsx|pdf`, `horarios=1` añade el horario
/// usado). El cuerpo es el JSON que ya tiene el navegador (runs + plantillas seleccionadas); el
/// servidor añade de qué GTFS salen los horarios. Responde el binario (el navegador lo descarga).
fn export_runs(state: &ServerState, query: &str, body: &[u8]) -> (&'static str, &'static str, Vec<u8>) {
    let q = parse_query(query);
    let req: crate::runs_export::ExportReq = match serde_json::from_slice(body) {
        Ok(r) => r,
        Err(e) => return ("400 Bad Request", "text/plain", format!("JSON inválido: {e}").into_bytes()),
    };
    let info: serde_json::Value = serde_json::from_str(&gtfs_info_json(state)).unwrap_or_default();
    // Malla cadenciada de la tanda (`cad=`): sus horarios son los que se exportan y se rotulan.
    let cad = match cadence_param(state, &q) {
        Some(Ok(t)) => Some(t),
        Some(Err(e)) => return ("400 Bad Request", "text/plain", e.into_bytes()),
        None => None,
    };
    let meta = crate::runs_export::Meta {
        generated: now_utc_string(),
        gtfs_date: info["date"].as_str().unwrap_or("desconocida").to_string(),
        service_id: if cad.is_some() {
            format!("horario cadenciado «{}»", q.get("cadname").map(|s| s.as_str()).unwrap_or("sin nombre"))
        } else {
            info["service_id"].as_str().unwrap_or("").to_string()
        },
    };
    let net = state.net.read().unwrap();
    let sched = if q.get("horarios").map(|v| v == "1").unwrap_or(false) {
        Some(match &cad {
            Some(t) => (**t).clone(),
            None => crate::game::build_schedule(&net, false, None),
        })
    } else {
        None
    };
    let sched_ref = sched.as_deref().map(|s| (&**net, s));
    let out = if q.get("fmt").map(|f| f == "pdf").unwrap_or(false) {
        crate::runs_export::pdf(&req, &meta, sched_ref).map(|b| ("application/pdf", b))
    } else {
        crate::runs_export::xlsx(&req, &meta, sched_ref)
            .map(|b| ("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", b))
    };
    match out {
        Ok((ct, b)) => ("200 OK", ct, b),
        Err(e) => ("500 Internal Server Error", "text/plain", e.into_bytes()),
    }
}

/// Ficha del GTFS GUARDADO (el cargado ahora): de qué día es (mtime de `stop_times.txt`, que se
/// reescribe en cada descarga/importación), servicio dominante y tamaño. Alimenta el selector de
/// horarios del juego y de /numeros ("GTFS guardado del …").
fn gtfs_info_json(state: &ServerState) -> String {
    let date = std::fs::metadata("data/gtfs/stop_times.txt")
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| scenario::date_from_unix(d.as_secs()));
    let net = state.net.read().unwrap();
    let n_lines = state.lines.read().unwrap().len();
    let sid = net.dominant_service().unwrap_or_default();
    // Salida del PRIMER tren del día (s): el motor arranca la jornada ahí (no a las 05:00 fijas).
    let first_dep = net
        .services
        .iter()
        .filter(|s| !s.is_bus && s.service_id == sid)
        .filter_map(|s| s.first_time())
        .min();
    serde_json::json!({
        "date": date,
        "first_dep": first_dep,
        "service_id": sid,
        "n_trips": net.services.len(),
        "n_lines": n_lines,
    })
    .to_string()
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

    // Física del motor unificado (misma que ve el juego); si falta, cae a Vmax por defecto.
    let adif = crate::calculator::infrastructure::AdifNet::load_default();
    let ltv = crate::calculator::ltv::load_default();
    let res = optimizer::optimize_system(&net, adif.as_ref(), ltv.as_ref(), sc, w, &cb);

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
