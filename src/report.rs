//! Generación de un dashboard HTML autocontenido (sin dependencias externas) para
//! visualizar el resultado de la simulación. Todo el CSS/JS va embebido, de modo que
//! el archivo se puede abrir con doble clic y funciona sin conexión.

use crate::gtfs_loader::fmt_hms;

// --------------------------------------------------------------------------
// Vistas (datos ya calculados que alimentan el HTML)
// --------------------------------------------------------------------------

pub struct SummaryView {
    pub load_ms: f64,
    pub n_stops: usize,
    pub n_edges: usize,
    pub n_services: usize,
    pub n_routes: usize,
    pub with_parent: usize,
    pub per_line: Vec<(String, usize)>,
    pub fastest_edge: Option<(String, String, u32)>,
}

pub struct StopRow {
    pub seq: u32,
    pub name: String,
    pub arr: u32,
    pub dep: u32,
    pub run_secs: u32,
    pub track: u32,
}

pub struct ExampleView {
    pub found: bool,
    pub wanted: String,
    pub train_number: String,
    pub route_short: String,
    pub route_id: String,
    pub trip_id: String,
    pub headsign: String,
    pub stops: Vec<StopRow>,
}

pub struct SimView {
    pub window: String,
    pub service_id: String,
    pub key_stations: Vec<String>,
    pub busiest: Option<(String, String, usize)>,
    pub incidents: Vec<String>,
    pub trains_run: usize,
    pub held: usize,
    pub peak_total: i64,
    pub peak_time: String,
    pub recovery: Option<String>,
    /// Línea temporal del retraso acumulado de la red (`(instante_s, retraso_total_s)`) del motor.
    pub timeline: Vec<(u32, f64)>,
    /// SVG del mapa geográfico de la red con trenes animados.
    pub map_svg: String,
}

pub struct ResRow {
    pub mins: u32,
    pub peak: i64,
    pub recovery: Option<u32>,
    pub held: usize,
}

pub struct ResView {
    pub segment: String,
    pub rows: Vec<ResRow>,
}

// --------------------------------------------------------------------------
// Utilidades
// --------------------------------------------------------------------------

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn run_txt(secs: u32) -> String {
    if secs == 0 {
        "—".into()
    } else {
        format!("{}m{:02}s", secs / 60, secs % 60)
    }
}

/// Gráfico de área SVG del retraso acumulado de la red a lo largo del tiempo. `timeline` =
/// `(instante_s, retraso_total_s)` muestreado por el motor unificado (`Engine::timeline`).
fn timeline_svg(timeline: &[(u32, f64)]) -> String {
    if timeline.is_empty() {
        return "<p class=\"muted\">Sin datos de serie temporal.</p>".into();
    }
    let w = 1000.0f64;
    let h = 280.0f64;
    let pad_l = 60.0;
    let pad_r = 20.0;
    let pad_t = 20.0;
    let pad_b = 40.0;
    let plot_w = w - pad_l - pad_r;
    let plot_h = h - pad_t - pad_b;

    let max_d = timeline.iter().map(|s| s.1).fold(1.0_f64, f64::max);
    let n = timeline.len();
    let x = |i: usize| pad_l + plot_w * (i as f64) / ((n - 1).max(1) as f64);
    let y = |d: f64| pad_t + plot_h * (1.0 - (d / max_d));

    // Área.
    let mut area = String::new();
    area.push_str(&format!("M {:.1} {:.1}", x(0), y(timeline[0].1)));
    for (i, s) in timeline.iter().enumerate().skip(1) {
        area.push_str(&format!(" L {:.1} {:.1}", x(i), y(s.1)));
    }
    let line_path = area.clone();
    area.push_str(&format!(
        " L {:.1} {:.1} L {:.1} {:.1} Z",
        x(n - 1),
        pad_t + plot_h,
        x(0),
        pad_t + plot_h
    ));

    // Rejilla horizontal + etiquetas Y (0, mitad, máximo, en segundos).
    let mut grid = String::new();
    for frac in [0.0, 0.5, 1.0] {
        let val = (max_d * frac) as i64;
        let yy = pad_t + plot_h * (1.0 - frac);
        grid.push_str(&format!(
            "<line x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" class=\"grid\"/>",
            pad_l,
            yy,
            w - pad_r,
            yy
        ));
        grid.push_str(&format!(
            "<text x=\"{:.1}\" y=\"{:.1}\" class=\"axis\" text-anchor=\"end\">{} s</text>",
            pad_l - 8.0,
            yy + 4.0,
            val
        ));
    }

    // Etiquetas X (cada ~6 muestras / 6 min si el muestreo es 1/min).
    let mut xlab = String::new();
    let step = (n / 8).max(1);
    let mut i = 0;
    while i < n {
        let xx = x(i);
        xlab.push_str(&format!(
            "<text x=\"{:.1}\" y=\"{:.1}\" class=\"axis\" text-anchor=\"middle\">{}</text>",
            xx,
            h - 14.0,
            &fmt_hms(timeline[i].0)[..5]
        ));
        i += step;
    }

    format!(
        r##"<svg viewBox="0 0 {w:.0} {h:.0}" preserveAspectRatio="xMidYMid meet" class="chart" role="img" aria-label="Retraso acumulado de la red">
  <defs>
    <linearGradient id="grad" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0%" stop-color="var(--accent)" stop-opacity="0.55"/>
      <stop offset="100%" stop-color="var(--accent)" stop-opacity="0.02"/>
    </linearGradient>
  </defs>
  {grid}
  <path d="{area}" fill="url(#grad)"/>
  <path d="{line}" fill="none" stroke="var(--accent)" stroke-width="2.5"/>
</svg>"##,
        w = w,
        h = h,
        grid = grid.to_string() + &xlab,
        area = area,
        line = line_path
    )
}

// --------------------------------------------------------------------------
// Render principal
// --------------------------------------------------------------------------

/// Estado de los controles de la UI (valores actuales de los sliders/selects).
pub struct Controls {
    pub start_h: u32,
    pub dur_h: u32,
    pub line: Option<String>,
    pub block: u32,
    pub delay: u32,
    pub random: bool,
}

const STYLE: &str = r#"<style>
/* Paleta Rodalies de Catalunya / Renfe: blancos y grises de la librea de los
   trenes, con el rojo corporativo de Rodalies como único acento, usado con
   mesura. Aspecto instrumental/científico: superficies planas, sin degradados. */
:root {
  /* Paleta Renfe/Rodalies combinada: morado Renfe (#830065) y naranja Rodalies (#EC6E15)
     como dos acentos repartidos por la interfaz, sobre blanco cálido (#F9F5F1) y gris
     oscuro (#26272D) para texto y estructura. */
  --bg:#f9f5f1; --panel:#ffffff; --panel2:#f4efe9; --border:#e3dace;
  --text:#26272d; --muted:#6c6a72; --ink:#26272d;
  --accent:#830065; --accent-soft:rgba(131,0,101,.08); --accent2:#ec6e15;
  --accent2-soft:rgba(236,110,21,.10);
  --arr:#1b7f3b; --dep:#1f6feb; --late:#c5221f; --warn:#b25a00;
  --grid:#ece3d8;
}
* { box-sizing:border-box; }
body { margin:0; background:var(--bg); color:var(--text);
  font-family:'Segoe UI',system-ui,-apple-system,sans-serif; line-height:1.5;
  -webkit-font-smoothing:antialiased; }
.mono { font-family:'Cascadia Code',Consolas,'Courier New',monospace; }
.muted { color:var(--muted); font-size:.9em; }
code { background:var(--panel2); padding:1px 5px; border-radius:4px; font-size:.85em; border:1px solid var(--border); }
.wrap { max-width:1200px; margin:0 auto; padding:24px 20px 60px; }
header.top { display:flex; align-items:center; gap:16px; padding:8px 0 20px; border-bottom:2px solid var(--ink); margin-bottom:24px; }
.logo { width:46px; height:46px; border-radius:6px; background:var(--accent); color:#fff;
  display:flex; align-items:center; justify-content:center; font-size:24px; font-weight:800;
  letter-spacing:-.02em; flex:0 0 auto; font-family:'Segoe UI',system-ui,sans-serif; }
h1 { font-size:1.45rem; margin:0; font-weight:700; letter-spacing:-.01em; }
h2 { font-size:.82rem; margin:0 0 14px; letter-spacing:.06em; text-transform:uppercase; color:var(--muted); font-weight:700; }
h3 { margin:0 0 4px; font-size:1.05rem; }
.sub { color:var(--muted); font-size:.85rem; }
.grid-kpi { display:grid; grid-template-columns:repeat(auto-fit,minmax(150px,1fr)); gap:14px; margin-bottom:26px; }
.kpi { background:var(--panel); border:1px solid var(--border); border-radius:6px; padding:16px 18px; border-top:3px solid var(--ink); }
.kpi-val { font-size:1.7rem; font-weight:700; font-variant-numeric:tabular-nums; }
.kpi-label { font-size:.9rem; margin-top:2px; }
.kpi-sub { color:var(--muted); font-size:.75rem; }
.card { background:var(--panel); border:1px solid var(--border); border-radius:6px; padding:22px; margin-bottom:22px; }
.cols { display:grid; grid-template-columns:1fr 1fr; gap:22px; }
@media (max-width:820px) { .cols { grid-template-columns:1fr; } }
.scroll { overflow-x:auto; }
table { width:100%; border-collapse:collapse; font-size:.9rem; }
th,td { text-align:left; padding:7px 10px; border-bottom:1px solid var(--border); white-space:nowrap; }
th { color:var(--muted); font-weight:700; font-size:.72rem; text-transform:uppercase; letter-spacing:.04em; border-bottom:2px solid var(--border); }
td.num { text-align:right; font-variant-numeric:tabular-nums; }
.pill { background:var(--panel2); border:1px solid var(--border); border-radius:4px; padding:1px 9px; font-size:.8rem; }
.chart { width:100%; height:auto; display:block; }
.map { width:100%; height:auto; display:block; background:var(--panel2); border:1px solid var(--border); border-radius:6px; }
.grid { stroke:var(--grid); stroke-width:1; }
.axis { fill:var(--muted); font-size:12px; font-family:monospace; }
.bar-row { display:flex; align-items:center; gap:10px; margin:6px 0; font-size:.85rem; }
.bar-name { width:52px; color:var(--muted); }
.bar-val { width:52px; text-align:right; font-variant-numeric:tabular-nums; }
.bar-track { flex:1; height:10px; background:var(--panel2); border:1px solid var(--border); border-radius:3px; overflow:hidden; }
.bar-track.sm { height:8px; }
.bar-fill { display:block; height:100%; background:var(--accent2); border-radius:2px; }
.metrics { display:grid; grid-template-columns:1fr 1fr; gap:10px 22px; }
.metric { display:flex; justify-content:space-between; border-bottom:1px dashed var(--border); padding:6px 0; font-size:.9rem; }
.metric-label { color:var(--muted); }
.metric-val { font-weight:600; font-variant-numeric:tabular-nums; }
.chips { display:flex; flex-wrap:wrap; gap:6px; margin-top:6px; }
.chip { background:var(--panel2); border:1px solid var(--border); border-radius:4px; padding:2px 10px; font-size:.78rem; color:var(--muted); }
.inc { list-style:none; padding:0; margin:0; }
.inc li { background:var(--accent-soft); border-left:3px solid var(--accent); padding:8px 12px; border-radius:0 4px 4px 0; margin-bottom:8px; font-size:.88rem; }
.log { max-height:520px; overflow:auto; border:1px solid var(--border); border-radius:6px; background:var(--panel); }
.ev { display:grid; grid-template-columns:64px 22px 58px 1fr 1.4fr 62px 66px; align-items:center; gap:8px; padding:4px 12px; font-size:.82rem; border-bottom:1px solid var(--border); }
.ev-inc { grid-template-columns:22px 1fr; background:var(--accent-soft); color:var(--accent); font-weight:600; }
.ev-badge { text-align:center; }
.ev-arr .ev-badge { color:var(--arr); }
.ev-dep .ev-badge { color:var(--dep); }
.ev-kind { color:var(--muted); font-size:.75rem; }
.ev-train em { color:var(--accent); font-style:normal; font-size:.78rem; }
.ev-sta { color:var(--text); }
.ev-track,.ev-delay { text-align:right; color:var(--muted); font-variant-numeric:tabular-nums; }
.ev.warn .ev-delay { color:var(--warn); }
.ev.late .ev-delay { color:var(--late); font-weight:700; }
footer { color:var(--muted); font-size:.8rem; text-align:center; padding-top:20px; border-top:1px solid var(--border); }
.controls { background:var(--panel); border:1px solid var(--border); border-radius:6px; padding:18px 22px; margin-bottom:22px; }
.controls-grid { display:grid; grid-template-columns:repeat(auto-fit,minmax(155px,1fr)); gap:16px 20px; align-items:end; }
.field { display:flex; flex-direction:column; gap:5px; font-size:.78rem; color:var(--muted); }
.field label b { color:var(--accent); font-variant-numeric:tabular-nums; }
.field input[type=range] { width:100%; accent-color:var(--accent); }
.field select { background:var(--panel); color:var(--text); border:1px solid var(--border); border-radius:4px; padding:7px 8px; }
.field .chk { display:flex; align-items:center; gap:7px; color:var(--text); font-size:.9rem; }
.btn { background:var(--accent); color:#fff; border:none; border-radius:4px;
  padding:11px 22px; font-size:.95rem; font-weight:600; cursor:pointer;
  text-decoration:none; display:inline-block; }
.btn:hover { background:#b71d16; }
.btn:disabled { opacity:.55; cursor:progress; }
#dashboard { transition:opacity .15s; }
.optchart { width:100%; height:auto; display:block; background:var(--panel); border:1px solid var(--border); border-radius:6px; margin-top:12px; }
.optdone { margin-top:12px; font-size:1rem; padding:12px 16px; background:var(--panel2); border:1px solid var(--border); border-radius:6px; }
.optdone a { color:var(--accent); font-weight:600; text-decoration:none; }
.optdone a:hover { text-decoration:underline; }
.formula { font-family:'Cascadia Code',Consolas,monospace; background:var(--panel2); border:1px solid var(--border); border-radius:6px; padding:10px 14px; font-size:.85rem; overflow-x:auto; }
.tabs { display:flex; gap:8px; margin-bottom:22px; border-bottom:2px solid var(--border); }
.tab { background:none; border:none; color:var(--muted); font-size:.95rem; font-weight:600; padding:10px 16px; cursor:pointer; border-bottom:2px solid transparent; margin-bottom:-2px; }
.tab.active { color:var(--accent); border-bottom-color:var(--accent); }
.tabpane { display:none; }
.tabpane.active { display:block; }
h4 { margin:16px 0 8px; font-size:.98rem; }
.series-row { display:flex; flex-wrap:wrap; gap:14px; }
.serie-chk { display:flex; align-items:center; gap:6px; color:var(--text); font-size:.9rem; background:var(--panel2); border:1px solid var(--border); border-radius:4px; padding:6px 12px; }
.serie-chk input[disabled]+b { color:var(--muted); }
.provb { font-size:.72rem; font-weight:600; border:1px solid var(--border); border-radius:4px; padding:1px 8px; }
.ficha { margin-top:16px; }
.src { margin:6px 0 0; padding-left:18px; font-size:.88rem; }
.src li { margin-bottom:8px; }
.src-note { font-style:italic; }
.dot { display:inline-block; width:10px; height:10px; border-radius:50%; margin-right:7px; vertical-align:middle; }
.calcmap { width:100%; height:440px; border-radius:6px; border:1px solid var(--border); background:var(--panel2); }
.legend { display:flex; flex-wrap:wrap; gap:14px; margin-top:10px; font-size:.82rem; color:var(--muted); }
.legend span { display:inline-flex; align-items:center; gap:6px; }
.legend i { width:14px; height:4px; border-radius:2px; display:inline-block; }
.qflag { display:inline-flex; align-items:center; gap:6px; font-size:.82rem; }
.qdot { width:11px; height:11px; border-radius:50%; display:inline-block; }
.q-oficial { background:var(--arr); } .q-secundaria { background:var(--dep); }
.q-estimacion { background:var(--warn); } .q-suposicion { background:var(--accent); } .q-nd { background:var(--late); }
.plot { width:100%; height:340px; }
.zoombtn { float:right; background:var(--panel2); border:1px solid var(--border); color:var(--text); border-radius:4px; padding:4px 10px; cursor:pointer; font-size:.8rem; }
.modal { display:none; position:fixed; inset:0; background:rgba(27,33,39,.55); z-index:1000; padding:3vh 3vw; }
.modal.open { display:block; }
.modal-inner { background:var(--panel); border:1px solid var(--border); border-radius:6px; width:100%; height:100%; padding:16px; position:relative; }
.modal-close { position:absolute; top:10px; right:14px; background:var(--accent); color:#fff; border:none; border-radius:4px; padding:6px 12px; cursor:pointer; font-weight:600; z-index:1; }
.modal-plot { width:100%; height:100%; }
.anom { background:rgba(154,103,0,.08); border-left:3px solid var(--warn); padding:8px 12px; border-radius:0 4px 4px 0; margin-bottom:8px; font-size:.86rem; }
.progress { height:8px; background:var(--panel2); border:1px solid var(--border); border-radius:4px; overflow:hidden; margin-top:8px; }
.progress > i { display:block; height:100%; width:0; background:var(--accent2); }
.subtabs { display:flex; gap:6px; flex-wrap:wrap; margin:6px 0 14px; }
.subtab { background:var(--panel2); border:1px solid var(--border); color:var(--muted); border-radius:4px; padding:6px 12px; cursor:pointer; font-size:.85rem; }
.subtab.active { color:var(--accent); border-color:var(--accent); }
.leaflet-popup-content { color:#111; }

/* ---- Tema oscuro (se activa con data-theme="dark" en <html>, lo pone el script
   THEME_INIT antes de pintar para evitar parpadeo). Mantiene la identidad Renfe/Rodalies
   subiendo la luminosidad de los acentos para que contrasten sobre fondo oscuro. ---- */
:root[data-theme="dark"] {
  --bg:#17181c; --panel:#212228; --panel2:#2a2c33; --border:#383a43;
  --text:#ece9e4; --muted:#9d9aa4; --ink:#ece9e4;
  --accent:#d072b8; --accent-soft:rgba(208,114,184,.14); --accent2:#ff8a3d;
  --accent2-soft:rgba(255,138,61,.14);
  --arr:#3fbf6a; --dep:#5aa0ff; --late:#ff6b68; --warn:#e0a03a;
  --grid:#2f313a;
}
:root[data-theme="dark"] .logo { color:#fff; }

/* ---- Menú principal ---- */
.menu-wrap { max-width:1040px; margin:0 auto; padding:40px 20px 60px; }
.menu-top { display:flex; align-items:center; justify-content:space-between; gap:16px; margin-bottom:40px; }
.menu-brand { display:flex; align-items:center; gap:16px; }
.gear { background:var(--panel); border:1px solid var(--border); color:var(--text);
  width:42px; height:42px; border-radius:8px; font-size:20px; cursor:pointer;
  display:flex; align-items:center; justify-content:center; transition:border-color .15s,transform .3s; }
.gear:hover { border-color:var(--accent); }
.gear:hover svg { transform:rotate(45deg); }
.gear svg { transition:transform .3s; }
.hero { margin-bottom:22px; }
.hero-card { display:block; position:relative; overflow:hidden; text-decoration:none; color:var(--text);
  background:var(--panel); border:1px solid var(--border); border-left:5px solid var(--accent);
  border-radius:10px; padding:34px 32px; transition:transform .15s,box-shadow .15s,border-color .15s; }
.hero-card:hover { transform:translateY(-3px); box-shadow:0 12px 30px rgba(0,0,0,.14); border-left-color:var(--accent2); }
.hero-badge { display:inline-block; font-size:.72rem; font-weight:700; letter-spacing:.08em;
  text-transform:uppercase; color:#fff; background:var(--accent); padding:3px 10px; border-radius:20px; margin-bottom:14px; }
.hero-card h2 { font-size:1.9rem; margin:0 0 8px; letter-spacing:-.02em; text-transform:none; color:var(--text); }
.hero-card p { color:var(--muted); font-size:.98rem; margin:0 0 18px; max-width:60ch; }
.hero-cta { font-weight:700; color:var(--accent); font-size:1.02rem; }
.hero-card:hover .hero-cta { color:var(--accent2); }
.tiles { display:grid; grid-template-columns:1fr 1fr; gap:18px; }
@media (max-width:720px) { .tiles { grid-template-columns:1fr; } }
.tile { display:block; text-decoration:none; color:var(--text); background:var(--panel);
  border:1px solid var(--border); border-top:3px solid var(--ink); border-radius:8px; padding:24px 24px;
  transition:transform .15s,box-shadow .15s,border-top-color .15s; }
.tile:hover { transform:translateY(-3px); box-shadow:0 10px 24px rgba(0,0,0,.12); border-top-color:var(--accent2); }
.tile-ic { font-size:1.8rem; line-height:1; margin-bottom:12px; }
.tile h3 { margin:0 0 6px; font-size:1.15rem; }
.tile p { color:var(--muted); font-size:.9rem; margin:0; }
.menu-more { text-align:center; margin-top:26px; font-size:.9rem; }
.menu-more a { color:var(--accent); text-decoration:none; font-weight:600; }
.menu-more a:hover { text-decoration:underline; }

/* ---- Barra del panel (volver al menú + configuración) ---- */
.panel-bar { display:flex; align-items:center; justify-content:space-between; gap:12px; margin:-8px 0 18px; }
.back-link { color:var(--muted); text-decoration:none; font-size:.9rem; font-weight:600; display:inline-flex; align-items:center; gap:6px; }
.back-link:hover { color:var(--accent); }

/* ---- Modal de configuración ---- */
.settings { display:none; position:fixed; inset:0; background:rgba(20,20,26,.55); z-index:2000;
  align-items:center; justify-content:center; padding:5vh 5vw; }
.settings.open { display:flex; }
.settings-inner { background:var(--panel); border:1px solid var(--border); border-radius:10px;
  width:100%; max-width:440px; padding:26px 26px 22px; position:relative; }
.settings-inner h2 { margin:0 0 18px; }
.settings-inner .settings-close { position:absolute; top:16px; right:18px; background:none; border:none;
  color:var(--muted); font-size:22px; cursor:pointer; line-height:1; }
.set-row { margin-bottom:20px; }
.set-row > label { display:block; font-size:.78rem; font-weight:700; letter-spacing:.05em;
  text-transform:uppercase; color:var(--muted); margin-bottom:8px; }
.seg { display:inline-flex; border:1px solid var(--border); border-radius:8px; overflow:hidden; }
.seg button { background:var(--panel2); border:none; color:var(--muted); padding:8px 16px; cursor:pointer;
  font-size:.9rem; font-weight:600; border-right:1px solid var(--border); }
.seg button:last-child { border-right:none; }
.seg button.active { background:var(--accent); color:#fff; }
</style>"#;

const HEADER: &str = r#"<header class="top">
    <div class="logo">R</div>
    <div>
      <h1>Rodalies Simulator <span class="sub">· panel de análisis</span> <span style="font-size:10px;opacity:.5;font-weight:400">v2.0</span></h1>
      <div class="sub">Simulación de tráfico ferroviario e incidencias · red de Rodalies de Catalunya · modelo construido a partir de datos GTFS</div>
    </div>
  </header>"#;

const SCRIPT: &str = r#"<script>
const $=id=>document.getElementById(id);
function qs(){
  const p=new URLSearchParams();
  p.set('start_h',$('c_start').value); p.set('dur_h',$('c_dur').value);
  p.set('line',$('c_line').value); p.set('block',$('c_block').value);
  p.set('delay',$('c_delay').value); p.set('random',$('c_random').checked?'1':'0');
  return p.toString();
}
async function simulate(){
  const b=$('c_run'); b.disabled=true; const t=b.textContent; b.textContent='Simulando…';
  const d=$('dashboard'); d.style.opacity=.4;
  try{ const r=await fetch('/api/render?'+qs()); d.innerHTML=await r.text(); }
  catch(e){ d.innerHTML='<div class="card">Error: '+e+'</div>'; }
  d.style.opacity=1; b.disabled=false; b.textContent=t;
}
document.addEventListener('input',e=>{ const o=e.target.dataset&&e.target.dataset.out; if(o)$(o).textContent=e.target.value; });
$('c_run').addEventListener('click',simulate);
</script>"#;

/// Panel del optimizador del SISTEMA (persistente, fuera de #dashboard).
fn optimizer_panel() -> String {
    r#"<div class="card">
    <h2>Optimizador del sistema · minimización del potencial V(H)</h2>
    <div class="formula">V(H) = w_delay·<b>retraso ponderado por pasajeros</b> (×3 en hora punta) + w_conf·<b>conflictos entre líneas</b> &nbsp;·&nbsp; integrado sobre el día completo (05:00–00:00)</div>
    <p class="muted" style="margin-top:8px">Coordina <b>todas las líneas simultáneamente</b>: el problema son los conflictos entre líneas en los cantones compartidos y la respuesta a las <b>incidencias</b>. Recocido simulado sobre el <b>desfase de fase de cada línea</b> (±5 min); cada candidato se evalúa con múltiples <b>simulaciones Monte Carlo del sistema completo</b>, cada una con incidencias aleatorias repartidas a lo largo del día (paralelizado con rayon). Física: cantones por bloques, vía única (testigo), vías por estación reales, autobuses de sustitución excluidos.</p>
    <button class="btn" id="o_run">Optimizar el sistema (día laborable)</button>
    <div id="o_status" class="muted" style="margin-top:10px">Preparado. Pulse para ver descender V(H) en tiempo real a lo largo de las simulaciones.</div>
    <canvas id="o_chart" width="920" height="190" class="optchart"></canvas>
    <div id="o_result"></div>
  </div>"#.to_string()
}

/// Panel lanzador del simulador de red (juego web). El juego se sirve en `/game` y carga la
/// topología y los horarios directamente desde la API (GTFS u optimizados).
fn game_panel() -> String {
    r#"<div class="card">
    <h2>Simulador de red · visualización dinámica del sistema</h2>
    <p class="muted" style="margin-top:8px">Simulador 2D de la red construido íntegramente a partir del GTFS de Fomento_Transit: estaciones, vías reales por estación y la <b>secuencia real de cada línea</b> (sin aproximaciones). Los trenes circulan por su ruta siguiendo el horario a lo largo de la jornada (05:00–00:00), con reloj y control de velocidad. Puede cargar el <b>horario programado (GTFS)</b> o los <b>horarios optimizados</b> que produce el optimizador, e inyectar incidencias para observar el retraso acumulado.</p>
    <div style="margin-top:14px;display:flex;gap:10px;flex-wrap:wrap">
      <a class="btn" href="/game" target="_blank" rel="noopener">Abrir con horario programado (GTFS)</a>
      <a class="btn" href="/game?source=optimized" target="_blank" rel="noopener" style="background:var(--accent2)">Abrir con horario optimizado</a>
    </div>
    <p class="muted" style="margin-top:10px">El horario optimizado requiere haber ejecutado antes el optimizador (los CSV en <code>report/optimized/</code>); si no existen, el juego usa el programado.</p>
  </div>"#.to_string()
}

const OPT_SCRIPT: &str = r#"<script>
(function(){
 const $=id=>document.getElementById(id);
 let timer=null;
 function draw(h,base){ const c=$('o_chart'); if(!c)return; const ctx=c.getContext('2d'); const W=c.width,H=c.height,P=34;
   ctx.clearRect(0,0,W,H); if(!h||!h.length)return;
   let mx=base||h[0],mn=h[0]; for(const v of h){if(v>mx)mx=v;if(v<mn)mn=v;} if(base){if(base<mn)mn=base;if(base>mx)mx=base;} if(mx-mn<1e-6)mx=mn+1;
   const x=i=>P+(W-2*P)*(h.length<2?1:i/(h.length-1)); const y=v=>P+(H-2*P)*(1-(v-mn)/(mx-mn));
   ctx.strokeStyle='#d3dae1';ctx.fillStyle='#5d6b78';ctx.font='11px monospace';ctx.lineWidth=1;
   ctx.beginPath();ctx.moveTo(P,y(mx));ctx.lineTo(W-P,y(mx));ctx.stroke();ctx.fillText(mx.toFixed(0),2,y(mx)+4);
   ctx.beginPath();ctx.moveTo(P,y(mn));ctx.lineTo(W-P,y(mn));ctx.stroke();ctx.fillText(mn.toFixed(0),2,y(mn)+4);
   if(base){ctx.strokeStyle='#9aa6b2';ctx.setLineDash([4,4]);ctx.beginPath();ctx.moveTo(P,y(base));ctx.lineTo(W-P,y(base));ctx.stroke();ctx.setLineDash([]);ctx.fillText('V inicial',W-P-52,y(base)-4);}
   ctx.strokeStyle='#830065';ctx.lineWidth=2;ctx.beginPath();h.forEach((v,i)=>{i?ctx.lineTo(x(i),y(v)):ctx.moveTo(x(i),y(v));});ctx.stroke();
 }
 async function poll(){ let j; try{j=await (await fetch('/api/optimize/status')).json();}catch(e){return;}
   draw(j.history,j.base_v);
   if(j.running){ $('o_status').textContent='Optimizando el sistema… iteración '+j.iter+'/'+j.total+'  ·  V actual '+j.current_v.toFixed(1)+'  ·  mejor '+j.best_v.toFixed(1); }
   else if(j.done){ if(timer){clearInterval(timer);timer=null;} $('o_run').disabled=false;
     if(j.error){ $('o_status').innerHTML='<span style="color:#c5221f">Error: '+j.error+'</span>'; }
     else {
       $('o_status').innerHTML='<b style="color:#1b7f3b">Optimización finalizada</b> · '+j.total+' iteraciones · '+j.trips+' trenes coordinados';
       let rows=(j.files||[]).map(f=>'<tr><td class=mono>'+f.line+'</td><td class=num>'+(f.offset_min>=0?'+':'')+f.offset_min+' min</td><td><a href="'+f.csv+'" target=_blank>CSV</a></td><td><a href="'+f.pdf+'" target=_blank>PDF</a></td></tr>').join('');
       $('o_result').innerHTML='<div class="optdone">Potencial V: <b>'+j.base_v.toFixed(1)+'</b> → <b style="color:#ec6e15">'+j.best_v.toFixed(1)+'</b> (<b>−'+j.delta_pct.toFixed(1)+'%</b>) &nbsp;·&nbsp; pico de retraso medio '+j.base_delay.toFixed(0)+' → '+j.best_delay.toFixed(0)+' s &nbsp;·&nbsp; recuperación '+j.base_recovery.toFixed(1)+' → '+j.best_recovery.toFixed(1)+' min</div>'
         +'<div class="scroll" style="margin-top:10px"><table><thead><tr><th>línea</th><th>desfase</th><th>horario</th><th></th></tr></thead><tbody>'+rows+'</tbody></table></div>';
     }
   }
 }
 function start(){ const b=$('o_run'); b.disabled=true; $('o_result').innerHTML=''; $('o_status').textContent='Iniciando… (simulando el sistema completo con incidencias)';
   fetch('/api/optimize/start').then(()=>{ if(timer)clearInterval(timer); timer=setInterval(poll,500); poll(); });
 }
 const b=$('o_run'); if(b) b.addEventListener('click',start);
})();
</script>"#;

/// Cambio de pestañas (sin frameworks).
const TAB_SCRIPT: &str = r#"<script>
document.querySelectorAll('.tab').forEach(t=>t.addEventListener('click',()=>{
  document.querySelectorAll('.tab').forEach(x=>x.classList.remove('active'));
  document.querySelectorAll('.tabpane').forEach(x=>x.classList.remove('active'));
  t.classList.add('active');
  document.getElementById(t.dataset.pane).classList.add('active');
}));
</script>"#;

/// Enlaces profundos desde el menú principal: `/panel#calc`, `/panel#opt`, `/panel#game`
/// activan la pestaña correspondiente. Se ejecuta el ÚLTIMO (tras CALC2_SCRIPT) para que el
/// `click()` sintético dispare también el listener que inicializa el mapa del calculador.
const HASH_TAB_SCRIPT: &str = r#"<script>
(function(){
  const m={'#calc':'pane-calc','#game':'pane-game','#opt':'pane-sim'};
  const pane=m[location.hash]; if(!pane) return;
  const btn=document.querySelector('.tab[data-pane="'+pane+'"]');
  if(btn) btn.click();
  if(location.hash==='#opt'){ const el=document.getElementById('o_run'); if(el) setTimeout(()=>el.scrollIntoView({behavior:'smooth',block:'center'}),80); }
})();
</script>"#;

/// Lógica del calculador de tiempo mínimo (fetch a /api/mintime).
const CALC_SCRIPT: &str = r#"<script>
(function(){
 const $=id=>document.getElementById(id);
 async function calc(){
   const o=$('mc_origin').value, d=$('mc_dest').value, dt=$('mc_dt').value;
   const series=[...document.querySelectorAll('.mc_serie:checked')].map(c=>c.value);
   if(o===d){ $('mc_result').innerHTML='<div class="card"><p class="muted">Seleccione un origen y un destino distintos.</p></div>'; return; }
   if(!series.length){ $('mc_result').innerHTML='<div class="card"><p class="muted">Seleccione al menos una serie.</p></div>'; return; }
   const b=$('mc_run'); b.disabled=true; const t=b.textContent; b.textContent='Calculando…';
   const p=new URLSearchParams(); p.set('origin',o); p.set('dest',d); p.set('dt',dt); p.set('series',series.join(',')); p.set('ltv',$('mc_ltv')&&$('mc_ltv').checked?'1':'0');
   try{ const r=await fetch('/api/mintime?'+p.toString()); $('mc_result').innerHTML=await r.text(); }
   catch(e){ $('mc_result').innerHTML='<div class="card">Error: '+e+'</div>'; }
   b.disabled=false; b.textContent=t;
 }
 const b=$('mc_run'); if(b) b.addEventListener('click',calc);
})();
</script>"#;

/// Mapa (Leaflet) + análisis de línea completa (Plotly) + modal. Todo cliente, sobre
/// los endpoints /api/stations, /api/lines y /api/line (cacheado en servidor y cliente).
const CALC2_SCRIPT: &str = r#"<script>
(function(){
 const $=id=>document.getElementById(id);
 const COL={'447':'#d2231b','450':'#1f6feb','470':'#1b7f3b','490':'#9a6700','456':'#5d6b78'};
 const fmt=s=>{s=Math.round(s);const m=Math.floor(s/60);return m+':'+String(s%60).padStart(2,'0');};
 let MAP=null,mapReady=false,STATIONS=[],STMARK={},ORIGIN=null,DEST=null,selLayer=null,lineLayer=null;
 let LINES=[],LINE_DATA=null,activeSeries=null,CHARTS={};

 // ---- Mapa ----
 function initMap(){
   if(mapReady){ if(MAP)MAP.invalidateSize(); return; }
   if(typeof L==='undefined'){ return; } mapReady=true;
   MAP=L.map('mc_map',{preferCanvas:true}).setView([41.55,2.05],9);
   L.tileLayer('https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png',{maxZoom:18,attribution:'© OpenStreetMap'}).addTo(MAP);
   fetch('/api/stations').then(r=>r.json()).then(list=>{ STATIONS=list;
     list.forEach(s=>{ const m=L.circleMarker([s.lat,s.lon],{radius:3,color:'#8b98a5',fillColor:'#8b98a5',weight:1,fillOpacity:.75});
       m.bindTooltip(s.name); m.on('click',()=>pickStation(s)); m.addTo(MAP); STMARK[s.id]=m; });
   }).catch(()=>{});
 }
 function pickStation(s){
   if(!ORIGIN || (ORIGIN&&DEST)){ ORIGIN=s; DEST=null; setSel('mc_origin',s.id); }
   else { DEST=s; setSel('mc_dest',s.id); }
   highlightSel();
 }
 function setSel(id,val){ const el=$(id); if([...el.options].some(o=>o.value===val)) el.value=val; }
 function highlightSel(){
   Object.values(STMARK).forEach(m=>m.setStyle({color:'#8b98a5',fillColor:'#8b98a5',radius:3}));
   if(selLayer){MAP.removeLayer(selLayer);selLayer=null;}
   const f=id=>STATIONS.find(s=>s.id===id);
   if(ORIGIN&&STMARK[ORIGIN.id])STMARK[ORIGIN.id].setStyle({color:'#3fb950',fillColor:'#3fb950',radius:6});
   if(DEST&&STMARK[DEST.id])STMARK[DEST.id].setStyle({color:'#e2231a',fillColor:'#e2231a',radius:6});
   if(ORIGIN&&DEST){const a=f(ORIGIN.id),b=f(DEST.id);if(a&&b)selLayer=L.polyline([[a.lat,a.lon],[b.lat,b.lon]],{color:'#f5a623',weight:3,dashArray:'6 6'}).addTo(MAP);}
 }
 function drawLineOnMap(){
   if(!MAP||!LINE_DATA)return; if(lineLayer){MAP.removeLayer(lineLayer);lineLayer=null;}
   const pts=LINE_DATA.stations.filter(s=>s.lat!=null&&s.lon!=null).map(s=>[s.lat,s.lon]);
   if(pts.length<2)return;
   lineLayer=L.polyline(pts,{color:'#f5a623',weight:4,opacity:.9}).addTo(MAP);
   MAP.fitBounds(lineLayer.getBounds(),{padding:[30,30]});
 }

 // ---- Poblar líneas/sentidos ----
 fetch('/api/lines').then(r=>r.json()).then(list=>{ LINES=list; const sel=$('ln_line'); if(!sel)return; sel.innerHTML='';
   list.forEach(l=>{const o=document.createElement('option');o.value=l.line;o.textContent=l.line;sel.appendChild(o);});
   fillDirs();
 }).catch(()=>{});
 function fillDirs(){ const l=LINES.find(x=>x.line===$('ln_line').value); const d=$('ln_dir'); if(!d)return; d.innerHTML='';
   if(!l)return; l.directions.forEach(dir=>{const o=document.createElement('option');o.value=dir.d0+'|'+dir.d1;o.textContent=dir.label+' ('+dir.n+' servicios)';d.appendChild(o);});
 }
 if($('ln_line'))$('ln_line').addEventListener('change',fillDirs);

 // ---- Calcular línea completa ----
 async function runLine(){
   const line=$('ln_line').value, dv=$('ln_dir').value.split('|');
   const series=[...document.querySelectorAll('.mc_serie:checked')].map(c=>c.value);
   if(!line||dv.length<2){return;}
   const dwell=$('ln_dwell').value, dws=$('ln_dwell_s').value, dt=$('mc_dt').value;
   const b=$('ln_run');b.disabled=true; const bar=$('ln_bar');bar.style.display='block';bar.firstElementChild.style.width='20%';
   $('ln_progress').textContent='Calculando todos los tramos de la línea…';
   const p=new URLSearchParams();p.set('line',line);p.set('d0',dv[0]);p.set('d1',dv[1]);p.set('series',series.join(','));p.set('dwell',dwell);if(dwell==='fixed')p.set('dwell_s',dws);p.set('dt',dt);p.set('ltv',$('ln_ltv')&&$('ln_ltv').checked?'1':'0');
   try{ const r=await fetch('/api/line?'+p.toString()); LINE_DATA=await r.json(); bar.firstElementChild.style.width='100%';
     if(LINE_DATA.error){ $('ln_out').innerHTML='<div class="card"><p class="muted">'+LINE_DATA.error+'</p></div>'; }
     else { activeSeries=null; renderLine(); drawLineOnMap(); $('ln_progress').textContent=LINE_DATA.n_segments+' tramos · '+LINE_DATA.n_stations+' estaciones'; }
   }catch(e){ $('ln_out').innerHTML='<div class="card">Error: '+e+'</div>'; }
   setTimeout(()=>{bar.style.display='none';bar.firstElementChild.style.width='0';},500); b.disabled=false;
 }
 if($('ln_run'))$('ln_run').addEventListener('click',runLine);

 // ---- Cálculos cliente ----
 function cumul(series){
   const st=LINE_DATA.stations,segs=LINE_DATA.segments; let mn=0,mr=0; let refok=LINE_DATA.adif_available;
   const o={km:[],min:[],ref:[],prog:[],margin:[]};
   for(let i=0;i<st.length;i++){
     if(i>0){const ss=segs[i-1].per_series[series];mn+=ss?ss.marcha_s:0; if(ss&&ss.marcha_ref_s!=null){mr+=ss.marcha_ref_s;}else{refok=false;}}
     o.km.push(st[i].cum_km); o.min.push(mn); o.ref.push(mr);
     const pg=st[i].programmed_cum_s; o.prog.push(pg==null?null:pg); o.margin.push(pg==null?null:(pg-mn));
     mn+=st[i].dwell_s; mr+=st[i].dwell_s;
   }
   o.refok=refok; return o;
 }

 // ---- Render ----
 function qdot(l){return '<span class="qdot q-'+l+'"></span>';}
 function card(sub,val,lab){return '<div class="kpi"><div class="kpi-sub">'+sub+'</div><div class="kpi-val">'+val+'</div><div class="kpi-label">'+lab+'</div></div>';}
 function chartCard(title,id){return '<div class="card"><button class="zoombtn" data-zoom="'+id+'">Ampliar</button><h2>'+title+'</h2><div id="'+id+'" class="plot"></div></div>';}

 function renderLine(){
   const d=LINE_DATA; if(!d||d.error)return;
   if(!activeSeries||!d.series.includes(activeSeries))activeSeries=d.series[0];
   const tot=d.totals[activeSeries]||{marcha_s:0,paradas_s:0,total_s:0,margin_median_s:0};
   const pr=d.programmed;
   const subtabs=d.series.map(s=>'<button class="subtab'+(s===activeSeries?' active':'')+'" data-serie="'+s+'" style="border-color:'+(s===activeSeries?COL[s]:'')+'">'+s+'</button>').join('');
   // Comparación
   let fastest=Math.min(...d.series.map(s=>d.totals[s].total_s));
   const refc = d.adif_available?'<th>Total (CVM ADIF)</th><th>Margen (CVM)</th>':'';
   let comp='<div class="scroll"><table><thead><tr><th>Tren</th><th>Marcha</th><th>Paradas</th><th>Total (Vmax tren)</th>'+refc+'<th>Programado (mediana)</th><th>Margen</th><th>Δ vs. rápido</th></tr></thead><tbody>';
   d.series.forEach(s=>{const t=d.totals[s];
     let rc='';
     if(d.adif_available){ rc = t.total_ref_s!=null ? '<td class="num mono" style="color:#3fb950">'+fmt(t.total_ref_s)+'</td><td class="num mono">'+(pr.n?(t.margin_ref_median_s>=0?'+':'−')+fmt(Math.abs(t.margin_ref_median_s)):'—')+'</td>' : '<td class="num muted">—</td><td class="num muted">—</td>'; }
     comp+='<tr><td class=mono><span class=dot style="background:'+COL[s]+'"></span>'+s+'</td><td class="num mono">'+fmt(t.marcha_s)+'</td><td class="num mono">'+fmt(t.paradas_s)+'</td><td class="num mono">'+fmt(t.total_s)+'</td>'+rc+'<td class="num mono">'+(pr.n?fmt(pr.median):'—')+'</td><td class="num mono">'+(pr.n?(t.margin_median_s>=0?'+':'')+fmt(Math.abs(t.margin_median_s)):'—')+'</td><td class="num mono">+'+fmt(t.total_s-fastest)+'</td></tr>';});
   d.unavailable.forEach(s=>{comp+='<tr class=muted><td class=mono>'+s+'</td><td colspan=6>datos insuficientes — no se calcula</td></tr>';});
   comp+='</tbody></table></div>';
   // Tabla por estaciones (serie activa)
   const cu=cumul(activeSeries); const st=d.stations,segs=d.segments;
   let str='<div class="scroll"><table><thead><tr><th>Estación</th><th>Dist. acum.</th><th>Marcha acum.</th><th>Parada</th><th>Total acum.</th><th>Programado</th><th>Δ</th></tr></thead><tbody>';
   for(let i=0;i<st.length;i++){const pg=st[i].programmed_cum_s;const diff=pg==null?null:(pg-cu.min[i]);
     str+='<tr><td>'+st[i].name+'</td><td class="num mono">'+st[i].cum_km.toFixed(2)+' km</td><td class="num mono">'+fmt(cu.min[i])+'</td><td class="num mono">'+(st[i].dwell_s?st[i].dwell_s+' s':'—')+'</td><td class="num mono">'+fmt(cu.min[i])+'</td><td class="num mono">'+(pg==null?'—':fmt(pg))+'</td><td class="num mono">'+(diff==null?'—':(diff>=0?'+':'')+fmt(Math.abs(diff)))+'</td></tr>';}
   str+='</tbody></table></div>';
   // Fichas
   let fich='<div class="scroll"><table><thead><tr><th>Tren</th><th>Vmax</th><th>Potencia</th><th>Masa</th><th>Acel.</th><th>Frenada</th></tr></thead><tbody>';
   d.fichas.forEach(f=>{ if(!f.available){fich+='<tr class=muted><td>'+f.id+'</td><td colspan=5>datos insuficientes (no se inventan)</td></tr>';return;}
     fich+='<tr><td class=mono>'+f.id+'</td><td>'+qdot(f.vmax_prov)+' '+f.vmax.toFixed(0)+' km/h</td><td>'+qdot(f.power_prov)+' '+f.power.toFixed(0)+' kW</td><td>'+qdot(f.mass_prov)+' '+f.mass.toFixed(1)+' t</td><td>'+qdot(f.accel_prov)+' '+f.accel.toFixed(2)+'</td><td>'+qdot(f.decel_prov)+' '+f.decel.toFixed(2)+'</td></tr>';});
   fich+='</tbody></table></div>';
   // Anomalías
   let an=d.anomalies.length?d.anomalies.map(a=>'<div class="anom">'+a+'</div>').join(''):'<p class="muted">No se ha detectado ninguna anomalía.</p>';
   // Calidad
   let ql=d.quality.map(q=>'<div class="qflag">'+qdot(q.level)+'<b>'+q.variable+'</b> — <span class="muted">'+q.note+'</span></div>').join('');
   // Fuentes
   let src='<div class="scroll"><table><thead><tr><th>Variable</th><th>Fuente</th><th>Método</th><th>Precisión</th></tr></thead><tbody>'+
     d.sources.map(s=>'<tr><td>'+qdot(s.level)+' '+s.variable+'</td><td>'+s.source+'<br><span class="muted">'+s.organismo+' · '+s.url+'</span></td><td class="muted">'+s.method+'</td><td class="muted">'+s.precision+'</td></tr>').join('')+'</tbody></table></div>';

   $('ln_out').innerHTML=
     '<div class="card"><h2>'+d.line+' · '+d.direction_label+'</h2>'+
       '<p class="muted">'+d.distance_km.toFixed(2)+' km · '+d.n_stations+' estaciones · dt '+d.dt+' s · paradas: '+d.dwell_mode+'</p>'+
       (d.adif_available?('<p class="muted"><span class="qdot q-oficial"></span> <b>CVM ADIF aplicada</b> · cobertura '+d.coverage_pct.toFixed(0)+'% · distancia ADIF '+(d.adif_distance_km!=null?d.adif_distance_km.toFixed(2)+' km':'—')+' (GTFS '+d.distance_km.toFixed(2)+' km)</p>'):'<p class="muted"><span class="qdot q-nd"></span> Sin CVM ADIF (ejecute scripts/fetch_adif_cvm.py).</p>')+
       (d.ltv_snapshot?('<p class="muted"><span class="qdot q-estimacion"></span> <b>LTV aplicadas</b> (instantánea '+d.ltv_snapshot+'): '+d.ltv_applied+' en el recorrido'+(d.min_ltv_kmh!=null?' · mín. '+d.min_ltv_kmh.toFixed(0)+' km/h':'')+' — temporal/fechado</p>'):'')+
       '<div class="subtabs">'+subtabs+'</div>'+
       '<div class="grid-kpi">'+card('Distancia',d.distance_km.toFixed(1)+' km','línea completa')+
         card('Tiempo mínimo ('+activeSeries+')',fmt(tot.total_s),'Vmax tren · marcha '+fmt(tot.marcha_s)+' + paradas '+fmt(tot.paradas_s))+
         (d.adif_available&&tot.total_ref_s!=null?card('Refinado CVM ('+activeSeries+')',fmt(tot.total_ref_s),'con velocidades ADIF reales'):'')+
         card('Programado',pr.n?fmt(pr.median):'—',pr.n?('mediana de '+pr.n+' servicios'):'sin datos')+
         card('Margen',pr.n?(((d.adif_available&&tot.margin_ref_median_s!=null?tot.margin_ref_median_s:tot.margin_median_s)>=0?'+':'−')+fmt(Math.abs(d.adif_available&&tot.margin_ref_median_s!=null?tot.margin_ref_median_s:tot.margin_median_s))):'—',d.adif_available?'programado − refinado':'programado − mínimo')+
       '</div>'+
       '<p class="muted src-note">'+d.observed_note+'</p>'+
       '<div style="margin-top:10px"><button class="zoombtn" style="float:none" id="ln_csv">Descargar CSV</button> <button class="zoombtn" style="float:none" id="ln_json">Descargar JSON</button></div>'+
     '</div>'+
     '<div class="card"><h2>Comparación de trenes</h2>'+comp+'</div>'+
     chartCard('Tiempo acumulado vs. distancia · mínimo ('+activeSeries+') vs. programado','ln_p_cum')+
     chartCard('Margen acumulado vs. distancia ('+activeSeries+')','ln_p_margin')+
     chartCard('Velocidad máxima alcanzada por tramo ('+activeSeries+')','ln_p_vmax')+
     '<div class="card"><h2>Análisis por estaciones ('+activeSeries+')</h2>'+str+'</div>'+
     '<div class="card"><h2>Comparar material</h2>'+fich+'<div id="ln_p_tot" class="plot" style="height:280px;margin-top:8px"></div></div>'+
     '<div class="card"><h2>Anomalías y consistencia</h2>'+an+'</div>'+
     '<div class="card"><h2>Calidad de los datos (por variable)</h2>'+ql+'</div>'+
     '<div class="card"><h2>Fuentes — de dónde procede cada dato</h2>'+src+'</div>';

   makeCharts(cu);
   // Wiring
   document.querySelectorAll('.subtab').forEach(b=>b.addEventListener('click',()=>{activeSeries=b.dataset.serie;renderLine();}));
   document.querySelectorAll('[data-zoom]').forEach(b=>b.addEventListener('click',()=>openModal(b.dataset.zoom)));
   $('ln_csv').addEventListener('click',exportCSV); $('ln_json').addEventListener('click',exportJSON);
 }

 function baseLayout(t){return {title:{text:t,font:{color:'#1b2127',size:13}},paper_bgcolor:'rgba(0,0,0,0)',plot_bgcolor:'rgba(0,0,0,0)',font:{color:'#5d6b78',size:11},margin:{l:58,r:14,t:36,b:42},xaxis:{gridcolor:'#e1e6eb',zeroline:false,title:'Distancia (km)'},yaxis:{gridcolor:'#e1e6eb',zeroline:false},legend:{orientation:'h'}};}
 function mk(id,data,layout){ if(typeof Plotly==='undefined')return; Plotly.newPlot(id,data,layout,{responsive:true,displaylogo:false}); CHARTS[id]={data,layout};}
 function makeCharts(cu){
   const d=LINE_DATA,c=COL[activeSeries];
   // acumulado
   let tr=[{x:cu.km,y:cu.min,name:'Mínimo (Vmax tren)',mode:'lines',line:{color:c,width:2}}];
   if(cu.refok)tr.push({x:cu.km,y:cu.ref,name:'Refinado (CVM ADIF)',mode:'lines',line:{color:'#1b7f3b',width:2}});
   if(d.programmed.n)tr.push({x:cu.km,y:cu.prog,name:'Programado',mode:'lines',line:{color:'#5d6b78',width:2,dash:'dot'}});
   let l1=baseLayout('');l1.yaxis.title='Tiempo acumulado (s)';mk('ln_p_cum',tr,l1);
   // margen
   let l2=baseLayout('');l2.yaxis.title='Margen (s)';mk('ln_p_margin',[{x:cu.km,y:cu.margin,mode:'lines',fill:'tozeroy',line:{color:'#d2231b',width:2},name:'Margen'}],l2);
   // vmax por tramo
   const xs=d.segments.map(s=>s.from+'→'+s.to),ys=d.segments.map(s=>{const ss=s.per_series[activeSeries];return ss?ss.vmax_reached_kmh:0;});
   let l3=baseLayout('');l3.xaxis.title='Tramo';l3.yaxis.title='Vmax alcanzada (km/h)';l3.xaxis.tickangle=-40;mk('ln_p_vmax',[{x:xs,y:ys,type:'bar',marker:{color:c}}],l3);
   // totales comparación
   const sx=d.series,sy=sx.map(s=>d.totals[s].total_s/60);
   let l4=baseLayout('');l4.xaxis.title='Serie';l4.yaxis.title='Tiempo total (min)';mk('ln_p_tot',[{x:sx,y:sy,type:'bar',marker:{color:sx.map(s=>COL[s])}}],l4);
 }

 // ---- Modal ----
 function openModal(id){ const m=$('mc_modal'); if(!CHARTS[id])return; m.classList.add('open');
   const lay=Object.assign({},CHARTS[id].layout); lay.autosize=true;
   Plotly.newPlot('mc_modal_plot',CHARTS[id].data,lay,{responsive:true,displaylogo:false});
 }
 if($('mc_modal_close'))$('mc_modal_close').addEventListener('click',()=>$('mc_modal').classList.remove('open'));

 // ---- Export ----
 function exportJSON(){ dl('linia_'+LINE_DATA.line+'.json',JSON.stringify(LINE_DATA,null,2),'application/json'); }
 function exportCSV(){ const d=LINE_DATA; let rows=[['linea','sentido','tren','tramo','distancia_km','tiempo_marcha_s','vmax_kmh','t_accel_s','t_crucero_s','t_frenada_s']];
   d.segments.forEach(seg=>{ d.series.forEach(s=>{ const ss=seg.per_series[s]; if(ss)rows.push([d.line,d.direction_label,s,seg.from+' → '+seg.to,seg.dist_km.toFixed(3),ss.marcha_s.toFixed(1),ss.vmax_reached_kmh.toFixed(1),ss.t_accel.toFixed(1),ss.t_cruise.toFixed(1),ss.t_brake.toFixed(1)]); }); });
   rows.push([]); rows.push(['tren','total_marcha_s','total_paradas_s','total_min_s','programado_mediana_s','margen_s']);
   d.series.forEach(s=>{const t=d.totals[s];rows.push([s,t.marcha_s.toFixed(1),t.paradas_s.toFixed(1),t.total_s.toFixed(1),d.programmed.median,t.margin_median_s.toFixed(1)]);});
   dl('linia_'+d.line+'.csv',rows.map(r=>r.join(',')).join('\n'),'text/csv');
 }
 function dl(name,txt,mime){ const b=new Blob([txt],{type:mime}); const a=document.createElement('a'); a.href=URL.createObjectURL(b); a.download=name; a.click(); URL.revokeObjectURL(a.href); }

 // ---- LTV: estat + recàrrega en calent ----
 function ltvStatus(){ const el=$('ltv_status'); if(!el)return; fetch('/api/ltv/status').then(r=>r.json()).then(j=>{
   el.innerHTML = j.available ? ('<span class="qdot q-estimacion"></span> LTV: '+j.count+' limitaciones · instantánea '+j.snapshot) : 'LTV: no cargadas (deje el ZIP diario en raw/ltv/ y recargue)';
 }).catch(()=>{}); }
 if($('ltv_reload'))$('ltv_reload').addEventListener('click',()=>{ const b=$('ltv_reload'); b.disabled=true; const t=b.textContent; b.textContent='Recargando…';
   fetch('/api/ltv/reload').then(r=>r.json()).then(()=>{ ltvStatus(); }).finally(()=>{ b.disabled=false; b.textContent=t; }); });
 ltvStatus();

 // ---- Init mapa al abrir la pestaña ----
 const calcTab=document.querySelector('.tab[data-pane="pane-calc"]');
 if(calcTab)calcTab.addEventListener('click',()=>setTimeout(initMap,60));
})();
</script>"#;

fn controls_html(lines: &[String], c: &Controls) -> String {
    let mut opts = String::from("<option value=\"\">Todas las líneas</option>");
    for l in lines {
        // La R3a se pliega sobre el R3 (tramo aislado en obras): no se lista como línea propia.
        if crate::topology::display_line(l) != l {
            continue;
        }
        let sel = if c.line.as_deref() == Some(l.as_str()) { " selected" } else { "" };
        opts.push_str(&format!("<option value=\"{0}\"{1}>{0}</option>", esc(l), sel));
    }
    let chk = if c.random { " checked" } else { "" };
    format!(
        r#"<div class="controls">
    <div class="controls-grid">
      <div class="field"><label>Hora inicio <b id="o_start">{start}</b>h</label>
        <input type="range" id="c_start" min="5" max="21" step="1" value="{start}" data-out="o_start"></div>
      <div class="field"><label>Duración <b id="o_dur">{dur}</b>h</label>
        <input type="range" id="c_dur" min="1" max="4" step="1" value="{dur}" data-out="o_dur"></div>
      <div class="field"><label>Línea</label>
        <select id="c_line">{opts}</select></div>
      <div class="field"><label>Bloqueo cantón <b id="o_block">{block}</b> min</label>
        <input type="range" id="c_block" min="0" max="20" step="1" value="{block}" data-out="o_block"></div>
      <div class="field"><label>Avería señal <b id="o_delay">{delay}</b> min</label>
        <input type="range" id="c_delay" min="0" max="15" step="1" value="{delay}" data-out="o_delay"></div>
      <div class="field"><label class="chk"><input type="checkbox" id="c_random"{chk}> Incidencias probabilísticas</label></div>
      <div class="field"><button class="btn" id="c_run">Simular</button></div>
    </div>
    <p class="muted" style="margin-top:8px;font-size:11px">Vías/andén, separación de bloque y demanda los DERIVA el motor unificado de datos reales (ADIF + perfil de demanda); no son ajustables.</p>
  </div>"#,
        start = c.start_h,
        dur = c.dur_h,
        opts = opts,
        block = c.block,
        delay = c.delay,
        chk = chk,
    )
}

/// Envoltorio de documento HTML completo (doctype + cabecera con charset/viewport + cuerpo).
/// `head` contiene el `<title>`, los estilos y, opcionalmente, enlaces a librerías de
/// visualización; `body` es el contenido de la página. Lengua declarada en español.
fn document(head: &str, body: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"es\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <meta name=\"color-scheme\" content=\"light dark\">\n{THEME_INIT}\n{head}\n</head>\n<body>\n{body}\n</body>\n</html>\n"
    )
}

/// Aplica el tema guardado ANTES de pintar el cuerpo (evita el parpadeo de pasar de claro a
/// oscuro). Lee `rod-theme` (light|dark|auto; por defecto auto = preferencia del sistema).
const THEME_INIT: &str = r#"<script>
(function(){try{
  var t=localStorage.getItem('rod-theme')||'auto';
  var dark=t==='dark'||(t==='auto'&&matchMedia('(prefers-color-scheme:dark)').matches);
  document.documentElement.setAttribute('data-theme',dark?'dark':'light');
}catch(e){}})();
</script>"#;

/// Modal de configuración (tema + fuente de horarios del juego). Compartido por el menú y el
/// panel; se acompaña de `SETTINGS_SCRIPT`, que lo cablea.
fn settings_modal() -> String {
    r#"<div class="settings" id="settingsModal" role="dialog" aria-modal="true" aria-label="Configuración">
    <div class="settings-inner">
      <button class="settings-close" id="settingsClose" aria-label="Cerrar">×</button>
      <h2>Configuración</h2>
      <div class="set-row">
        <label>Tema</label>
        <div class="seg" id="themeSeg" role="group" aria-label="Tema">
          <button data-theme-choice="light">Claro</button>
          <button data-theme-choice="dark">Oscuro</button>
          <button data-theme-choice="auto">Auto</button>
        </div>
      </div>
      <div class="set-row">
        <label>Horario al lanzar el simulador</label>
        <div class="seg" id="sourceSeg" role="group" aria-label="Fuente de horarios del juego">
          <button data-source-choice="gtfs">GTFS programado</button>
          <button data-source-choice="optimized">Malla optimizada</button>
        </div>
      </div>
      <p class="muted" style="margin:0">La malla optimizada requiere haber ejecutado antes el optimizador; si no existe, el simulador usa el horario programado.</p>
    </div>
  </div>"#.to_string()
}

/// Cableado del engranaje (⚙) y del modal de configuración: abre/cierra, cambia el tema en
/// caliente (persistido en `rod-theme`) y la fuente de horarios del juego (`rod-game-source`),
/// que decide a dónde apunta el botón "Lanzar simulador".
const SETTINGS_SCRIPT: &str = r#"<script>
(function(){
 const $=id=>document.getElementById(id);
 const modal=$('settingsModal'); if(!modal) return;
 const open=()=>modal.classList.add('open'), close=()=>modal.classList.remove('open');
 const gear=$('settingsBtn'); if(gear) gear.addEventListener('click',open);
 const x=$('settingsClose'); if(x) x.addEventListener('click',close);
 modal.addEventListener('click',e=>{ if(e.target===modal) close(); });
 document.addEventListener('keydown',e=>{ if(e.key==='Escape') close(); });

 // Tema
 function applyTheme(t){
   const dark=t==='dark'||(t==='auto'&&matchMedia('(prefers-color-scheme:dark)').matches);
   document.documentElement.setAttribute('data-theme',dark?'dark':'light');
   document.querySelectorAll('#themeSeg button').forEach(b=>b.classList.toggle('active',b.dataset.themeChoice===t));
 }
 const curTheme=localStorage.getItem('rod-theme')||'auto';
 applyTheme(curTheme);
 document.querySelectorAll('#themeSeg button').forEach(b=>b.addEventListener('click',()=>{
   const t=b.dataset.themeChoice; localStorage.setItem('rod-theme',t); applyTheme(t);
 }));
 // Si el tema es "auto", seguir los cambios del sistema en vivo.
 matchMedia('(prefers-color-scheme:dark)').addEventListener('change',()=>{
   if((localStorage.getItem('rod-theme')||'auto')==='auto') applyTheme('auto');
 });

 // Fuente de horarios del juego → actualiza el enlace "Lanzar simulador".
 function applySource(s){
   document.querySelectorAll('#sourceSeg button').forEach(b=>b.classList.toggle('active',b.dataset.sourceChoice===s));
   const play=$('playCard'); if(play) play.setAttribute('href', s==='optimized' ? '/game?source=optimized' : '/game');
 }
 const curSrc=localStorage.getItem('rod-game-source')||'gtfs';
 applySource(curSrc);
 document.querySelectorAll('#sourceSeg button').forEach(b=>b.addEventListener('click',()=>{
   const s=b.dataset.sourceChoice; localStorage.setItem('rod-game-source',s); applySource(s);
 }));
})();
</script>"#;

/// Página estática (offline): estilos + cabecera + cuerpo, sin controles.
pub fn render_html(
    summary: &SummaryView,
    example: &Option<ExampleView>,
    sim: &SimView,
    res: &ResView,
    generated_at: &str,
) -> String {
    let body = render_body(summary, example, sim, res, generated_at);
    let head = format!("<title>Rodalies Simulator · panel de análisis</title>\n{STYLE}");
    let page = format!("<div class=\"wrap\">\n  {HEADER}\n{body}\n</div>");
    document(&head, &page)
}

/// Menú principal (`/`): el juego es la pieza central (hero), con lanzadores del calculador y
/// del optimizador y un botón de configuración (tema + fuente de horarios del juego). El juego
/// se abre en `/game`; el calculador y el optimizador viven en el panel (`/panel#calc`,
/// `/panel#opt`), que conserva las pestañas de análisis.
pub fn render_menu_page() -> String {
    let head = format!("<title>Rodalies Simulator</title>\n{STYLE}");
    let body = format!(
        r#"<div class="menu-wrap">
    <div class="menu-top">
      <div class="menu-brand">
        <div class="logo">R</div>
        <div>
          <h1>Rodalies Simulator <span style="font-size:11px;opacity:.5;font-weight:400">v2.0</span></h1>
          <div class="sub">Simulador de tráfico ferroviario · red de Rodalies de Catalunya · modelo construido a partir de datos GTFS</div>
        </div>
      </div>
      <button class="gear" id="settingsBtn" title="Configuración" aria-label="Configuración">
        <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>
      </button>
    </div>

    <div class="hero">
      <a class="hero-card" id="playCard" href="/game">
        <span class="hero-badge">Modo visual</span>
        <h2>Simulador de red (visual)</h2>
        <p>Toda la red de Rodalies sobre el mapa satélite, <b>reproduciendo el motor unificado</b>: los trenes aceleran y frenan de verdad (Vmax de vía CVM/ADIF + LTV + física del material real), con señalización de bloque, vía única e incidencias. La pieza central.</p>
        <span class="hero-cta">Lanzar modo visual →</span>
      </a>
    </div>

    <div class="tiles">
      <a class="tile" href="/info">
        <div class="tile-ic">ℹ️</div>
        <h3>Información</h3>
        <p>Cómo se calculan los tiempos entre estaciones (física del material + Vmax de vía + LTV) y todas las probabilidades del modelo de incidencias (dónde y cuándo aparecen).</p>
      </a>
      <a class="tile" href="/numeros">
        <div class="tile-ic">⚡</div>
        <h3>Varias runs (solo números)</h3>
        <p>Corre el motor headless muchas veces (varias semillas) sin dibujar nada y compara solo las métricas: retraso, retenidos, PAX·min e incidencias del día. Mismo motor que el visual.</p>
      </a>
      <a class="tile" href="/panel#calc">
        <div class="tile-ic">📐</div>
        <h3>Calcular tramos</h3>
        <p>Tiempo mínimo entre estaciones con la física real del material (datos ADIF · LTV): marchas, paradas y márgenes frente al horario programado.</p>
      </a>
      <a class="tile" href="/panel#opt">
        <div class="tile-ic">🧮</div>
        <h3>Optimizador de horarios</h3>
        <p>Recocido simulado sobre los desfases de fase de cada línea; minimiza el potencial V(H) del sistema completo y exporta la malla optimizada.</p>
      </a>
    </div>

    <p class="menu-more">También disponible: <a href="/panel">Panel de análisis completo →</a></p>
  </div>
  {modal}
  {SETTINGS_SCRIPT}"#,
        modal = settings_modal(),
    );
    document(&head, &body)
}

/// Página de INFORMACIÓN: cómo se calculan los tiempos entre estaciones (física) y todas las
/// probabilidades del modelo de incidencias. Documenta el modelo real del motor unificado.
pub fn render_info_page() -> String {
    let head = format!("<title>Rodalies Simulator · Información</title>\n{STYLE}");
    let body = r#"<style>
.info-card{max-width:840px;margin:14px auto;text-align:left;line-height:1.55}
.info-card h2{margin:.2em 0 .5em;font-size:18px}
.info-card pre{background:rgba(255,255,255,.06);padding:8px 10px;border-radius:8px;overflow-x:auto;font-size:12.5px;white-space:pre-wrap}
.info-card ul{margin:.3em 0 .6em 1.1em}
.info-card li{margin:.2em 0}
</style>
<div class="menu-wrap">
    <div class="panel-bar"><a class="back-link" href="/">← Menú</a></div>
    <div class="menu-top"><div class="menu-brand"><div class="logo">R</div><div>
      <h1>Información del modelo</h1>
      <div class="sub">Cómo el motor unificado calcula los tiempos entre estaciones y las probabilidades de las incidencias.</div>
    </div></div></div>

    <section class="card info-card">
      <h2>⏱️ Tiempos entre estaciones — por FÍSICA, no por horario</h2>
      <p>El motor <b>no</b> aplica un tiempo fijo por tramo: integra el movimiento tren a tren, cada fracción de segundo. La velocidad en cada punto es</p>
      <pre>v_permitida = mín( Vmax del material , Vmax de vía (CVM/ADIF) , LTV , frenada anticipada )</pre>
      <ul>
        <li><b>Vmax del material</b> — serie real asignada a cada línea:
          <ul>
            <li><b>R2 / R2N / R2S → serie 450</b> (doble piso, 140 km/h)</li>
            <li><b>Regionales (no R11 ni R3) → serie 470</b> (140 km/h)</li>
            <li><b>R1 / R3 / R4 → serie 465</b> (Civia, 120 km/h)</li>
            <li><b>Resto (R7, R8, R11, R12…) → serie 447</b> (120 km/h)</li>
          </ul>
        </li>
        <li><b>Vmax de vía</b> — velocidad de diseño oficial de ADIF (CVM/RFIG) proyectada a cada cantón.</li>
        <li><b>LTV</b> — limitaciones temporales de velocidad de ADIF, superpuestas donde aplican.</li>
        <li><b>Frenada anticipada</b> — el tren empieza a frenar con antelación para parar en la estación o ante una señal en rojo.</li>
      </ul>
      <p><b>Conducción:</b> el maquinista acelera a <b>¾ de la aceleración máxima</b> (no todos van a tope) y siempre intenta ir a la máxima permitida. Ante una señal en <b>ámbar</b> reduce un <b>20%</b> para poder detenerse si la siguiente sigue en rojo.</p>
      <p><b>El horario es un OBJETIVO, no una imposición de velocidad:</b> un tren <b>nunca sale de una estación antes de su hora programada</b> de salida, y respeta la <b>parada mínima</b> (20–60 s según la densidad de servicio de la estación). Si la física o la congestión lo retrasan, el retraso se propaga — como en la realidad.</p>
    </section>

    <section class="card info-card">
      <h2>⚠️ Incidencias — probabilidades del modelo</h2>
      <p>Toda incidencia es de <b>dos tipos</b>: una <b>señal en rojo</b> sin tren delante (en un sentido o en ambos), o un <b>tramo de vía doble que pasa a vía única</b>. Cada plantilla que creas tiene un <b>motivo</b>, un <b>efecto</b>, una <b>duración</b> y una <b>probabilidad diaria</b>.</p>
      <p><b>Antes de empezar el día</b>, el motor "tira los dados" (reproducible con la <b>semilla</b>): para cada plantilla activa saca un número y, con su probabilidad diaria, decide si aparece. No hay máximo de incidencias.</p>
      <p>Si aparece, se decide <b>dónde</b> y <b>cuándo</b> con distribuciones iguales para todos los tipos:</p>
      <ul>
        <li><b>Dónde (distribución espacial):</b> más probable en <b>nudos (Barcelona)</b>, <b>estaciones grandes</b>, <b>tramos más frecuentados</b> y <b>terminales</b>. Peso por cantón:
          <pre>peso = (1 + vías_norm) · (1 + frecuencia_norm) · (1 + terminal) · (1 + nudo_BCN)</pre>
          donde <i>vías</i> = nº de vías de andén, <i>frecuencia</i> = nº de servicios que pasan, <i>terminal</i> = extremo de línea, <i>nudo_BCN</i> = estación de Barcelona.</li>
        <li><b>Cuándo (distribución temporal):</b> más probable en <b>hora punta</b> que en horas valle o primera/última hora (sigue el perfil de demanda del día).</li>
      </ul>
      <p>El creador de incidencias (plantillas eliminables y activables) está en el panel <b>INCID.</b> del modo visual y se comparte con el modo de números.</p>
    </section>
  </div>"#;
    document(&head, body)
}

/// Página del modo "VARIAS RUNS (solo números)": corre el motor headless para varias semillas y
/// compara las métricas. Otra ventana/modo aparte del visual (mismo motor).
pub fn render_numbers_page() -> String {
    let head = format!("<title>Rodalies Simulator · Varias runs (números)</title>\n{STYLE}");
    let body = r##"<style>
.info-card{max-width:920px;margin:14px auto;text-align:left;box-sizing:border-box}
.num-tbl{width:100%;border-collapse:collapse;margin-top:12px;font-size:13px;table-layout:fixed}
.num-tbl th,.num-tbl td{border-bottom:1px solid rgba(255,255,255,.1);padding:6px 8px;text-align:right;vertical-align:top;overflow-wrap:anywhere;word-break:break-word}
.num-tbl th{opacity:.7;font-weight:600}
.num-tbl td:last-child,.num-tbl th:last-child{text-align:left;width:40%}
.info-card label{font-size:13px;display:inline-flex;flex-direction:column;gap:3px;opacity:.85}
.info-card input{background:rgba(255,255,255,.08);border:1px solid rgba(255,255,255,.15);border-radius:7px;padding:5px 7px;color:inherit;font:inherit}
.inc-cell{line-height:1.35}
.inc-line{padding:3px 0;border-bottom:1px dashed rgba(255,255,255,.08)}
.inc-line:last-child{border-bottom:none}
.inc-line .eff{opacity:.65;font-size:11px}
.inc-line .when{opacity:.7;font-variant-numeric:tabular-nums}
.num-tbl .muted{opacity:.5}
</style>
<div class="menu-wrap">
    <div class="panel-bar"><a class="back-link" href="/">← Menú</a></div>
    <div class="menu-top"><div class="menu-brand"><div class="logo">R</div><div>
      <h1>Varias runs · solo motor y números</h1>
      <div class="sub">Corre el motor unificado headless (física + incidencias) para un rango de semillas y compara métricas. Sin dibujar nada — el mismo motor que el modo visual.</div>
    </div></div></div>

    <section class="card info-card">
      <div style="display:flex;gap:12px;align-items:end;flex-wrap:wrap">
        <label>Semillas (desde) <input type="number" id="s0" value="1" min="0" style="width:90px"></label>
        <label>Semillas (hasta) <input type="number" id="s1" value="8" min="0" style="width:90px"></label>
        <button class="hero-cta" id="runBtn" style="cursor:pointer;border:none">▶ Ejecutar runs</button>
        <span id="prog" style="opacity:.7"></span>
      </div>
      <p class="sub" style="margin-top:6px">Cada run simula la jornada completa (~961 trenes); tarda unos segundos. Se ejecutan una a una.</p>
      <table class="num-tbl" id="tbl">
        <thead><tr><th>Semilla</th><th>Trenes</th><th>Retraso máx (min)</th><th>Pico retenidos</th><th>Final retenidos</th><th>PAX·min</th><th>Incidencias</th></tr></thead>
        <tbody></tbody>
      </table>
    </section>
  </div>
  <script>
  (function(){
    const $=id=>document.getElementById(id);
    const EFF={red_one:"roja 1 sentido",red_both:"roja 2 sentidos",single:"vía única"};
    $("runBtn").onclick=async()=>{
      const a=+$("s0").value||0, b=+$("s1").value||0;
      const tb=$("tbl").querySelector("tbody"); tb.innerHTML="";
      $("runBtn").disabled=true;
      for(let s=a;s<=b;s++){
        $("prog").textContent="Ejecutando semilla "+s+"…";
        try{
          const r=await (await fetch("/api/simulate?seed="+s)).json();
          const esc=x=>String(x).replace(/&/g,"&amp;").replace(/</g,"&lt;").replace(/>/g,"&gt;");
          // Log de incidencias: DÓNDE (nombres de estación) y CUÁNDO (HH:MM–HH:MM), una por línea.
          const inc=(r.incidents||[]).length
            ? (r.incidents).map(i=>`<div class="inc-line"><b>${esc(i.motivo)}</b> <span class="eff">${EFF[i.effect]||i.effect}</span><br><span class="where">${esc(i.from_name||i.from)} → ${esc(i.to_name||i.to)}</span> · <span class="when">${i.t0_hm||Math.floor(i.t0/3600)+"h"}–${i.t1_hm||""}</span></div>`).join("")
            : '<span class="muted">— sin incidencias hoy</span>';
          const tr=document.createElement("tr");
          tr.innerHTML=`<td>${s}</td><td>${r.n_trains}</td><td>${r.max_delay_min.toFixed(0)}</td><td>${r.peak_waiting}</td><td>${r.final_waiting}</td><td>${Math.round(r.pax_min).toLocaleString()}</td><td class="inc-cell">${inc}</td>`;
          tb.appendChild(tr);
        }catch(e){
          const tr=document.createElement("tr"); tr.innerHTML=`<td>${s}</td><td colspan="6">error</td>`; tb.appendChild(tr);
        }
      }
      $("prog").textContent="Hecho.";
      $("runBtn").disabled=false;
    };
  })();
  </script>"##;
    document(&head, body)
}

/// Página interactiva (servida por el servidor web): cabecera + controles +
/// contenedor `#dashboard` con el cuerpo inicial + JS que hace fetch a `/api/render`.
pub fn render_interactive_page(
    summary: &SummaryView,
    example: &Option<ExampleView>,
    sim: &SimView,
    res: &ResView,
    lines: &[String],
    controls: &Controls,
    generated_at: &str,
    calc_panel: &str,
) -> String {
    let body = render_body(summary, example, sim, res, generated_at);
    let ctrls = controls_html(lines, controls);
    let optp = optimizer_panel();
    let head = format!("<title>Rodalies Simulator · panel interactivo</title>\n{STYLE}");
    let page = format!(
        "<div class=\"wrap\">\n  \
         <div class=\"panel-bar\">\
           <a class=\"back-link\" href=\"/\">← Menú</a>\
           <button class=\"gear\" id=\"settingsBtn\" title=\"Configuración\" aria-label=\"Configuración\">\
             <svg viewBox=\"0 0 24 24\" width=\"20\" height=\"20\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><circle cx=\"12\" cy=\"12\" r=\"3\"/><path d=\"M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z\"/></svg>\
           </button>\
         </div>\n  {HEADER}\n  \
         <div class=\"tabs\">\
           <button class=\"tab active\" data-pane=\"pane-sim\">Simulador · Optimizador</button>\
           <button class=\"tab\" data-pane=\"pane-calc\">Calculador de tiempo mínimo</button>\
           <button class=\"tab\" data-pane=\"pane-game\">Simulador de red (juego)</button>\
         </div>\n  \
         <div id=\"pane-sim\" class=\"tabpane active\">\n  {ctrls}\n  {optp}\n  <div id=\"dashboard\">\n{body}\n  </div>\n  </div>\n  \
         <div id=\"pane-calc\" class=\"tabpane\">\n  {calc_panel}\n  </div>\n  \
         <div id=\"pane-game\" class=\"tabpane\">\n  {game_panel}\n  </div>\n\
         </div>\n  {modal}\n{SCRIPT}\n{OPT_SCRIPT}\n{TAB_SCRIPT}\n{CALC_SCRIPT}\n{CALC2_SCRIPT}\n{SETTINGS_SCRIPT}\n{HASH_TAB_SCRIPT}",
        game_panel = game_panel(),
        modal = settings_modal(),
    );
    document(&head, &page)
}

/// Cuerpo del dashboard (secciones), sin `<title>`, estilos ni cabecera.
pub fn render_body(
    summary: &SummaryView,
    example: &Option<ExampleView>,
    sim: &SimView,
    res: &ResView,
    generated_at: &str,
) -> String {
    // --- KPIs ---
    let kpis = [
        ("Vías / andenes", format!("{}", summary.n_stops), "nodos del grafo"),
        ("Cantones", format!("{}", summary.n_edges), "secciones de vía"),
        ("Servicios de tren", format!("{}", summary.n_services), "circulaciones GTFS"),
        ("Líneas", format!("{}", summary.n_routes), "R1, R2N, R4…"),
        ("Tiempo de carga", format!("{:.0} ms", summary.load_ms), "GTFS → grafo"),
    ];
    let mut kpi_html = String::new();
    for (label, value, sub) in kpis {
        kpi_html.push_str(&format!(
            "<div class=\"kpi\"><div class=\"kpi-val\">{}</div><div class=\"kpi-label\">{}</div><div class=\"kpi-sub\">{}</div></div>",
            esc(&value), esc(label), esc(sub)
        ));
    }

    // --- Barras de serveis per línia ---
    let max_line = summary.per_line.iter().map(|(_, c)| *c).max().unwrap_or(1).max(1);
    let mut lines_html = String::new();
    for (name, cnt) in &summary.per_line {
        let pct = (*cnt as f64 / max_line as f64) * 100.0;
        lines_html.push_str(&format!(
            "<div class=\"bar-row\"><span class=\"bar-name\">{}</span><span class=\"bar-track\"><span class=\"bar-fill\" style=\"width:{:.1}%\"></span></span><span class=\"bar-val\">{}</span></div>",
            esc(name), pct, cnt
        ));
    }
    let fastest = summary
        .fastest_edge
        .as_ref()
        .map(|(a, b, s)| format!("{} → {} ({}s)", esc(a), esc(b), s))
        .unwrap_or_else(|| "—".into());
    lines_html.push_str(&format!(
        "<p class=\"muted\" style=\"margin-top:14px\">Andenes con <code>parent_station</code>: {} · cantón más rápido: {}</p>",
        summary.with_parent, fastest
    ));

    // --- Ruta d'exemple ---
    let example_html = match example {
        None => "<p class=\"muted\">No hay servicios que mostrar.</p>".to_string(),
        Some(ex) => {
            let title = if ex.found {
                format!("Ruta del tren {}", esc(&ex.wanted))
            } else {
                format!(
                    "Tren {} no encontrado → servicio equivalente {} ({})",
                    esc(&ex.wanted), esc(&ex.train_number), esc(&ex.route_short)
                )
            };
            let mut rows = String::new();
            for s in &ex.stops {
                rows.push_str(&format!(
                    "<tr><td class=\"num\">{}</td><td>{}</td><td class=\"mono\">{}</td><td class=\"mono\">{}</td><td class=\"mono\">{}</td><td class=\"num\"><span class=\"pill\">{}</span></td></tr>",
                    s.seq, esc(&s.name), fmt_hms(s.arr), fmt_hms(s.dep), run_txt(s.run_secs), s.track
                ));
            }
            format!(
                "<h3>{title}</h3><p class=\"muted\">Línea {} · route_id {} · trip_id {} · {} paradas{}</p>\
                 <div class=\"scroll\"><table><thead><tr><th>seq</th><th>Estación</th><th>llegada</th><th>salida</th><th>marcha</th><th>vía</th></tr></thead><tbody>{rows}</tbody></table></div>",
                esc(&ex.route_short), esc(&ex.route_id), esc(&ex.trip_id), ex.stops.len(),
                if ex.headsign.is_empty() { String::new() } else { format!(" · {}", esc(&ex.headsign)) },
            )
        }
    };

    // --- Incidències ---
    let mut inc_html = String::new();
    for i in &sim.incidents {
        inc_html.push_str(&format!("<li>{}</li>", esc(i)));
    }
    if inc_html.is_empty() {
        inc_html = "<li class=\"muted\">Ninguna incidencia inyectada.</li>".into();
    }

    // --- Mètriques (del motor unificat) ---
    let recovery = sim.recovery.clone().unwrap_or_else(|| "no alcanzado".into());
    let metrics = [
        ("Trenes simulados", format!("{}", sim.trains_run)),
        ("Retenciones (conflictos)", format!("{}", sim.held)),
        ("Pico de retraso acumulado", format!("{} s", sim.peak_total)),
        ("Hora del pico", sim.peak_time.clone()),
        ("Retorno al equilibrio", recovery),
    ];
    let mut metrics_html = String::new();
    for (l, v) in metrics {
        metrics_html.push_str(&format!(
            "<div class=\"metric\"><span class=\"metric-label\">{}</span><span class=\"metric-val\">{}</span></div>",
            esc(l), esc(&v)
        ));
    }

    // --- Taula de resiliència ---
    let max_peak = res.rows.iter().map(|r| r.peak).max().unwrap_or(1).max(1) as f64;
    let mut res_rows = String::new();
    for r in &res.rows {
        let recov = match r.recovery {
            Some(m) => format!("{} min", m),
            None => "no recupera".into(), // (etiqueta en español; "no recupera" es idéntico en cat./es.)
        };
        let pct = (r.peak as f64 / max_peak) * 100.0;
        res_rows.push_str(&format!(
            "<tr><td class=\"mono\">+{} min</td><td class=\"num\">{}</td><td><span class=\"bar-track sm\"><span class=\"bar-fill\" style=\"width:{:.1}%\"></span></span></td><td>{}</td><td class=\"num\">{}</td></tr>",
            r.mins, r.peak, pct, esc(&recov), r.held
        ));
    }

    let busiest = sim
        .busiest
        .as_ref()
        .map(|(a, b, c)| format!("{} → {} ({} circulaciones)", esc(a), esc(b), c))
        .unwrap_or_else(|| "—".into());

    let chart = timeline_svg(&sim.timeline);
    let key_stations = sim
        .key_stations
        .iter()
        .map(|s| format!("<span class=\"chip\">{}</span>", esc(s)))
        .collect::<Vec<_>>()
        .join("");

    format!(
        r##"<div class="grid-kpi">{kpi_html}</div>

  <div class="cols">
    <section class="card">
      <h2>Servicios por línea</h2>
      {lines_html}
    </section>
    <section class="card">
      <h2>Simulación · ventana {window}</h2>
      <p class="muted">service_id dominante <b>{service_id}</b> · cantón más cargado: {busiest}</p>
      <div class="metrics">{metrics_html}</div>
      <div class="chips">{key_stations}</div>
      <h3 style="margin-top:18px">Incidencias inyectadas</h3>
      <ul class="inc">{inc_html}</ul>
    </section>
  </div>

  <section class="card">
    <h2>Mapa de la red · trenes en circulación</h2>
    <p class="muted">Estaciones y cantones proyectados a partir de la latitud/longitud del GTFS; los puntos de color son trenes desplazándose por su ruta real (bucle de la ventana simulada).</p>
    {map_svg}
  </section>

  <section class="card">
    <h2>Retraso acumulado de la red</h2>
    {chart}
  </section>

  <section class="card">
    <h2>Análisis de resiliencia</h2>
    <p class="muted">Escenario: bloqueo del cantón {segment}. Cada fila es una duración de bloqueo (Monte Carlo, rayon · motor unificado).</p>
    <div class="scroll"><table>
      <thead><tr><th>bloqueo</th><th>pico acumulado</th><th></th><th>recuperación</th><th>conflictos</th></tr></thead>
      <tbody>{res_rows}</tbody>
    </table></div>
  </section>

  <section class="card">
    <h2>Ruta de ejemplo</h2>
    {example_html}
  </section>

  <footer>Generado el {generated_at} · Rodalies Simulator · datos GTFS Rodalies/Cercanías</footer>
"##,
        kpi_html = kpi_html,
        lines_html = lines_html,
        window = esc(&sim.window),
        service_id = esc(&sim.service_id),
        busiest = busiest,
        metrics_html = metrics_html,
        key_stations = key_stations,
        inc_html = inc_html,
        map_svg = sim.map_svg,
        chart = chart,
        segment = esc(&res.segment),
        res_rows = res_rows,
        example_html = example_html,
        generated_at = esc(generated_at),
    )
}
