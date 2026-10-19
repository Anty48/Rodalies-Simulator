//! **Recolector en la nube (Cloudflare Workers, plan gratuito)**: el recolector de datos reales
//! (GTFS-RT de Renfe) corriendo 24/7 sin servidor propio, controlado desde el simulador.
//!
//! El programa de la nube está en `scripts/cloudflare/` (un Durable Object con una alarma cada 20 s;
//! ver su cabecera para los límites gratuitos). El simulador lo despliega con la herramienta oficial
//! `wrangler` (vía `npx`, Node ya instalado), con una carpeta de trabajo propia
//! (`report/cloudflare-worker/`, fuera del repositorio) y una clave aleatoria que protege la API
//! (`report/cloudflare.json`, ignorado por git). Las consultas van por HTTPS con `curl` (Windows).
//! Los datos se descargan a `raw/gtfsrt/cloud/` con el mismo formato que el recolector del PC
//! (el analizador junta PC + nube sin duplicar).
//!
//! Gratis: una cuenta de Cloudflare sin tarjeta está en el plan Workers Free; si alguna vez se
//! superaran sus límites, Cloudflare simplemente rechaza las ejecuciones (no puede cobrar).

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

const CFG_PATH: &str = "report/cloudflare.json";
const WORK_DIR: &str = "report/cloudflare-worker";
const SRC_DIR: &str = "scripts/cloudflare";
const LOCAL_DIR: &str = "raw/gtfsrt/cloud";
/// Incidencias por día de publicación (mismo formato que `scripts/incidents.py`).
const INC_DIR: &str = "raw/incidents";
const WRANGLER: &str = "wrangler@4";

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct CloudCfg {
    /// URL pública del Worker (https://rod4lia-gtfsrt.<subdominio>.workers.dev).
    pub url: String,
    /// Clave de la API del Worker (secreto `TOKEN` en Cloudflare).
    pub token: String,
    pub deployed: bool,
    /// Último sondeo descargado (t), último aviso (seen) y última incidencia capturada (captured):
    /// descargas incrementales.
    pub last_t: i64,
    pub last_alert: i64,
    pub last_inc: i64,
}

impl CloudCfg {
    pub fn load() -> Self {
        std::fs::read_to_string(CFG_PATH).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }
    fn save(&self) {
        std::fs::create_dir_all("report").ok();
        std::fs::write(CFG_PATH, serde_json::to_string_pretty(self).unwrap_or_default()).ok();
    }
}

fn err(msg: impl std::fmt::Display) -> String {
    serde_json::json!({ "ok": false, "error": msg.to_string() }).to_string()
}

/// `npx wrangler@4 <args>` (en Windows `npx` es un .cmd: se lanza a través de `cmd /C`).
fn wrangler(args: &[&str], cwd: &str, interactive: bool) -> Command {
    let mut c = Command::new("cmd");
    c.arg("/C").arg("npx").arg("--yes").arg(WRANGLER).args(args).current_dir(cwd);
    c.env("WRANGLER_SEND_METRICS", "false");
    if !interactive {
        c.env("CI", "true"); // sin preguntas: si falta algo, falla con un mensaje
    }
    c
}

fn output_text(o: &std::process::Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

/// Estado de la configuración para la página.
pub fn info_json() -> String {
    let cfg = CloudCfg::load();
    serde_json::json!({ "deployed": cfg.deployed, "url": cfg.url }).to_string()
}

/// Abre el navegador para autorizar a `wrangler` en tu cuenta de Cloudflare (no bloquea).
pub fn login() -> String {
    std::fs::create_dir_all("report").ok();
    match wrangler(&["login"], "report", true).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
        Ok(_) => serde_json::json!({ "ok": true }).to_string(),
        Err(e) => err(format!("No se pudo ejecutar npx (¿Node.js instalado?): {e}")),
    }
}

/// ¿Está `wrangler` autorizado? → cuenta/email.
pub fn whoami() -> String {
    std::fs::create_dir_all("report").ok();
    match wrangler(&["whoami"], "report", false).output() {
        Ok(o) => {
            let txt = output_text(&o);
            let logged = txt.contains("You are logged in") || txt.contains("associated with the email");
            let email = txt
                .split("associated with the email")
                .nth(1)
                .map(|s| s.trim().trim_end_matches('.').split_whitespace().next().unwrap_or("").trim_end_matches('.').to_string());
            serde_json::json!({ "ok": true, "logged_in": logged, "email": email }).to_string()
        }
        Err(e) => err(format!("No se pudo ejecutar npx (¿Node.js instalado?): {e}")),
    }
}

fn random_token() -> String {
    use rand::Rng;
    let mut r = rand::thread_rng();
    (0..32).map(|_| format!("{:02x}", r.gen::<u8>())).collect()
}

/// Despliega (o actualiza) el recolector en Cloudflare, guarda su URL y su clave y lo arranca.
pub fn deploy() -> String {
    let mut cfg = CloudCfg::load();
    // Carpeta de trabajo con una copia del programa (wrangler deja allí su estado `.wrangler/`).
    std::fs::create_dir_all(format!("{WORK_DIR}/src")).ok();
    for f in ["wrangler.toml", "src/index.js"] {
        if let Err(e) = std::fs::copy(format!("{SRC_DIR}/{f}"), format!("{WORK_DIR}/{f}")) {
            return err(format!("No se pudo preparar {f}: {e}"));
        }
    }
    let out = match wrangler(&["deploy"], WORK_DIR, false).output() {
        Ok(o) => o,
        Err(e) => return err(format!("No se pudo ejecutar npx (¿Node.js instalado?): {e}")),
    };
    let log = output_text(&out);
    if !out.status.success() {
        let hint = if log.contains("workers.dev") && log.to_lowercase().contains("subdomain") {
            "\n\nFalta tu subdominio workers.dev: entra en dash.cloudflare.com → «Workers & Pages» una vez (lo crea al abrirlo) y vuelve a pulsar «Desplegar»."
        } else if log.contains("Not logged in") || log.contains("not authenticated") || log.contains("login") {
            "\n\nPrimero pulsa «Conectar con Cloudflare» y autoriza en el navegador."
        } else {
            ""
        };
        return serde_json::json!({ "ok": false, "error": format!("El despliegue falló.{hint}"), "log": log }).to_string();
    }
    let url = log
        .split_whitespace()
        .find(|w| w.starts_with("https://") && w.contains(".workers.dev"))
        .map(|s| s.trim_end_matches(|c: char| !c.is_ascii_alphanumeric() && c != '/').to_string());
    let Some(url) = url else {
        return serde_json::json!({ "ok": false, "error": "Desplegado, pero no encuentro la URL workers.dev en la salida.", "log": log }).to_string();
    };
    if cfg.token.is_empty() {
        cfg.token = random_token();
    }
    // La clave va como secreto del Worker (por la entrada estándar: no queda en ninguna línea de órdenes).
    let mut child = match wrangler(&["secret", "put", "TOKEN"], WORK_DIR, false)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return err(e),
    };
    if let Some(mut si) = child.stdin.take() {
        let _ = si.write_all(cfg.token.as_bytes());
    }
    let sec = match child.wait_with_output() {
        Ok(o) => o,
        Err(e) => return err(e),
    };
    if !sec.status.success() {
        return serde_json::json!({ "ok": false, "error": "No se pudo guardar la clave del Worker.", "log": format!("{log}\n{}", output_text(&sec)) }).to_string();
    }
    cfg.url = url.trim_end_matches('/').to_string();
    cfg.deployed = true;
    cfg.save();
    // Arrancar la recogida (los secretos tardan unos segundos en propagarse: se reintenta).
    let mut started = false;
    for _ in 0..6 {
        if api(&cfg, "POST", "/start").is_ok() {
            started = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(5));
    }
    serde_json::json!({ "ok": true, "url": cfg.url, "started": started, "log": log }).to_string()
}

/// Llamada a la API del Worker con su clave. Devuelve el cuerpo.
fn api(cfg: &CloudCfg, method: &str, path: &str) -> Result<Vec<u8>, String> {
    if !cfg.deployed || cfg.url.is_empty() {
        return Err("El recolector en la nube no está desplegado.".into());
    }
    let out = Command::new("curl")
        .args(["-s", "-f", "--compressed", "--max-time", "120", "-X", method])
        .arg("-H")
        .arg(format!("Authorization: Bearer {}", cfg.token))
        .arg(format!("{}{}", cfg.url, path))
        .output()
        .map_err(|e| format!("No se pudo ejecutar curl: {e}"))?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(format!("La nube no responde ({}).", String::from_utf8_lossy(&out.stderr).trim().trim_start_matches("curl: ")))
    }
}

/// Estado del recolector en la nube (último guardado, siguiente, días…).
pub fn status() -> String {
    let cfg = CloudCfg::load();
    if !cfg.deployed {
        return serde_json::json!({ "ok": true, "deployed": false }).to_string();
    }
    match api(&cfg, "GET", "/status").map(|b| serde_json::from_slice::<serde_json::Value>(&b)) {
        Ok(Ok(mut v)) => {
            v["ok"] = true.into();
            v["deployed"] = true.into();
            v["url"] = cfg.url.into();
            v.to_string()
        }
        Ok(Err(e)) => err(format!("Respuesta inesperada de la nube: {e}")),
        Err(e) => err(e),
    }
}

pub fn service(action: &str) -> String {
    let cfg = CloudCfg::load();
    match api(&cfg, "POST", if action == "stop" { "/stop" } else { "/start" }) {
        Ok(_) => serde_json::json!({ "ok": true }).to_string(),
        Err(e) => err(e),
    }
}

/// Fecha UTC "AAAA-MM-DD" de un instante (para nombrar el fichero de avisos).
fn utc_day(t: i64) -> String {
    let days = t.div_euclid(86400);
    // Algoritmo civil de Howard Hinnant (días desde 1970-01-01 → fecha).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn append(path: &Path, lines: &[String]) -> std::io::Result<()> {
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    for l in lines {
        f.write_all(l.as_bytes())?;
        f.write_all(b"\n")?;
    }
    Ok(())
}

/// Último `t` guardado en un fichero de sondeos (se escriben en orden: basta la última línea).
/// Lee solo el final del fichero (el del día pesa ~20 MB).
fn last_t_in_file(path: &Path) -> i64 {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else { return 0 };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(64 * 1024);
    if f.seek(SeekFrom::Start(start)).is_err() {
        return 0;
    }
    let mut buf = String::new();
    if f.read_to_string(&mut buf).is_err() {
        return 0;
    }
    buf.lines()
        .rev()
        .filter(|l| !l.trim().is_empty())
        .find_map(|l| serde_json::from_str::<serde_json::Value>(l).ok().and_then(|v| v["t"].as_i64()))
        .unwrap_or(0)
}

/// Descarga los sondeos y avisos nuevos a `raw/gtfsrt/cloud/` (incremental, por tandas de 1.000).
/// SIN DUPLICADOS aunque se pulse dos veces, se corte a medias o se haya «olvidado» la nube:
/// una sola descarga a la vez, y a cada fichero de día solo se añaden sondeos posteriores a su
/// última línea (además del cursor `last_t`).
pub fn sync() -> String {
    static BUSY: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let Ok(_guard) = BUSY.try_lock() else {
        return serde_json::json!({ "ok": true, "busy": true, "downloaded": 0, "alerts": 0, "mb": 0.0 }).to_string();
    };
    let mut cfg = CloudCfg::load();
    std::fs::create_dir_all(LOCAL_DIR).ok();
    let mut polls = 0usize;
    let mut bytes = 0usize;
    loop {
        let body = match api(&cfg, "GET", &format!("/data?since={}&limit=1000", cfg.last_t)) {
            Ok(b) => b,
            Err(e) => {
                cfg.save();
                return err(format!("Descarga interrumpida tras {polls} sondeos: {e}"));
            }
        };
        bytes += body.len();
        let text = String::from_utf8_lossy(&body);
        let mut by_day: std::collections::BTreeMap<String, Vec<String>> = Default::default();
        let mut n = 0usize;
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
            let (Some(day), Some(t)) = (v["day"].as_str(), v["rec"]["t"].as_i64()) else { continue };
            if day.contains(['/', '\\', '.']) {
                continue;
            }
            by_day.entry(day.to_string()).or_default().push(v["rec"].to_string());
            cfg.last_t = cfg.last_t.max(t);
            n += 1;
        }
        let mut written = 0usize;
        for (day, lines) in &by_day {
            let path = Path::new(LOCAL_DIR).join(format!("{day}.jsonl"));
            let have = last_t_in_file(&path);
            let fresh: Vec<String> = lines
                .iter()
                .filter(|l| {
                    serde_json::from_str::<serde_json::Value>(l).ok().and_then(|v| v["t"].as_i64()).unwrap_or(0) > have
                })
                .cloned()
                .collect();
            if let Err(e) = append(&path, &fresh) {
                cfg.save();
                return err(format!("No se pudo escribir {day}.jsonl: {e}"));
            }
            written += fresh.len();
        }
        polls += written;
        cfg.save();
        if n < 1000 {
            break;
        }
    }
    let mut alerts = 0usize;
    if let Ok(body) = api(&cfg, "GET", &format!("/alerts?since={}", cfg.last_alert)) {
        let mut by_day: std::collections::BTreeMap<String, Vec<String>> = Default::default();
        for line in String::from_utf8_lossy(&body).lines().filter(|l| !l.trim().is_empty()) {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
            let seen = v["seen"].as_i64().unwrap_or(0);
            by_day.entry(utc_day(seen)).or_default().push(v.to_string());
            cfg.last_alert = cfg.last_alert.max(seen);
            alerts += 1;
        }
        for (day, lines) in &by_day {
            let _ = append(&Path::new(LOCAL_DIR).join(format!("alerts_{day}.jsonl")), lines);
        }
        cfg.save();
    }
    let incidents = sync_incidents(&mut cfg);
    serde_json::json!({ "ok": true, "downloaded": polls, "alerts": alerts, "incidents": incidents, "mb": bytes as f64 / 1e6 })
        .to_string()
}

/// Incidencias del RSS de la Generalitat que capturó la nube → `raw/incidents/<día>.jsonl`. Sin
/// repetir: la misma incidencia puede llegar también del PC o del botón (misma clave sha1).
fn sync_incidents(cfg: &mut CloudCfg) -> usize {
    let Ok(body) = api(cfg, "GET", &format!("/incidents?since={}", cfg.last_inc)) else { return 0 };
    let mut by_day: std::collections::BTreeMap<String, Vec<serde_json::Value>> = Default::default();
    for line in String::from_utf8_lossy(&body).lines().filter(|l| !l.trim().is_empty()) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let Some(day) = v["day"].as_str() else { continue };
        if day.contains(['/', '\\', '.']) {
            continue;
        }
        cfg.last_inc = cfg.last_inc.max(v["rec"]["captured"].as_i64().unwrap_or(0));
        by_day.entry(day.to_string()).or_default().push(v["rec"].clone());
    }
    std::fs::create_dir_all(INC_DIR).ok();
    let mut added = 0usize;
    for (day, recs) in &by_day {
        let path = Path::new(INC_DIR).join(format!("{day}.jsonl"));
        let mut have: std::collections::HashSet<String> = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter_map(|v| v["key"].as_str().map(str::to_string))
            .collect();
        let fresh: Vec<String> = recs
            .iter()
            .filter(|r| r["key"].as_str().is_some_and(|k| have.insert(k.to_string())))
            .map(|r| r.to_string())
            .collect();
        if append(&path, &fresh).is_ok() {
            added += fresh.len();
        }
    }
    cfg.save();
    added
}

/// Olvida la nube en este ordenador (el Worker sigue en Cloudflare: se borra desde su panel).
pub fn forget() -> String {
    CloudCfg::default().save();
    serde_json::json!({ "ok": true }).to_string()
}

#[cfg(test)]
mod tests {
    use super::utc_day;

    #[test]
    fn fecha_utc_de_un_instante() {
        assert_eq!(utc_day(0), "1970-01-01");
        assert_eq!(utc_day(1_792_399_392), "2026-10-19");
        assert_eq!(utc_day(951_782_400), "2000-02-29");
    }
}
