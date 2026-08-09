//! Servidor web local (tokio) para la UI interactiva. Sirve:
//!   * `GET /`            → página con controles + dashboard inicial + JS.
//!   * `GET /api/render`  → fragmento HTML del dashboard para unos parámetros.
//!
//! Es un servidor HTTP/1.1 mínimo (solo GET, `Connection: close`) pensado para
//! `127.0.0.1`; sin dependencias de framework, solo `tokio`.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::gtfs_loader::Network;
use crate::report::{self, Controls};
use crate::scenario::{self, now_utc_string, SimParams};

pub struct ServerState {
    pub net: Arc<Network>,
    pub load_ms: f64,
    pub lines: Vec<String>,
}

pub async fn serve(state: Arc<ServerState>, port: u16) -> std::io::Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    println!("\n🌐 Servidor interactiu a  http://127.0.0.1:{port}");
    println!("   (Ctrl+C per aturar)\n");
    loop {
        let (stream, _) = listener.accept().await?;
        let st = state.clone();
        tokio::spawn(async move {
            if let Err(e) = handle(stream, st).await {
                eprintln!("  · connexió tancada: {e}");
            }
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

    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p, q),
        None => (target, ""),
    };

    let (status, ctype, body) = match path {
        "/" => ("200 OK", "text/html", render_page(&state)),
        "/api/render" => ("200 OK", "text/html", render_fragment(&state, query)),
        "/health" => ("200 OK", "text/plain", "ok".to_string()),
        _ => ("404 Not Found", "text/plain", "404".to_string()),
    };

    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await?;
    stream.flush().await?;
    Ok(())
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
    report::render_interactive_page(
        &summary,
        &example,
        &sim,
        &res,
        &state.lines,
        &controls,
        &now_utc_string(),
    )
}

fn render_fragment(state: &ServerState, query: &str) -> String {
    let net = &state.net;
    let q = parse_query(query);
    let p = params_from_query(&q);
    let summary = scenario::summary_view(net, state.load_ms);
    let example = scenario::example_view(net, "25412", p.line.as_deref());
    let sim = scenario::sim_view(net, &p);
    let res = scenario::res_view(net, &p);
    report::render_body(&summary, &example, &sim, &res, &now_utc_string())
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

/// Decodificación mínima de percent-encoding (`%XX` y `+` → espacio).
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
                let h = hex(bytes[i + 1]);
                let l = hex(bytes[i + 2]);
                if let (Some(h), Some(l)) = (h, l) {
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
