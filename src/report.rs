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

const STYLE: &str = r#"<link rel="preconnect" href="https://fonts.googleapis.com"><link rel="preconnect" href="https://fonts.gstatic.com" crossorigin><link href="https://fonts.googleapis.com/css2?family=Montserrat:wght@600;700;800&family=Inter:wght@400;500;600;700&family=Roboto+Mono:wght@400;500;700&display=swap" rel="stylesheet">
<style>
/* Paleta Rodalies de Catalunya / Renfe: blancos y grises de la librea de los
   trenes, con el rojo corporativo de Rodalies como único acento, usado con
   mesura. Aspecto instrumental/científico: superficies planas, sin degradados. */
:root {
  /* ANDANA — identidad Rodalies: naranja (#EC6E15) como acento primario y morado
     Renfe (#830065) como secundario menor (tags/datos). Se conservan TODOS los
     nombres de variable (los usan JS/Rust); solo cambian los valores. */
  --bg:#f9f5f1; --panel:#ffffff; --panel2:#f6efe7; --border:#e6dccd;
  --text:#26272d; --muted:#6c6a72; --ink:#26272d;
  --accent:#ec6e15; --accent-soft:rgba(236,110,21,.10); --accent2:#830065;
  --accent2-soft:rgba(131,0,101,.08);
  --arr:#1b7f3b; --dep:#1f6feb; --late:#c5221f; --warn:#b25a00;
  --grid:#ece3d8;
  /* ANDANA — tipografía display (títulos/números de línea) + gradiente CTA naranja
     + lavado naranja de fondo (claro). */
  --display:'Montserrat','Segoe UI',system-ui,sans-serif;
  --grad-cta:linear-gradient(180deg, #FF8A3D 0%, #EC6E15 55%, #C8590E 100%);
  --wash:radial-gradient(120% 90% at 50% -15%, rgba(236,110,21,0.08), transparent 60%), radial-gradient(90% 70% at 100% 0%, rgba(255,138,61,0.05), transparent 55%);
}
* { box-sizing:border-box; }
/* `hidden` debe ganar a .rod-load{display:flex} (si no, el loader del tren no se ocultaría). */
[hidden] { display:none !important; }
body { margin:0; background:var(--bg); background-image:var(--wash); color:var(--text);
  font-family:'Inter',system-ui,-apple-system,'Segoe UI',sans-serif; line-height:1.5;
  -webkit-font-smoothing:antialiased; }
.mono { font-family:'Roboto Mono',ui-monospace,'Cascadia Code',Consolas,monospace; }
.muted { color:var(--muted); font-size:.9em; }
code { background:var(--panel2); padding:1px 5px; border-radius:4px; font-size:.85em; border:1px solid var(--border); }
.wrap { max-width:1200px; margin:0 auto; padding:24px 20px 60px; }
header.top { display:flex; align-items:center; gap:16px; padding:8px 0 20px; border-bottom:2px solid var(--ink); margin-bottom:24px; }
/* Logo pixel-art de Rodalies (mismo sprite que el juego), ampliado con bordes duros. */
.logo { width:46px; height:46px; border-radius:9px; flex:0 0 auto; display:block; object-fit:cover;
  image-rendering:pixelated; image-rendering:crisp-edges;
  box-shadow:0 3px 12px rgba(236,110,21,.38); }
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
.btn { background:var(--grad-cta); color:#fff; border:none; border-radius:4px;
  padding:11px 22px; font-size:.95rem; font-weight:600; cursor:pointer;
  text-decoration:none; display:inline-block; }
.btn:hover { filter:brightness(1.06); }
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
/* Panel sin pestañas: las secciones se apilan; separa el calculador del simulador. */
#pane-calc.tabpane.active { margin-top:30px; padding-top:24px; border-top:2px solid var(--border); }
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
.modal-inner { background:var(--panel); border:1px solid var(--border); border-top:3px solid var(--accent); border-radius:6px; width:100%; height:100%; padding:16px; position:relative; }
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
  --bg:#14161b; --panel:#1c1e25; --panel2:#252730; --border:#363843;
  --text:#f2ede7; --muted:#a09ba4; --ink:#f2ede7;
  --accent:#ff8a3d; --accent-soft:rgba(255,138,61,.15); --accent2:#d072b8;
  --accent2-soft:rgba(208,114,184,.14);
  --arr:#3fbf6a; --dep:#5aa0ff; --late:#ff6b68; --warn:#e0a03a;
  --grid:#2f313a;
  /* Lavado naranja + oscurecido inferior para el fondo oscuro. */
  --wash:radial-gradient(120% 80% at 50% -10%, rgba(236,110,21,0.16), transparent 60%), radial-gradient(120% 80% at 50% 110%, rgba(11,13,18,0.55), transparent 55%);
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
  text-transform:uppercase; color:#fff; background:var(--grad-cta); padding:3px 10px; border-radius:20px; margin-bottom:14px; }
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
.settings-inner { background:var(--panel); border:1px solid var(--border); border-top:3px solid var(--accent); border-radius:10px;
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
.seg button.active { background:var(--grad-cta); color:#fff; }

/* ── ANDANA · tipografía display + iconos SVG ─────────────────────────── */
/* Montserrat (var --display) en títulos y números; se declara al final para
   ganar en orden a reglas previas (p. ej. .logo con su font-family propia). */
h1, h2, h3, .logo, .hero-card h2, .tile h3, .hero-badge, .kpi-val, .menu-brand h1 { font-family:var(--display); }
.tile-ic { color:var(--accent); }
.tile-ic svg { width:26px; height:26px; color:var(--accent); }
.ic-h { display:inline-flex; align-items:center; vertical-align:-4px; }
.ic-h svg { width:20px; height:20px; color:var(--accent); }
#runBtn svg { width:15px; height:15px; vertical-align:-2px; margin-right:3px; }
/* Banda/loader del tren pixel en flujo (menú y /numeros): fondo transparente,
   se adapta a ambos temas. */
.rod-band { background:transparent !important; padding:12px 0 4px; gap:10px; }
.rod-band .rod-load-caption { color:var(--muted); font-size:12px; letter-spacing:.14em; }
.rod-band .rod-load-dots i { background:var(--accent); }

/* ════════════════════════════════════════════════════════════════════════
   ANDANA · Tren pixel-art de càrrega (Rodalies) — 100% autocontingut, sense
   imatges externes. Sprite = SVG inline blocky (crispEdges), taronja temable
   amb var(--rod-orange). Rodes = <g> independents que giren. Pista de traves
   amb gradients "hard-stop" que es desplacen (moviment relatiu cap a la dreta).
   Rendiment: només transforms/opacity + background-position (GPU-friendly).
   ════════════════════════════════════════════════════════════════════════ */

/* ── Contenidor / escena ─────────────────────────────────────────────── */
.rod-load{
  /* Tokens de marca (temables des de fora; fallback = taronja Rodalies) */
  --rod-orange:      #EC6E15;
  --rod-orange-lift: #FF8A3D;
  --rod-orange-deep: #C8590E;
  --rod-purple:      #830065;   /* accent secundari, ús mínim */
  --rod-white:       #F9F5F1;   /* blanc càlid */
  --rod-carbon:      #0B0D12;
  --rod-window:      #0B0D12;
  --rod-steel:       #6b7078;
  /* Pista (sobreescrit per cada skin) */
  --rod-rail:        #4a4d55;
  --rod-tie:         #2a2d34;
  --rod-track-speed: .62s;      /* sincronitzat amb el gir de roda */

  display:flex; flex-direction:column; align-items:center; justify-content:center;
  gap:22px; padding:32px 24px;
  font-family:'Inter', system-ui, -apple-system, 'Segoe UI', sans-serif;
  -webkit-user-select:none; user-select:none;
  transition:opacity .45s ease;
}
/* Modificador d'overlay: el contenidor superior del joc ocupa tota la pantalla */
#rodLoad{ position:fixed; inset:0; z-index:9999; }

/* Fade-out en acabar (JS afegeix .is-done abans d'amagar) */
.rod-load.is-done{ opacity:0; pointer-events:none; }

/* ── Escena del tren (bob + xup-xup horitzontal) ─────────────────────── */
.rod-load-scene{ display:flex; flex-direction:column; align-items:center; }
.rod-train-scene{
  position:relative; width:264px; height:96px;
  display:flex; align-items:flex-end; justify-content:center;
  animation:rod-train-bob .55s ease-in-out infinite;
  will-change:transform;
}
/* Ombra tova que "respira" a contratemps del bot */
.rod-train-scene::after{
  content:""; position:absolute; left:50%; bottom:2px; translate:-50% 0;
  width:210px; height:12px; border-radius:50%;
  background:radial-gradient(ellipse at center, rgba(0,0,0,.42), rgba(0,0,0,0) 70%);
  animation:rod-shadow-breathe .55s ease-in-out infinite;
  z-index:0;
}

/* ── Sprite SVG: crisp/pixel ─────────────────────────────────────────── */
.rod-train{
  position:relative; z-index:1;
  width:264px; height:auto;
  shape-rendering:crispEdges;      /* vores dures = look pixel */
  image-rendering:pixelated;
  image-rendering:-moz-crisp-edges;
  filter:drop-shadow(0 2px 0 rgba(0,0,0,.18));
}
/* Farciments (temables via CSS custom props) */
.rod-train-hull   rect{ fill:var(--rod-orange, #EC6E15); }
.rod-train-lift   rect{ fill:var(--rod-orange-lift, #FF8A3D); }  /* reflex de sostre */
.rod-train-shade  rect{ fill:var(--rod-orange-deep, #C8590E); }  /* ombra inferior */
.rod-train-stripe      { fill:var(--rod-white, #F9F5F1); }       /* franja blanca */
.rod-train-glass  rect{ fill:var(--rod-window, #0B0D12); }       /* finestres/portes */
.rod-train-lamp        { fill:#FFD9A0; }                          /* far càlid */
.rod-train-frame  rect{ fill:#0f1013; }                           /* baix + bogies */

/* Rodes: cada <g> gira sobre el seu propi centre (fill-box) */
.rod-train-wheel{
  transform-box:fill-box; transform-origin:center;
  animation:rod-wheel-spin var(--rod-track-speed) linear infinite;
  will-change:transform;
}
.rod-train-wheel .tyre { fill:#17181C; }
.rod-train-wheel .hub  { fill:var(--rod-steel, #6b7078); }
.rod-train-wheel .spoke{ stroke:var(--rod-steel, #6b7078); stroke-width:1; }
.rod-train-wheel-set{ shape-rendering:geometricPrecision; } /* rodes rodones netes */

/* ── Bafarades opcionals (toc lúdic; s'amaguen amb .no-steam) ─────────── */
.rod-load-puff{
  position:absolute; left:26px; bottom:56px;
  width:6px; height:6px; border-radius:1px;
  background:var(--rod-white, #F9F5F1);
  image-rendering:pixelated; opacity:0; z-index:2;
  animation:rod-puff-rise 2.2s ease-out infinite;
}
.rod-load-puff:nth-child(2){ animation-delay:.73s; }
.rod-load-puff:nth-child(3){ animation-delay:1.46s; }
.rod-load.no-steam .rod-load-puff{ display:none; }

/* ── Via/pista de traves (desplaçament = sensació d'avanç) ────────────── */
.rod-load-track{
  width:280px; height:14px; margin-top:-7px;   /* s'encaixa sota les rodes */
  border-radius:1px;
  image-rendering:pixelated;
  background-image:
    repeating-linear-gradient(90deg, var(--rod-tie) 0 6px, transparent 6px 18px), /* traves */
    linear-gradient(var(--rod-rail) 0 0);                                          /* carril */
  background-size:18px 14px, 100% 3px;
  background-position:0 9px, 0 1px;
  background-repeat:repeat-x, repeat-x;
  animation:rod-track-scroll var(--rod-track-speed) linear infinite;
}

/* ── Peu: llegenda + tres punts de càrrega ───────────────────────────── */
.rod-load-caption{
  font-family:'Montserrat', 'Inter', system-ui, sans-serif;
  font-weight:800; font-size:15px; letter-spacing:.16em; text-transform:uppercase;
  display:inline-flex; align-items:baseline;
}
.rod-load-dots{ display:inline-flex; margin-left:8px; }
.rod-load-dots i{
  width:6px; height:6px; margin-left:5px; display:inline-block;
  background:var(--rod-orange, #EC6E15); border-radius:1px; image-rendering:pixelated;
  animation:rod-dot 1.05s ease-in-out infinite;
}
.rod-load-dots i:nth-child(2){ animation-delay:.18s; }
.rod-load-dots i:nth-child(3){ animation-delay:.36s; }

/* ── Skins de context ────────────────────────────────────────────────── */
/* Joc: overlay carbó gairebé opac */
.rod-load.on-dark{
  background:
    radial-gradient(120% 90% at 50% 40%, rgba(24,27,35,.94), rgba(11,13,18,.985));
  color:var(--rod-white, #F9F5F1);
  --rod-rail:#565a63; --rod-tie:#23262e;
}
.rod-load.on-dark .rod-load-caption{ color:var(--rod-white, #F9F5F1); }
/* Informes: paper blanc càlid */
.rod-load.on-light{
  background:var(--rod-white, #F9F5F1);
  color:#17181C;
  --rod-window:#14161c;
  --rod-rail:#b7afa5; --rod-tie:#d9cfc3;
}
.rod-load.on-light .rod-load-caption{ color:#17181C; }
.rod-load.on-light .rod-train{ filter:drop-shadow(0 2px 0 rgba(120,90,60,.22)); }

/* ── Keyframes (tot transform/opacity/background-position) ────────────── */
@keyframes rod-train-bob{
  0%   { transform:translate3d(-1px, 0, 0); }
  25%  { transform:translate3d( 0, -3px, 0); }
  50%  { transform:translate3d( 1px, 0, 0); }
  75%  { transform:translate3d( 0, -2px, 0); }
  100% { transform:translate3d(-1px, 0, 0); }
}
@keyframes rod-shadow-breathe{
  0%,50%,100%{ transform:scaleX(1);   opacity:.85; }
  25%,75%    { transform:scaleX(.9);  opacity:.55; }
}
@keyframes rod-wheel-spin{ to{ transform:rotate(360deg); } }
@keyframes rod-track-scroll{ to{ background-position:-18px 9px, 0 1px; } }
@keyframes rod-puff-rise{
  0%  { transform:translate(0,0) scale(.7);      opacity:0;   }
  18% { opacity:.6; }
  100%{ transform:translate(-16px,-26px) scale(2.1); opacity:0; }
}
@keyframes rod-dot{
  0%,70%,100%{ opacity:.25; transform:translateY(0); }
  35%        { opacity:1;   transform:translateY(-4px); }
}

/* Accessibilitat: respecta qui demana menys moviment */
@media (prefers-reduced-motion:reduce){
  .rod-train-scene, .rod-train-scene::after, .rod-train-wheel,
  .rod-load-track, .rod-load-puff, .rod-load-dots i{ animation:none !important; }
}
</style>"#;

const HEADER: &str = r#"<header class="top">
    <img class="logo" src="/game/lines/r_logo.png" alt="Rodalies" width="46" height="46">
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

/// Enlaces profundos desde el menú principal: `/panel#calc` y `/panel#opt` hacen scroll a la
/// sección correspondiente (ya no hay pestañas: ambas secciones se muestran apiladas).
const HASH_TAB_SCRIPT: &str = r#"<script>
(function(){
  const m={'#calc':'pane-calc','#opt':'o_run'};
  const id=m[location.hash]; if(!id) return;
  const el=document.getElementById(id);
  if(el) setTimeout(()=>el.scrollIntoView({behavior:'smooth',block:location.hash==='#opt'?'center':'start'}),120);
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
 // Sin pestañas: el mapa del calculador se inicializa al cargar (su sección ya es visible).
 setTimeout(initMap,300);
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
         <meta name=\"color-scheme\" content=\"light dark\">\n{THEME_INIT}\n{I18N_HEAD}\n{head}\n</head>\n<body>\n{body}\n</body>\n</html>\n"
    )
}

/// Aplica el tema guardado ANTES de pintar el cuerpo (evita el parpadeo de pasar de claro a
/// oscuro). Lee `rod-theme` (light|dark|auto; por defecto auto = preferencia del sistema).
/// Capa de idiomas (castellano · català · English): diccionario + motor que traduce el DOM. Va
/// en TODAS las páginas (y en el juego) para que el idioma elegido en Configuración se respete en
/// toda la app. En castellano no hace nada.
const I18N_HEAD: &str = r#"<script src="/game/i18n_dict.js"></script><script src="/game/i18n.js"></script>"#;

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
        <label>Idioma</label>
        <div class="seg" id="langSeg" role="group" aria-label="Idioma" data-no-i18n>
          <button data-lang-choice="es">Castellano</button>
          <button data-lang-choice="ca">Català</button>
          <button data-lang-choice="en">English</button>
        </div>
      </div>
      <p class="muted" style="margin:0">Los horarios (GTFS guardado / nuevo / importado) se eligen en la pantalla previa al lanzar el simulador visual.</p>
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
 // Idioma (castellano · català · English): se guarda en `rod-lang` y vale para toda la app.
 const curLang=(window.I18N&&I18N.lang)||'es';
 document.querySelectorAll('#langSeg button').forEach(b=>{
   b.classList.toggle('active',b.dataset.langChoice===curLang);
   b.addEventListener('click',()=>{ if(window.I18N&&b.dataset.langChoice!==curLang) I18N.set(b.dataset.langChoice); });
 });
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

/// Iconos SVG de línea (ANDANA) que sustituyen a los emojis de la interfaz. Usan `currentColor`
/// para heredar el color del contenedor (normalmente el naranja de marca). Fuente: assets ANDANA.
fn icon(name: &str) -> &'static str {
    match name {
        "info" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><line x1="12" y1="11" x2="12" y2="16"/><path d="M12 7.6h.01"/></svg>"#,
        "runs" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M13 2 4.5 13.2a.6.6 0 0 0 .48.96H11l-1 7.84 8.5-11.2a.6.6 0 0 0-.48-.96H12z"/></svg>"#,
        "ruler" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="7" width="20" height="10" rx="1.5"/><line x1="6" y1="7" x2="6" y2="12"/><line x1="10" y1="7" x2="10" y2="11"/><line x1="14" y1="7" x2="14" y2="12"/><line x1="18" y1="7" x2="18" y2="11"/></svg>"#,
        "optimize" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="4" y1="21" x2="4" y2="15"/><line x1="4" y1="11" x2="4" y2="3"/><line x1="12" y1="21" x2="12" y2="13"/><line x1="12" y1="9" x2="12" y2="3"/><line x1="20" y1="21" x2="20" y2="17"/><line x1="20" y1="13" x2="20" y2="3"/><line x1="1.5" y1="13" x2="6.5" y2="13"/><line x1="9.5" y1="9" x2="14.5" y2="9"/><line x1="17.5" y1="15" x2="22.5" y2="15"/></svg>"#,
        "timer" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="13" r="8"/><line x1="9.5" y1="2" x2="14.5" y2="2"/><line x1="12" y1="2" x2="12" y2="5"/><line x1="12" y1="13" x2="12" y2="9.5"/><line x1="18.5" y1="6.5" x2="20" y2="5"/></svg>"#,
        "warning" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10.3 3.6 1.9 18a2 2 0 0 0 1.7 3h16.8a2 2 0 0 0 1.7-3L13.7 3.6a2 2 0 0 0-3.4 0Z"/><line x1="12" y1="9" x2="12" y2="13.5"/><path d="M12 17h.01"/></svg>"#,
        "play" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M7 4.5v15a.6.6 0 0 0 .92.5l12-7.5a.6.6 0 0 0 0-1L7.92 4a.6.6 0 0 0-.92.5Z"/></svg>"#,
        "refresh" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 11.5a8 8 0 1 0-.8 4.5"/><polyline points="20 4 20 11.5 12.5 11.5"/></svg>"#,
        "back-arrow" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="20" y1="12" x2="5" y2="12"/><polyline points="11 6 5 12 11 18"/></svg>"#,
        "download" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="12" y1="3" x2="12" y2="15"/><polyline points="7 10 12 15 17 10"/><path d="M4 20h16"/></svg>"#,
        "chart" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 3v18h18"/><path d="M6 15c3-9 5-9 7-4s4 4 8-6"/></svg>"#,
        _ => "",
    }
}

/// Escena reutilizable del tren pixel (ANDANA): sprite SVG crisp + vía de traves. La usan la banda
/// decorativa del menú y el loader de "varias runs". El contenedor (`.rod-load …`) y el caption los
/// pone quien la incruste. Sin recursos externos (el color naranja lo dan las custom props del CSS).
const TRAIN_SCENE: &str = r#"<div class="rod-load-scene">
        <div class="rod-train-scene">
          <svg class="rod-train" viewBox="0 0 128 44" width="264" height="91" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Tren de Rodalies">
            <g class="rod-train-lift"><rect x="14" y="7" width="86" height="3"/></g>
            <g class="rod-train-hull">
              <rect x="8"  y="14" width="98" height="14"/>
              <rect x="12" y="10" width="92" height="4"/>
              <rect x="6"  y="17" width="2"  height="9"/>
              <rect x="106" y="12" width="4" height="16"/>
              <rect x="110" y="14" width="4" height="14"/>
              <rect x="114" y="17" width="4" height="11"/>
              <rect x="118" y="21" width="3" height="7"/>
            </g>
            <g class="rod-train-shade">
              <rect x="8"   y="26" width="98" height="2"/>
              <rect x="106" y="26" width="4"  height="2"/>
              <rect x="110" y="26" width="4"  height="2"/>
            </g>
            <rect class="rod-train-stripe" x="8" y="22" width="98" height="3"/>
            <g class="rod-train-glass">
              <rect x="14" y="15" width="7" height="6"/>
              <rect x="24" y="15" width="7" height="6"/>
              <rect x="34" y="15" width="7" height="6"/>
              <rect x="44" y="15" width="7" height="10"/>
              <rect x="54" y="15" width="7" height="6"/>
              <rect x="64" y="15" width="7" height="6"/>
              <rect x="74" y="15" width="7" height="6"/>
              <rect x="84" y="15" width="7" height="10"/>
              <rect x="94" y="15" width="7" height="6"/>
              <rect x="103" y="14" width="7" height="6"/>
              <rect x="110" y="16" width="4" height="4"/>
            </g>
            <rect class="rod-train-lamp" x="116" y="24" width="3" height="3"/>
            <g class="rod-train-frame">
              <rect x="10" y="28" width="98" height="2"/>
              <rect x="16" y="29" width="30" height="4"/>
              <rect x="72" y="29" width="34" height="4"/>
            </g>
            <g class="rod-train-wheel-set">
              <g class="rod-train-wheel">
                <circle class="tyre" cx="22" cy="33" r="5"/>
                <circle class="hub"  cx="22" cy="33" r="1.6"/>
                <line class="spoke" x1="22" y1="28.5" x2="22" y2="37.5"/>
                <line class="spoke" x1="17.5" y1="33" x2="26.5" y2="33"/>
              </g>
              <g class="rod-train-wheel">
                <circle class="tyre" cx="40" cy="33" r="5"/>
                <circle class="hub"  cx="40" cy="33" r="1.6"/>
                <line class="spoke" x1="40" y1="28.5" x2="40" y2="37.5"/>
                <line class="spoke" x1="35.5" y1="33" x2="44.5" y2="33"/>
              </g>
              <g class="rod-train-wheel">
                <circle class="tyre" cx="82" cy="33" r="5"/>
                <circle class="hub"  cx="82" cy="33" r="1.6"/>
                <line class="spoke" x1="82" y1="28.5" x2="82" y2="37.5"/>
                <line class="spoke" x1="77.5" y1="33" x2="86.5" y2="33"/>
              </g>
              <g class="rod-train-wheel">
                <circle class="tyre" cx="100" cy="33" r="5"/>
                <circle class="hub"  cx="100" cy="33" r="1.6"/>
                <line class="spoke" x1="100" y1="28.5" x2="100" y2="37.5"/>
                <line class="spoke" x1="95.5" y1="33" x2="104.5" y2="33"/>
              </g>
            </g>
          </svg>
        </div>
        <div class="rod-load-track" aria-hidden="true"></div>
      </div>"#;

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
        <img class="logo" src="/game/lines/r_logo.png" alt="Rodalies" width="46" height="46">
        <div>
          <h1>Rodalies Simulator <span style="font-size:11px;opacity:.5;font-weight:400">v2.0</span></h1>
          <div class="sub">Simulador de tráfico ferroviario · red de Rodalies de Catalunya · modelo construido a partir de datos GTFS</div>
        </div>
      </div>
      <button class="gear" id="settingsBtn" title="Configuración" aria-label="Configuración">
        <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>
      </button>
    </div>

    <!-- ANDANA · banda decorativa: tren pixel corriendo, ENCIMA del simulador visual. -->
    <div class="rod-load rod-band no-steam" role="img" aria-label="Rodalies en marcha">
      {TRAIN_SCENE}
      <div class="rod-load-caption">La red en marcha<span class="rod-load-dots" aria-hidden="true"><i></i><i></i><i></i></span></div>
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
      <a class="tile" href="/incidencias">
        <div class="tile-ic">🚨</div>
        <h3>Incidencias</h3>
        <p>Crea, edita y borra las plantillas de incidencia (señal roja o vía única) con su probabilidad <b>cada X horas</b>. Las usan el modo visual y las runs.</p>
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
    // Sustituye los emojis de las tiles por iconos SVG de línea (ANDANA).
    let body = body
        .replace("ℹ️", icon("info"))
        .replace("⚡", icon("runs"))
        .replace("📐", icon("ruler"))
        .replace("🧮", icon("optimize"))
        .replace("🚨", icon("warning"));
    document(&head, &body)
}

/// Página de INFORMACIÓN: cómo se calculan los tiempos entre estaciones (física) y todas las
/// probabilidades del modelo de incidencias. Documenta el modelo real del motor unificado.
pub fn render_info_page() -> String {
    let head = format!("<title>Rodalies Simulator · Información</title>\n{STYLE}");
    let body = r#"<style>
.info-card{max-width:840px;margin:14px auto;text-align:left;line-height:1.55}
.info-card h2{margin:.2em 0 .5em;font-size:18px;display:flex;align-items:center;gap:9px}
.info-card h2 svg{width:21px;height:21px;flex:0 0 auto;color:var(--accent)}
.info-card pre{background:var(--panel2);border:1px solid var(--border);padding:8px 10px;border-radius:8px;overflow-x:auto;font-size:12.5px;white-space:pre-wrap}
.info-card ul{margin:.3em 0 .6em 1.1em}
.info-card li{margin:.2em 0}
</style>
<div class="menu-wrap">
    <div class="panel-bar"><a class="back-link" href="/">← Menú</a></div>
    <div class="menu-top"><div class="menu-brand"><img class="logo" src="/game/lines/r_logo.png" alt="Rodalies" width="46" height="46"><div>
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
      <p>Toda incidencia es de <b>dos tipos</b>: una <b>señal en rojo</b> sin tren delante (en un sentido o en ambos), o un <b>tramo de vía doble que pasa a vía única</b>. Cada plantilla que creas (apartado <a href="/incidencias">Incidencias</a> del menú) tiene un <b>motivo</b>, un <b>efecto</b>, una <b>duración</b> y una <b>probabilidad p cada X horas</b>.</p>
      <p><b>Antes de empezar el día</b>, el motor "tira los dados" (reproducible con la <b>semilla</b>): la jornada (de la salida del primer tren a las 24:00) se trocea en un <b>número entero de secciones iguales</b>, el más cercano a <i>horas de jornada / X</i> (mínimo 1), y, para cada plantilla activa, en <b>cada sección</b> se saca un número: con probabilidad <b>p</b> aparece una instancia dentro de esa sección. Así una misma incidencia puede ocurrir <b>varias veces</b> en un día; de media, <b>p × nº de secciones</b> veces. "Cada 24 h" = una sola tirada al día (la antigua probabilidad diaria). No hay máximo de incidencias.</p>
      <p>Si aparece, se decide <b>dónde</b> y <b>cuándo</b> con distribuciones iguales para todos los tipos:</p>
      <ul>
        <li><b>Dónde (distribución espacial):</b> más probable en <b>nudos (Barcelona)</b>, <b>estaciones grandes</b>, <b>tramos más frecuentados</b> y <b>terminales</b>. Peso por cantón:
          <pre>peso = (1 + vías_norm) · (1 + frecuencia_norm) · (1 + terminal) · (1 + nudo_BCN)</pre>
          donde <i>vías</i> = nº de vías de andén, <i>frecuencia</i> = nº de servicios que pasan, <i>terminal</i> = extremo de línea, <i>nudo_BCN</i> = estación de Barcelona.</li>
        <li><b>Cuándo (distribución temporal):</b> más probable en <b>hora punta</b> que en horas valle o primera/última hora (sigue el perfil de demanda del día).</li>
      </ul>
      <p>El creador de incidencias (crear, editar y borrar plantillas) es el apartado <a href="/incidencias">Incidencias</a> del menú. Las plantillas se comparten; cada modo (visual / varias runs) elige por su cuenta cuáles entran en juego. Además, en el modo visual se puede <b>lanzar una incidencia AHORA</b> (panel <b>INCID.</b>) en el tramo que elijas.</p>
    </section>
  </div>"#;
    // Sustituye los emojis de las cabeceras por iconos SVG de línea (ANDANA). Se cubre la forma con
    // selector de variación (️, U+FE0F) y la base por si acaso.
    let body = body
        .replace("⏱️", icon("timer")).replace("⏱", icon("timer"))
        .replace("⚠️", icon("warning")).replace("⚠", icon("warning"));
    document(&head, &body)
}

/// Página del modo "VARIAS RUNS (solo números)": corre el motor headless para varias semillas y
/// compara las métricas. Otra ventana/modo aparte del visual (mismo motor).
pub fn render_numbers_page() -> String {
    let head = format!("<title>Rodalies Simulator · Varias runs (números)</title>\n{STYLE}");
    let body = r##"<style>
/* Esta página necesita ancho para la tabla de runs (la columna de incidencias es la clave). */
.menu-wrap{max-width:1320px}
.num-layout{display:grid;grid-template-columns:330px minmax(0,1fr);gap:20px;align-items:start;max-width:1320px;margin:14px auto}
@media(max-width:840px){.num-layout{grid-template-columns:1fr}}
.num-side h2{margin:0 0 4px}
.num-side .sub{margin:0 0 14px}
.num-seeds{display:flex;gap:10px;margin-bottom:14px}
.fld{display:flex;flex-direction:column;gap:4px;font-size:.74rem;font-weight:700;letter-spacing:.04em;text-transform:uppercase;color:var(--muted)}
.num-seeds .fld{flex:1;min-width:0}
.fld input{width:100%;min-width:0;background:var(--panel2);border:1px solid var(--border);border-radius:7px;padding:8px 9px;color:var(--text);font:inherit;font-weight:400;text-transform:none;letter-spacing:0}
.gtfs-box{border:1px solid var(--border);border-radius:9px;padding:11px 12px;margin-bottom:14px;background:var(--panel2)}
.gtfs-box .lbl{font-size:.72rem;font-weight:700;letter-spacing:.05em;text-transform:uppercase;color:var(--muted);margin-bottom:6px}
.gtfs-box b{color:var(--text)}
.sched-opts{display:flex;flex-direction:column;gap:6px}
.sched-opt{display:flex;gap:9px;align-items:flex-start;padding:8px 10px;border:1px solid var(--border);border-radius:8px;background:var(--panel);cursor:pointer;transition:.14s}
.sched-opt:hover{border-color:var(--accent)}
.sched-opt:has(input:checked){border-color:var(--accent);box-shadow:inset 0 0 0 1px var(--accent);background:var(--accent-soft)}
.sched-opt input{margin-top:3px;accent-color:var(--accent)}
.sched-opt>span{display:flex;flex-direction:column;line-height:1.3;font-size:.86rem}
.sched-opt small{color:var(--muted);font-size:.74rem}
.sched-opt small svg{width:12px;height:12px;vertical-align:-2px}
.file-pick{position:relative;display:block;margin-top:8px}
.file-pick input{position:absolute;inset:0;opacity:0;cursor:pointer}
.file-pick span{display:block;text-align:center;font-size:.8rem;color:var(--muted);border:1px dashed var(--border);border-radius:8px;padding:8px}
.file-pick:hover span{border-color:var(--accent);color:var(--text)}
.fmt-help{position:relative;display:inline-flex;align-items:center;justify-content:center;width:16px;height:16px;margin-left:5px;border-radius:50%;font-size:10px;font-weight:700;color:#fff;background:var(--accent);cursor:help;vertical-align:1px}
.fmt-pop{position:absolute;left:-12px;top:calc(100% + 8px);width:320px;max-width:80vw;z-index:20;padding:10px 12px;border-radius:9px;font-size:.74rem;font-weight:400;line-height:1.5;color:var(--text);background:var(--panel);border:1px solid var(--border);box-shadow:0 12px 32px rgba(0,0,0,.18);opacity:0;visibility:hidden;transition:opacity .15s;text-transform:none;letter-spacing:0}
.fmt-help:hover .fmt-pop,.fmt-help:focus .fmt-pop{opacity:1;visibility:visible}
.tpl-title{font-size:.72rem;font-weight:700;letter-spacing:.05em;text-transform:uppercase;color:var(--muted);margin:2px 0 6px}
.num-tpls{list-style:none;padding:0;margin:0;display:flex;flex-direction:column;gap:6px}
.num-tpls li{display:flex;gap:9px;align-items:flex-start;font-size:.86rem;background:var(--panel2);border:1px solid var(--border);border-left:3px solid var(--accent);border-radius:7px;padding:8px 10px}
.num-tpls li.off{border-left-color:var(--border);opacity:.55}
.num-tpls li small{display:block;color:var(--muted);font-size:.72rem;margin-top:2px}
.num-tpls input{margin-top:2px;width:15px;height:15px;accent-color:var(--accent);flex:0 0 auto;cursor:pointer}
.num-tpls .empty{border-left-color:transparent;color:var(--muted);font-style:italic;background:none}
/* Barra de acciones ENCIMA de los resultados: ejecutar + progreso + descargar. */
.res-bar{display:flex;align-items:center;gap:12px;flex-wrap:wrap;margin-bottom:16px;padding-bottom:14px;border-bottom:1px solid var(--border)}
#runBtn{display:inline-flex;align-items:center;gap:8px;font-size:1rem;padding:11px 22px;border-radius:8px}
#runBtn svg,.dl-menu svg{width:15px;height:15px;flex:0 0 auto}
#prog{color:var(--muted);font-size:.85rem}
.btn.ghost{background:var(--panel2);color:var(--text);border:1px solid var(--border)}
.btn.ghost:hover:not(:disabled){border-color:var(--accent)}
.dl-menu{position:relative;margin-left:auto}
#dlBtn{display:inline-flex;align-items:center;gap:8px;border-radius:8px;padding:10px 16px}
.dl-pop{position:absolute;right:0;top:calc(100% + 6px);z-index:30;min-width:230px;background:var(--panel);border:1px solid var(--border);border-radius:10px;box-shadow:0 14px 34px rgba(0,0,0,.16);padding:6px}
.dl-grp{font-size:.68rem;font-weight:700;letter-spacing:.06em;text-transform:uppercase;color:var(--muted);padding:8px 10px 4px}
.dl-pop button{display:block;width:100%;text-align:left;background:none;border:none;border-radius:7px;padding:8px 10px;font:inherit;font-size:.88rem;color:var(--text);cursor:pointer}
.dl-pop button:hover{background:var(--accent-soft);color:var(--accent)}
.all-toggle{display:flex;align-items:center;gap:9px;font-size:.84rem;font-weight:600;padding:7px 10px;margin-bottom:6px;border:1px dashed var(--border);border-radius:7px;cursor:pointer}
.all-toggle input{width:15px;height:15px;accent-color:var(--accent);cursor:pointer}
.num-tbl{width:100%;border-collapse:collapse;font-size:13px}
.num-tbl th,.num-tbl td{border-bottom:1px solid var(--border);padding:8px 6px;text-align:right;vertical-align:top;white-space:nowrap}
.num-tbl th{color:var(--muted);font-weight:700;font-size:.7rem;text-transform:uppercase;letter-spacing:.03em;border-bottom:2px solid var(--border);white-space:normal;vertical-align:bottom;line-height:1.25;max-width:92px}
.num-tbl td:first-child,.num-tbl th:first-child{text-align:left}
.num-tbl td{vertical-align:middle}
.num-tbl th.act,.num-tbl td.act{text-align:left}
/* Acciones por run: una fila = una línea (las incidencias y los gráficos se abren en un modal). */
.row-btn{display:inline-flex;align-items:center;gap:6px;font:600 .78rem var(--sans);padding:6px 10px;border-radius:7px;border:1px solid var(--border);background:var(--panel2);color:var(--text);cursor:pointer;white-space:nowrap;transition:.14s}
.row-btn:hover:not(:disabled){border-color:var(--accent);color:var(--accent)}
.row-btn:disabled{opacity:.45;cursor:default}
.row-btn svg{width:14px;height:14px;flex:0 0 auto}
.row-acts{display:flex;flex-direction:column;gap:5px;align-items:stretch}
a.row-btn{text-decoration:none}
.row-btn .cnt{font-family:var(--mono);background:var(--accent-soft);color:var(--accent);border-radius:5px;padding:0 5px}
.num-tbl .muted{color:var(--muted)}
#numLoad{margin:14px auto;max-width:440px}
</style>
<div class="menu-wrap">
    <div class="panel-bar"><a class="back-link" href="/">← Menú</a></div>
    <div class="menu-top"><div class="menu-brand"><img class="logo" src="/game/lines/r_logo.png" alt="Rodalies" width="46" height="46"><div>
      <h1>Varias runs · solo motor y números</h1>
      <div class="sub">Corre el motor unificado headless (física + incidencias) para un rango de semillas y compara métricas. Mismo motor que el modo visual.</div>
    </div></div></div>

    <div class="num-layout">
      <aside class="card num-side">
        <h2>Escenario</h2>
        <p class="sub">Elige el rango de semillas y qué incidencias entran en juego, y ejecuta.</p>
        <div class="num-seeds">
          <label class="fld">Semilla desde<input type="number" id="s0" value="1" min="0"></label>
          <label class="fld">Semilla hasta<input type="number" id="s1" value="8" min="0"></label>
        </div>
        <div class="gtfs-box">
          <div class="lbl">Horarios</div>
          <div class="sched-opts" id="schedOpts">
            <label class="sched-opt"><input type="radio" name="sched" value="saved" checked><span><b>GTFS guardado</b><small id="gtfsSavedInfo">Comprobando…</small></span></label>
            <label class="sched-opt"><input type="radio" name="sched" value="new"><span><b>GTFS nuevo</b><small>__DL__ Descarga el feed de hoy (Renfe) antes de correr</small></span></label>
            <label class="sched-opt"><input type="radio" name="sched" value="import"><span><b>Importar horarios <span class="fmt-help" tabindex="0" aria-label="Formato">?<span class="fmt-pop" role="tooltip"><b>Formato: ZIP GTFS estándar</b><br>Debe contener <code>routes.txt</code>, <code>trips.txt</code>, <code>stop_times.txt</code> y <code>stops.txt</code> (CSV con cabecera, como el feed de Renfe/Fomento).<br>Columnas mínimas: <code>route_id, route_short_name</code> · <code>trip_id, route_id, service_id</code> · <code>trip_id, stop_id, stop_sequence, arrival_time, departure_time</code> (HH:MM:SS, admite &gt;24h) · <code>stop_id, stop_name, stop_lat, stop_lon</code>.<br>Se conservan las líneas cuyo <code>route_short_name</code> empieza por <b>R</b>.</span></span></b><small>Sube un .zip propio antes de correr</small></span></label>
          </div>
          <label class="file-pick" id="numFileWrap" hidden><input type="file" id="numGtfsFile" accept=".zip"><span id="numFileName">Elegir fichero .zip…</span></label>
          <div id="numGtfsStatus" class="sub" style="margin:6px 0 0"></div>
        </div>
        <div class="tpl-title">Incidencias en juego <span class="sub" style="text-transform:none;letter-spacing:0;font-weight:400">· plantillas del apartado <a href="/incidencias">Incidencias</a>; esta selección es solo de este modo</span></div>
        <label class="all-toggle"><input type="checkbox" id="selAll"><span>Todas / ninguna</span></label>
        <ul class="num-tpls" id="numTpls"><li class="empty">Cargando plantillas…</li></ul>
      </aside>

      <section class="card num-main">
        <div class="res-bar">
          <button class="btn" id="runBtn">__PLAY__ Ejecutar runs</button>
          <span id="prog"></span>
          <div class="dl-menu">
            <button class="btn ghost" id="dlBtn" disabled aria-haspopup="true" aria-expanded="false">__DL__ Descargar resultados ▾</button>
            <div class="dl-pop" id="dlPop" hidden>
              <div class="dl-grp">Sin horarios</div>
              <button data-fmt="xlsx" data-h="0">Excel (.xlsx)</button>
              <button data-fmt="pdf" data-h="0">PDF</button>
              <div class="dl-grp">Con los horarios usados</div>
              <button data-fmt="xlsx" data-h="1">Excel + horarios</button>
              <button data-fmt="pdf" data-h="1">PDF + horarios</button>
            </div>
          </div>
        </div>
        <div id="numLoad" class="rod-load rod-band no-steam" hidden>__SCENE__<div class="rod-load-caption">Simulando la jornada<span class="rod-load-dots" aria-hidden="true"><i></i><i></i><i></i></span></div></div>
        <div class="scroll">
        <table class="num-tbl" id="tbl">
          <thead><tr><th>Semilla</th><th>Trenes</th><th>Retraso máx tren (min)</th><th title="Pico del retraso medio por tren en servicio">Pico retraso medio (min)</th><th title="Pico del retraso acumulado de la red (Σ retrasos vivos)">Pico retraso red (min)</th><th>Pico retenidos</th><th>Final retenidos</th><th>PAX·min</th><th class="act">Incidencias del día</th><th class="act">Retrasos</th></tr></thead>
          <tbody><tr><td class="muted" colspan="10" style="text-align:center;padding:22px">Pulsa «Ejecutar runs» para empezar.</td></tr></tbody>
        </table>
        </div>
      </section>
    </div>
  </div>
  <script>
  (function(){
    const $=id=>document.getElementById(id);
    const EFF={red_one:"roja 1 sentido",red_both:"roja 2 sentidos",single:"vía única"};
    // Plantillas COMPARTIDAS (apartado /incidencias); la SELECCIÓN es independiente por modo
    // (localStorage "rod-sel-numeros") y se guarda por ID ESTABLE de plantilla (editar una
    // plantilla no la desmarca; borrar/añadir desde otro sitio no desplaza la selección).
    let TPLS=[], SEL=null;
    const SELKEY="rod-sel-numeros";
    const rateTxt=t=>`${Math.round(t.prob*100)} % cada ${(Math.round(t.every_h*100)/100).toLocaleString((window.LOCALE?LOCALE():"es-ES"))} h`;
    function loadSel(){
      try{
        const v=JSON.parse(localStorage.getItem(SELKEY));
        if(Array.isArray(v)){
          // Migración: versiones previas guardaban "motivo|efecto|dur|prob" en vez del id.
          const old=new Set(v.filter(x=>typeof x==="string"&&x.includes("|")));
          SEL=new Set(v.filter(x=>typeof x==="string"&&!x.includes("|")));
          TPLS.forEach(t=>{ if(old.has([t.motivo,t.effect,t.dur_min,t.prob].join("|"))) SEL.add(t.id); });
        }
      }catch(e){}
    }
    function saveSel(){ try{ localStorage.setItem(SELKEY, JSON.stringify([...SEL])); }catch(e){} }
    async function loadTpls(){
      try{ TPLS=await (await fetch("/api/incidents/templates")).json(); }catch(e){ TPLS=[]; }
      loadSel();
      if(!SEL) SEL=new Set(TPLS.filter(t=>t.enabled).map(t=>t.id)); // 1ª vez: las activas del servidor
      renderTpls();
    }
    function renderTpls(){
      const ul=$("numTpls"), all=$("selAll");
      const nOn=TPLS.filter(t=>SEL.has(t.id)).length;
      all.checked=TPLS.length>0&&nOn===TPLS.length; all.indeterminate=nOn>0&&nOn<TPLS.length; all.disabled=!TPLS.length;
      if(!TPLS.length){ ul.innerHTML='<li class="empty">Sin plantillas. Créalas en el apartado <a href="/incidencias">Incidencias</a> del menú.</li>'; return; }
      ul.innerHTML="";
      TPLS.forEach((t,i)=>{
        const on=SEL.has(t.id);
        const li=document.createElement("li"); if(!on) li.className="off";
        li.innerHTML=`<input type="checkbox" ${on?"checked":""} data-i="${i}"><span><b>${t.motivo||"(sin motivo)"}</b><small>${EFF[t.effect]||t.effect} · ${t.dur_min} min · ${rateTxt(t)}</small></span>`;
        ul.appendChild(li);
      });
      ul.querySelectorAll("input").forEach(c=>c.onchange=()=>{
        const id=TPLS[+c.dataset.i].id;
        if(c.checked) SEL.add(id); else SEL.delete(id);
        saveSel(); renderTpls();
      });
    }
    // Casilla maestra: marcar todas / ninguna de golpe.
    $("selAll").onchange=()=>{ SEL=$("selAll").checked?new Set(TPLS.map(t=>t.id)):new Set(); saveSel(); renderTpls(); };
    loadTpls();

    // Selector de horarios (igual que el visual): guardado (con su fecha) / nuevo / importar.
    let GTFS_DATE="";   // fecha del GTFS cargado (se adjunta a cada tanda para "ver en simulador")
    async function infoGtfs(){
      try{
        const j=await (await fetch("/api/gtfs/info")).json();
        GTFS_DATE=j.date||"";
        const p=(j.date||"").split("-");
        $("gtfsSavedInfo").textContent=(j.date?`Del ${p[2]}/${p[1]}/${p[0]}`:"Fecha desconocida")+` · ${j.n_lines} líneas · ${j.n_trips.toLocaleString((window.LOCALE?LOCALE():"es-ES"))} viajes (por defecto)`;
      }catch(e){ $("gtfsSavedInfo").textContent="El que está cargado ahora (por defecto)"; }
    }
    infoGtfs();
    const schedMode=()=>(document.querySelector('input[name="sched"]:checked')||{}).value||"saved";
    $("schedOpts").onchange=()=>{ $("numFileWrap").hidden=schedMode()!=="import"; $("numGtfsStatus").textContent=""; };
    $("numGtfsFile").onchange=()=>{ const f=$("numGtfsFile").files[0]; $("numFileName").textContent=f?f.name:"Elegir fichero .zip…"; };
    // Arranca la descarga/importación en el servidor y espera a que acabe (true = OK).
    async function jobGtfs(start){
      const st=$("numGtfsStatus"); let r;
      try{ r=await (await start()).json(); }catch(e){ r={started:false,reason:String(e)}; }
      if(!r.started){ st.textContent=r.reason||"No se pudo iniciar."; return false; }
      for(;;){
        await new Promise(res=>setTimeout(res,900));
        let j; try{ j=await (await fetch("/api/gtfs/status")).json(); }catch(e){ j={}; }
        if(j.error){ st.textContent="Error: "+j.error; return false; }
        st.textContent=j.step||"Trabajando…";
        if(j.done){ st.textContent="GTFS actualizado."; await infoGtfs(); document.querySelector('input[name="sched"][value="saved"]').checked=true; $("numFileWrap").hidden=true; return true; }
      }
    }

    $("runBtn").onclick=async()=>{
      const a=+$("s0").value||0, b=+$("s1").value||0;
      const mode=schedMode();
      $("runBtn").disabled=true;
      if(mode==="new"){
        $("prog").textContent="Descargando el GTFS de hoy…";
        if(!(await jobGtfs(()=>fetch("/api/gtfs/update")))){ $("prog").textContent=""; $("runBtn").disabled=false; return; }
      }else if(mode==="import"){
        const f=$("numGtfsFile").files[0];
        if(!f){ $("numGtfsStatus").textContent="Elige el fichero .zip a importar."; $("runBtn").disabled=false; return; }
        $("prog").textContent="Importando "+f.name+"…";
        if(!(await jobGtfs(()=>fetch("/api/gtfs/upload",{method:"POST",body:f})))){ $("prog").textContent=""; $("runBtn").disabled=false; return; }
      }
      const tb=$("tbl").querySelector("tbody"); tb.innerHTML="";
      const nl=$("numLoad"); if(nl) nl.hidden=false;
      const chosen=TPLS.filter(t=>SEL.has(t.id));
      const selQ = "&sel="+chosen.map(t=>encodeURIComponent(t.id)).join(",");
      // Resultados de ESTA tanda (para exportar): runs + plantillas en juego + rango de semillas.
      LAST={runs:[],templates:chosen.map(t=>({motivo:t.motivo,effect:t.effect,dur_min:t.dur_min,prob:t.prob,every_h:t.every_h})),a,b,
        selIds:chosen.map(t=>t.id),gtfs:GTFS_DATE};
      $("dlBtn").disabled=true;
      for(let s=a;s<=b;s++){
        $("prog").textContent="Ejecutando semilla "+s+"…";
        try{
          const r=await (await fetch("/api/simulate?seed="+s+selQ)).json();
          const k=LAST.runs.push(r)-1;
          const ni=(r.incidents||[]).length;
          // Picos en HORAS DE SERVICIO (≥25 % del máximo de trenes circulando, igual que el
          // análisis): al cierre, con 3 trenes, la media salta con cualquier llegada tarde.
          const pk=a=>window.DelayTrack&&r.track?DelayTrack.servicePeak(r.track,a).v:null;
          const f=(v,d)=>v==null?"–":v.toLocaleString((window.LOCALE?LOCALE():"es-ES"),{minimumFractionDigits:d,maximumFractionDigits:d});
          const tr=document.createElement("tr");
          tr.innerHTML=`<td>${s}</td><td>${r.n_trains}</td><td>${r.max_delay_min.toFixed(0)}</td><td>${f(pk(r.track&&r.track.mean),2)}</td><td>${f(pk(r.track&&r.track.total),0)}</td><td>${r.peak_waiting}</td><td>${r.final_waiting}</td><td>${Math.round(r.pax_min).toLocaleString((window.LOCALE?LOCALE():"es-ES"))}</td>
            <td class="act"><button class="row-btn" data-inc="${k}" ${ni?"":"disabled"}>__ALERT__ Incidencias <span class="cnt">${ni}</span></button></td>
            <td class="act"><div class="row-acts"><button class="row-btn" data-trk="${k}">__CHART__ Seguimiento</button><a class="row-btn" target="_blank" rel="noopener" title="Abre el simulador visual con esta misma semilla, incidencias y horarios: solo queda pulsar Confirmar" href="/game?seed=${s}&sel=${encodeURIComponent(LAST.selIds.join(","))}&from=numeros${LAST.gtfs?"&gtfs="+LAST.gtfs:""}">__PLAY__ Ver en simulador</a></div></td>`;
          tb.appendChild(tr);
        }catch(e){
          const tr=document.createElement("tr"); tr.innerHTML=`<td>${s}</td><td colspan="9">error</td>`; tb.appendChild(tr);
        }
      }
      $("prog").textContent="Hecho.";
      if(nl) nl.hidden=true;
      $("runBtn").disabled=false;
      $("dlBtn").disabled=!LAST.runs.length;
    };

    // Botones por run: incidencias del día (por gravedad) y seguimiento de retrasos (gráficos +
    // análisis de recuperación). Delegación en la tabla: las filas se crean conforme llegan runs.
    $("tbl").addEventListener("click",e=>{
      const b=e.target.closest("button[data-inc],button[data-trk]"); if(!b||!LAST||!window.DelayTrack) return;
      const r=LAST.runs[+(b.dataset.inc??b.dataset.trk)]; if(!r) return;
      const title="Semilla "+r.seed;
      if(b.dataset.inc!=null) DelayTrack.openIncidents(r,{title}); else DelayTrack.open(r,{title});
    });

    // ── Descargar resultados (Excel / PDF, con o sin los horarios usados) ──
    let LAST=null;
    const pop=$("dlPop");
    const closePop=()=>{ pop.hidden=true; $("dlBtn").setAttribute("aria-expanded","false"); };
    $("dlBtn").onclick=e=>{ e.stopPropagation(); pop.hidden=!pop.hidden; $("dlBtn").setAttribute("aria-expanded",String(!pop.hidden)); };
    document.addEventListener("click",e=>{ if(!pop.hidden&&!pop.contains(e.target)) closePop(); });
    document.addEventListener("keydown",e=>{ if(e.key==="Escape") closePop(); });
    pop.querySelectorAll("button").forEach(btn=>btn.onclick=async()=>{
      closePop();
      if(!LAST||!LAST.runs.length) return;
      const fmt=btn.dataset.fmt, hor=btn.dataset.h==="1";
      $("prog").textContent="Generando "+(fmt==="pdf"?"PDF":"Excel")+(hor?" con horarios":"")+"…";
      $("dlBtn").disabled=true;
      try{
        const res=await fetch(`/api/runs/export?fmt=${fmt}&horarios=${hor?1:0}`,{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify({runs:LAST.runs.map(({track,analysis,...r})=>r),templates:LAST.templates})});
        if(!res.ok) throw new Error(await res.text());
        const blob=await res.blob(), url=URL.createObjectURL(blob);
        const a=document.createElement("a");
        a.href=url; a.download=`rodalies_runs_semillas_${LAST.a}-${LAST.b}${hor?"_con_horarios":""}.${fmt}`;
        document.body.appendChild(a); a.click(); a.remove();
        setTimeout(()=>URL.revokeObjectURL(url),4000);
        $("prog").textContent="Descargado.";
      }catch(e){ $("prog").textContent="No se pudo generar el fichero: "+e.message; }
      finally{ $("dlBtn").disabled=false; }
    });
  })();
  </script>
  <script src="/game/delay_track.js"></script>"##;
    // Incrusta la escena del tren pixel y los iconos SVG (play / descarga / alerta / gráfico).
    let body = body
        .replace("__SCENE__", TRAIN_SCENE)
        .replace("__ALERT__", icon("warning"))
        .replace("__CHART__", icon("chart"))
        .replace("__PLAY__", icon("play"))
        .replace("__DL__", icon("download"));
    document(&head, &body)
}

/// Página del CREADOR DE INCIDENCIAS (apartado propio del menú): crear, editar y borrar las
/// plantillas compartidas por el modo visual y `/numeros`. Cada plantilla = motivo + efecto +
/// duración + **probabilidad cada X horas** (el motor trocea la jornada en secciones de X h y tira
/// los dados en cada una). La SELECCIÓN de qué plantillas entran se hace en cada modo, no aquí.
pub fn render_incidents_page() -> String {
    let head = format!("<title>Rodalies Simulator · Incidencias</title>\n{STYLE}");
    let body = r##"<style>
.menu-wrap{max-width:1180px}
.inc-layout{display:grid;grid-template-columns:360px minmax(0,1fr);gap:20px;align-items:start;margin-top:14px}
@media(max-width:860px){.inc-layout{grid-template-columns:1fr}}
.inc-form h2{margin:0 0 4px}
.inc-form .sub{margin:0 0 14px}
.fld{display:flex;flex-direction:column;gap:4px;font-size:.74rem;font-weight:700;letter-spacing:.04em;text-transform:uppercase;color:var(--muted);margin-bottom:12px;min-width:0}
.fld input,.fld select{width:100%;min-width:0;height:38px;background:var(--panel2);border:1px solid var(--border);border-radius:7px;padding:0 10px;color:var(--text);font:inherit;font-weight:400;text-transform:none;letter-spacing:0}
.fld-row{display:flex;gap:10px;align-items:flex-end}
.sub a{color:var(--accent)}
.fld-row .fld{flex:1}
.rate-box{background:var(--accent-soft);border:1px solid var(--border);border-left:3px solid var(--accent);border-radius:8px;padding:10px 12px;font-size:.84rem;margin:2px 0 14px;line-height:1.45}
.rate-box b{color:var(--text)}
.form-btns{display:flex;gap:8px}
.form-btns .btn{flex:1;display:inline-flex;align-items:center;justify-content:center;gap:7px;border-radius:8px}
.btn.ghost{background:var(--panel2);color:var(--text);border:1px solid var(--border)}
.btn.ghost:hover{border-color:var(--accent)}
.inc-form.editing{border-color:var(--accent);box-shadow:0 0 0 2px var(--accent-soft)}
.tpl-cards{list-style:none;margin:0;padding:0;display:flex;flex-direction:column;gap:8px}
.tpl-card{display:grid;grid-template-columns:1fr auto;gap:10px;align-items:center;background:var(--panel);border:1px solid var(--border);border-left:4px solid var(--accent);border-radius:9px;padding:11px 14px}
.tpl-card.sel{box-shadow:0 0 0 2px var(--accent-soft);border-color:var(--accent)}
.tpl-card b{font-size:.98rem}
.tpl-card .meta{color:var(--muted);font-size:.8rem;margin-top:2px}
.tpl-card .rate{font-variant-numeric:tabular-nums}
.tpl-card .acts{display:flex;gap:6px}
.icon-act{border:1px solid var(--border);background:var(--panel2);color:var(--text);border-radius:7px;padding:6px 11px;font:inherit;font-size:.8rem;font-weight:600;cursor:pointer}
.icon-act:hover{border-color:var(--accent);color:var(--accent)}
.icon-act.del:hover{border-color:var(--late);color:var(--late)}
.list-head{display:flex;align-items:baseline;justify-content:space-between;gap:10px;margin-bottom:10px}
.empty-list{color:var(--muted);font-style:italic;padding:18px;text-align:center;border:1px dashed var(--border);border-radius:9px}
#saveMsg{min-height:1.2em;font-size:.8rem;color:var(--muted);margin-top:8px}
</style>
<div class="menu-wrap">
  <div class="panel-bar"><a class="back-link" href="/">← Menú</a></div>
  <div class="menu-top"><div class="menu-brand"><img class="logo" src="/game/lines/r_logo.png" alt="Rodalies" width="46" height="46"><div>
    <h1>Incidencias · creador de plantillas</h1>
    <div class="sub">Plantillas compartidas por el <a href="/game">modo visual</a> y <a href="/numeros">Varias runs</a>. Cada modo elige después cuáles entran en juego.</div>
  </div></div></div>

  <div class="inc-layout">
    <aside class="card inc-form" id="tplForm">
      <h2 id="formTitle">Nueva plantilla</h2>
      <p class="sub">La jornada se trocea en secciones de <b>X horas</b> y en cada una se tiran los dados con tu probabilidad: la misma incidencia puede ocurrir varias veces en un día.</p>
      <label class="fld">Motivo<input id="fMotivo" type="text" placeholder="p. ej. Avería de puertas"></label>
      <label class="fld">Efecto<select id="fEffect">
        <option value="red_one">Señal roja (1 sentido)</option>
        <option value="red_both">Señal roja (2 sentidos)</option>
        <option value="single">Tramo → vía única</option>
      </select></label>
      <div class="fld-row">
        <label class="fld">Duración (min)<input id="fDur" type="number" min="1" value="15"></label>
        <label class="fld">Probabilidad (%)<input id="fProb" type="number" min="0" max="100" step="1" value="20"></label>
        <label class="fld">Cada (horas)<input id="fEvery" type="number" min="0.25" step="0.25" value="4"></label>
      </div>
      <div class="rate-box" id="rateBox"></div>
      <div class="form-btns">
        <button class="btn" id="fSave">+ Añadir plantilla</button>
        <button class="btn ghost" id="fCancel" hidden>Cancelar</button>
      </div>
      <div id="saveMsg"></div>
    </aside>

    <section class="card">
      <div class="list-head"><h2 style="margin:0">Plantillas guardadas</h2><span class="sub" id="dayInfo"></span></div>
      <ul class="tpl-cards" id="tplCards"><li class="empty-list">Cargando…</li></ul>
    </section>
  </div>
</div>
<script>
(function(){
  const $=id=>document.getElementById(id);
  const EFF={red_one:"Señal roja (1 sentido)",red_both:"Señal roja (2 sentidos)",single:"Tramo → vía única"};
  const esc=x=>String(x).replace(/&/g,"&amp;").replace(/</g,"&lt;").replace(/>/g,"&gt;").replace(/"/g,"&quot;");
  let TPLS=[], editId=null, dayH=19;   // duración de la jornada (h): de la 1ª salida a las 24:00
  const hFmt=h=>(Math.round(h*100)/100).toLocaleString((window.LOCALE?LOCALE():"es-ES"));
  // Nº de secciones y esperanza de incidencias/día con la MISMA regla que el motor
  // (engine::n_sections): nº entero de secciones iguales más cercano a jornada/X, mínimo 1.
  function rate(prob, every){
    const X=Math.max(0.25, every||24), n=Math.max(1, Math.round(dayH/X));
    return { n, esperadas: prob * n };
  }
  function rateTxt(t){ return `${Math.round(t.prob*100)} % cada ${hFmt(t.every_h)} h`; }
  function paintRate(){
    const p=Math.min(100,Math.max(0,+$("fProb").value||0))/100, X=Math.max(0.25,+$("fEvery").value||24);
    const r=rate(p,X);
    $("rateBox").innerHTML=`En una jornada de <b>${hFmt(dayH)} h</b>: <b>${r.n}</b> tirada${r.n===1?"":"s"} de dados (secciones iguales de ${hFmt(dayH/r.n)} h) → de media <b>${r.esperadas.toLocaleString((window.LOCALE?LOCALE():"es-ES"),{maximumFractionDigits:2})}</b> incidencia${r.esperadas===1?"":"s"} al día.`;
  }
  ["fProb","fEvery"].forEach(id=>$(id).addEventListener("input",paintRate));

  async function load(){
    try{
      const g=await (await fetch("/api/gtfs/info")).json();
      if(g.first_dep!=null){ dayH=(86400-g.first_dep)/3600; const h=Math.floor(g.first_dep/3600), m=Math.floor(g.first_dep%3600/60);
        $("dayInfo").textContent=`Jornada ${String(h).padStart(2,"0")}:${String(m).padStart(2,"0")} → 24:00 (${hFmt(dayH)} h)`; }
    }catch(e){}
    try{ TPLS=await (await fetch("/api/incidents/templates")).json(); }catch(e){ TPLS=[]; }
    render(); paintRate();
  }
  async function save(msg){
    try{
      const r=await (await fetch("/api/incidents/templates",{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify(TPLS)})).json();
      if(r.templates) TPLS=r.templates;   // ids definitivos asignados por el servidor
      $("saveMsg").textContent=r.ok?msg:"No se pudo guardar.";
    }catch(e){ $("saveMsg").textContent="No se pudo guardar."; }
    render();
  }
  function render(){
    const ul=$("tplCards");
    if(!TPLS.length){ ul.innerHTML='<li class="empty-list">Sin plantillas. Crea la primera a la izquierda.</li>'; return; }
    ul.innerHTML=TPLS.map(t=>{
      const r=rate(t.prob,t.every_h);
      return `<li class="tpl-card${t.id===editId?" sel":""}"><div><b>${esc(t.motivo||"(sin motivo)")}</b>
        <div class="meta">${EFF[t.effect]||t.effect} · ${t.dur_min} min · <span class="rate">${rateTxt(t)}</span> · ≈ ${r.esperadas.toLocaleString((window.LOCALE?LOCALE():"es-ES"),{maximumFractionDigits:2})}/día</div></div>
        <div class="acts"><button class="icon-act" data-edit="${esc(t.id)}">Editar</button><button class="icon-act del" data-del="${esc(t.id)}">Eliminar</button></div></li>`;
    }).join("");
    ul.querySelectorAll("[data-edit]").forEach(b=>b.onclick=()=>startEdit(b.dataset.edit));
    ul.querySelectorAll("[data-del]").forEach(b=>b.onclick=()=>{
      const t=TPLS.find(x=>x.id===b.dataset.del); if(!t) return;
      if(!confirm((window.I18N?I18N.t("¿Eliminar la plantilla?"):"¿Eliminar la plantilla?")+` «${t.motivo}» — `+(window.I18N?I18N.t("Se quita para los dos modos."):"Se quita para los dos modos."))) return;
      TPLS=TPLS.filter(x=>x.id!==t.id); if(editId===t.id) resetForm();
      save("Plantilla eliminada.");
    });
  }
  function startEdit(id){
    const t=TPLS.find(x=>x.id===id); if(!t) return;
    editId=id;
    $("fMotivo").value=t.motivo; $("fEffect").value=t.effect; $("fDur").value=t.dur_min;
    $("fProb").value=Math.round(t.prob*1000)/10; $("fEvery").value=t.every_h;
    $("formTitle").textContent="Editar plantilla"; $("fSave").textContent="Guardar cambios"; $("fCancel").hidden=false;
    $("tplForm").classList.add("editing"); paintRate(); render(); $("fMotivo").focus();
  }
  function resetForm(){
    editId=null; $("fMotivo").value=""; $("fEffect").value="red_one"; $("fDur").value=15; $("fProb").value=20; $("fEvery").value=4; paintRate();
    $("formTitle").textContent="Nueva plantilla"; $("fSave").textContent="+ Añadir plantilla"; $("fCancel").hidden=true;
    $("tplForm").classList.remove("editing"); render();
  }
  $("fCancel").onclick=resetForm;
  $("fSave").onclick=()=>{
    const v={
      motivo:($("fMotivo").value||"").trim()||"Incidencia",
      effect:$("fEffect").value,
      dur_min:Math.max(1,+$("fDur").value||15),
      prob:Math.min(1,Math.max(0,(+$("fProb").value||0)/100)),
      every_h:Math.max(0.25,+$("fEvery").value||24),
    };
    if(editId){
      const t=TPLS.find(x=>x.id===editId); if(t) Object.assign(t,v);
      resetForm(); save("Cambios guardados.");
    }else{
      TPLS.push({id:"",enabled:true,...v}); $("fMotivo").value="";
      save("Plantilla añadida.");
    }
  };
  load();
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
         <div id=\"pane-sim\" class=\"tabpane active\">\n  {ctrls}\n  {optp}\n  <div id=\"dashboard\">\n{body}\n  </div>\n  </div>\n  \
         <div id=\"pane-calc\" class=\"tabpane active\">\n  {calc_panel}\n  </div>\n\
         </div>\n  {modal}\n{SCRIPT}\n{OPT_SCRIPT}\n{CALC_SCRIPT}\n{CALC2_SCRIPT}\n{SETTINGS_SCRIPT}\n{HASH_TAB_SCRIPT}",
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
