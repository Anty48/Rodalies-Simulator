//! Generación de un dashboard HTML autocontenido (sin dependencias externas) para
//! visualizar el resultado de la simulación. Todo el CSS/JS va embebido, de modo que
//! el archivo se puede abrir con doble clic y funciona sin conexión.

use crate::gtfs_loader::fmt_hms;
use crate::simulation_engine::{CtcEvent, Sample};

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
    pub events: Vec<CtcEvent>,
    pub trains_run: usize,
    pub arrivals: usize,
    pub held: usize,
    pub peak_total: i64,
    pub peak_time: String,
    pub peak_delayed: usize,
    pub peak_mean: f64,
    pub recovery: Option<String>,
    pub timeline: Vec<Sample>,
    /// SVG del mapa geográfico de la red con trenes animados.
    pub map_svg: String,
}

pub struct ResRow {
    pub mins: u32,
    pub peak: i64,
    pub delayed: usize,
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

/// Gráfico de área SVG del retraso acumulado de la red a lo largo del tiempo.
fn timeline_svg(timeline: &[Sample]) -> String {
    if timeline.is_empty() {
        return "<p class=\"muted\">Sense dades de línia temporal.</p>".into();
    }
    let w = 1000.0f64;
    let h = 280.0f64;
    let pad_l = 60.0;
    let pad_r = 20.0;
    let pad_t = 20.0;
    let pad_b = 40.0;
    let plot_w = w - pad_l - pad_r;
    let plot_h = h - pad_t - pad_b;

    let max_d = timeline
        .iter()
        .map(|s| s.total_delay)
        .max()
        .unwrap_or(1)
        .max(1) as f64;
    let n = timeline.len();
    let x = |i: usize| pad_l + plot_w * (i as f64) / ((n - 1).max(1) as f64);
    let y = |d: i64| pad_t + plot_h * (1.0 - (d as f64 / max_d));

    // Área.
    let mut area = String::new();
    area.push_str(&format!("M {:.1} {:.1}", x(0), y(timeline[0].total_delay)));
    for (i, s) in timeline.iter().enumerate().skip(1) {
        area.push_str(&format!(" L {:.1} {:.1}", x(i), y(s.total_delay)));
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
            &fmt_hms(timeline[i].time)[..5]
        ));
        i += step;
    }

    format!(
        r##"<svg viewBox="0 0 {w:.0} {h:.0}" preserveAspectRatio="xMidYMid meet" class="chart" role="img" aria-label="Retard acumulat de la xarxa">
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
    pub cap: u32,
    pub headway: u32,
    pub random: bool,
}

const STYLE: &str = r#"<style>
:root {
  --bg:#0e1116; --panel:#161b22; --panel2:#1c232d; --border:#2a333f;
  --text:#e6edf3; --muted:#8b98a5; --accent:#e2231a; --accent2:#f5a623;
  --arr:#3fb950; --dep:#58a6ff; --late:#f85149; --warn:#f5a623;
}
* { box-sizing:border-box; }
body { margin:0; background:var(--bg); color:var(--text);
  font-family:'Segoe UI',system-ui,-apple-system,sans-serif; line-height:1.5; }
.mono { font-family:'Cascadia Code',Consolas,'Courier New',monospace; }
.muted { color:var(--muted); font-size:.9em; }
code { background:var(--panel2); padding:1px 5px; border-radius:5px; font-size:.85em; }
.wrap { max-width:1200px; margin:0 auto; padding:24px 20px 60px; }
header.top { display:flex; align-items:center; gap:16px; padding:8px 0 20px; border-bottom:1px solid var(--border); margin-bottom:24px; }
.logo { width:46px; height:46px; border-radius:12px; background:linear-gradient(135deg,var(--accent),var(--accent2));
  display:flex; align-items:center; justify-content:center; font-size:26px; flex:0 0 auto; box-shadow:0 4px 18px rgba(226,35,26,.35); }
h1 { font-size:1.5rem; margin:0; }
h2 { font-size:1.05rem; margin:0 0 14px; letter-spacing:.02em; text-transform:uppercase; color:var(--muted); }
h3 { margin:0 0 4px; font-size:1.05rem; }
.sub { color:var(--muted); font-size:.85rem; }
.grid-kpi { display:grid; grid-template-columns:repeat(auto-fit,minmax(150px,1fr)); gap:14px; margin-bottom:26px; }
.kpi { background:var(--panel); border:1px solid var(--border); border-radius:14px; padding:16px 18px; }
.kpi-val { font-size:1.7rem; font-weight:700; }
.kpi-label { font-size:.9rem; margin-top:2px; }
.kpi-sub { color:var(--muted); font-size:.75rem; }
.card { background:var(--panel); border:1px solid var(--border); border-radius:16px; padding:22px; margin-bottom:22px; }
.cols { display:grid; grid-template-columns:1fr 1fr; gap:22px; }
@media (max-width:820px) { .cols { grid-template-columns:1fr; } }
.scroll { overflow-x:auto; }
table { width:100%; border-collapse:collapse; font-size:.9rem; }
th,td { text-align:left; padding:7px 10px; border-bottom:1px solid var(--border); white-space:nowrap; }
th { color:var(--muted); font-weight:600; font-size:.78rem; text-transform:uppercase; letter-spacing:.03em; }
td.num { text-align:right; }
.pill { background:var(--panel2); border:1px solid var(--border); border-radius:20px; padding:1px 9px; font-size:.8rem; }
.chart { width:100%; height:auto; display:block; }
.map { width:100%; height:auto; display:block; background:var(--bg); border:1px solid var(--border); border-radius:10px; }
.grid { stroke:var(--border); stroke-width:1; }
.axis { fill:var(--muted); font-size:12px; font-family:monospace; }
.bar-row { display:flex; align-items:center; gap:10px; margin:6px 0; font-size:.85rem; }
.bar-name { width:52px; color:var(--muted); }
.bar-val { width:52px; text-align:right; }
.bar-track { flex:1; height:10px; background:var(--panel2); border-radius:6px; overflow:hidden; }
.bar-track.sm { height:8px; }
.bar-fill { display:block; height:100%; background:linear-gradient(90deg,var(--accent2),var(--accent)); border-radius:6px; }
.metrics { display:grid; grid-template-columns:1fr 1fr; gap:10px 22px; }
.metric { display:flex; justify-content:space-between; border-bottom:1px dashed var(--border); padding:6px 0; font-size:.9rem; }
.metric-label { color:var(--muted); }
.metric-val { font-weight:600; }
.chips { display:flex; flex-wrap:wrap; gap:6px; margin-top:6px; }
.chip { background:var(--panel2); border:1px solid var(--border); border-radius:20px; padding:2px 10px; font-size:.78rem; color:var(--muted); }
.inc { list-style:none; padding:0; margin:0; }
.inc li { background:rgba(226,35,26,.08); border-left:3px solid var(--accent); padding:8px 12px; border-radius:6px; margin-bottom:8px; font-size:.88rem; }
.log { max-height:520px; overflow:auto; border:1px solid var(--border); border-radius:10px; background:var(--bg); }
.ev { display:grid; grid-template-columns:64px 22px 58px 1fr 1.4fr 62px 66px; align-items:center; gap:8px; padding:4px 12px; font-size:.82rem; border-bottom:1px solid rgba(42,51,63,.5); }
.ev-inc { grid-template-columns:22px 1fr; background:rgba(226,35,26,.10); color:var(--accent2); font-weight:600; }
.ev-badge { text-align:center; }
.ev-arr .ev-badge { color:var(--arr); }
.ev-dep .ev-badge { color:var(--dep); }
.ev-kind { color:var(--muted); font-size:.75rem; }
.ev-train em { color:var(--accent2); font-style:normal; font-size:.78rem; }
.ev-sta { color:var(--text); }
.ev-track,.ev-delay { text-align:right; color:var(--muted); }
.ev.warn .ev-delay { color:var(--warn); }
.ev.late .ev-delay { color:var(--late); font-weight:700; }
footer { color:var(--muted); font-size:.8rem; text-align:center; padding-top:20px; border-top:1px solid var(--border); }
.controls { background:var(--panel); border:1px solid var(--border); border-radius:16px; padding:18px 22px; margin-bottom:22px; }
.controls-grid { display:grid; grid-template-columns:repeat(auto-fit,minmax(155px,1fr)); gap:16px 20px; align-items:end; }
.field { display:flex; flex-direction:column; gap:5px; font-size:.78rem; color:var(--muted); }
.field label b { color:var(--accent2); }
.field input[type=range] { width:100%; accent-color:var(--accent); }
.field select { background:var(--panel2); color:var(--text); border:1px solid var(--border); border-radius:8px; padding:7px 8px; }
.field .chk { display:flex; align-items:center; gap:7px; color:var(--text); font-size:.9rem; }
.btn { background:linear-gradient(135deg,var(--accent),var(--accent2)); color:#fff; border:none; border-radius:10px;
  padding:11px 22px; font-size:.95rem; font-weight:700; cursor:pointer; box-shadow:0 3px 12px rgba(226,35,26,.3); }
.btn:disabled { opacity:.6; cursor:progress; }
#dashboard { transition:opacity .15s; }
.optchart { width:100%; height:auto; display:block; background:var(--bg); border:1px solid var(--border); border-radius:8px; margin-top:12px; }
.optdone { margin-top:12px; font-size:1rem; padding:12px 16px; background:var(--panel2); border:1px solid var(--border); border-radius:10px; }
.optdone a { color:var(--accent2); font-weight:700; text-decoration:none; }
.formula { font-family:'Cascadia Code',Consolas,monospace; background:var(--panel2); border:1px solid var(--border); border-radius:8px; padding:10px 14px; font-size:.85rem; overflow-x:auto; }
</style>"#;

const HEADER: &str = r#"<header class="top">
    <div class="logo">🚆</div>
    <div>
      <h1>rodalies-sim <span class="sub">· dashboard</span></h1>
      <div class="sub">Simulació de tràfic ferroviari i incidències · Rodalies de Barcelona · construït des de GTFS</div>
    </div>
  </header>"#;

const SCRIPT: &str = r#"<script>
const $=id=>document.getElementById(id);
function qs(){
  const p=new URLSearchParams();
  p.set('start_h',$('c_start').value); p.set('dur_h',$('c_dur').value);
  p.set('line',$('c_line').value); p.set('block',$('c_block').value);
  p.set('delay',$('c_delay').value); p.set('cap',$('c_cap').value);
  p.set('headway',$('c_headway').value); p.set('random',$('c_random').checked?'1':'0');
  return p.toString();
}
async function simulate(){
  const b=$('c_run'); b.disabled=true; const t=b.textContent; b.textContent='Simulant…';
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
    <h2>Optimitzador del SISTEMA · minimització del potencial V(H)</h2>
    <div class="formula">V(H) = w_delay·<b>retard ponderat per passatgers</b> (×3 en punta) + w_conf·<b>conflictes entre línies</b> &nbsp;·&nbsp; integrat sobre tot el dia (05:00–00:00)</div>
    <p class="muted" style="margin-top:8px">Coordina <b>TOTES les línies alhora</b>: el problema són els conflictes entre línies als cantons compartits i la resposta a les <b>incidències</b>. Recuit simulat sobre el <b>desfàs de fase de cada línia</b> (±5 min); cada candidat s'avalua amb moltes <b>simulacions Monte Carlo del sistema sencer</b>, cadascuna amb incidències aleatòries repartides pel dia (<b>rayon</b>). Física: cantons per blocs, vía única (testigo), andanes reals, sense busos de substitució.</p>
    <button class="btn" id="o_run">▶ Optimitzar el sistema (dia laborable)</button>
    <div id="o_status" class="muted" style="margin-top:10px">Preparat. Prem per veure baixar V(H) en viu al llarg de moltes simulacions.</div>
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
   ctx.strokeStyle='#2a333f';ctx.fillStyle='#8b98a5';ctx.font='11px monospace';ctx.lineWidth=1;
   ctx.beginPath();ctx.moveTo(P,y(mx));ctx.lineTo(W-P,y(mx));ctx.stroke();ctx.fillText(mx.toFixed(0),2,y(mx)+4);
   ctx.beginPath();ctx.moveTo(P,y(mn));ctx.lineTo(W-P,y(mn));ctx.stroke();ctx.fillText(mn.toFixed(0),2,y(mn)+4);
   if(base){ctx.strokeStyle='#8b98a5';ctx.setLineDash([4,4]);ctx.beginPath();ctx.moveTo(P,y(base));ctx.lineTo(W-P,y(base));ctx.stroke();ctx.setLineDash([]);ctx.fillText('V inicial',W-P-52,y(base)-4);}
   ctx.strokeStyle='#e2231a';ctx.lineWidth=2;ctx.beginPath();h.forEach((v,i)=>{i?ctx.lineTo(x(i),y(v)):ctx.moveTo(x(i),y(v));});ctx.stroke();
 }
 async function poll(){ let j; try{j=await (await fetch('/api/optimize/status')).json();}catch(e){return;}
   draw(j.history,j.base_v);
   if(j.running){ $('o_status').textContent='Optimitzant el sistema… iteració '+j.iter+'/'+j.total+'  ·  V actual '+j.current_v.toFixed(1)+'  ·  millor '+j.best_v.toFixed(1); }
   else if(j.done){ if(timer){clearInterval(timer);timer=null;} $('o_run').disabled=false;
     if(j.error){ $('o_status').innerHTML='<span style="color:#f85149">⚠ '+j.error+'</span>'; }
     else {
       $('o_status').innerHTML='<b style="color:#3fb950">✔ Optimització FINALITZADA</b> · '+j.total+' iteracions · '+j.trips+' trens coordinats';
       let rows=(j.files||[]).map(f=>'<tr><td class=mono>'+f.line+'</td><td class=num>'+(f.offset_min>=0?'+':'')+f.offset_min+' min</td><td><a href="'+f.csv+'" target=_blank>CSV</a></td><td><a href="'+f.pdf+'" target=_blank>PDF</a></td></tr>').join('');
       $('o_result').innerHTML='<div class="optdone">Potencial V: <b>'+j.base_v.toFixed(1)+'</b> → <b style="color:#f5a623">'+j.best_v.toFixed(1)+'</b> (<b>−'+j.delta_pct.toFixed(1)+'%</b>) &nbsp;·&nbsp; pic de retard mitjà '+j.base_delay.toFixed(0)+' → '+j.best_delay.toFixed(0)+' s &nbsp;·&nbsp; recuperació '+j.base_recovery.toFixed(1)+' → '+j.best_recovery.toFixed(1)+' min</div>'
         +'<div class="scroll" style="margin-top:10px"><table><thead><tr><th>línia</th><th>desfàs</th><th>horari</th><th></th></tr></thead><tbody>'+rows+'</tbody></table></div>';
     }
   }
 }
 function start(){ const b=$('o_run'); b.disabled=true; $('o_result').innerHTML=''; $('o_status').textContent='Iniciant… (simulant el sistema sencer amb incidències)';
   fetch('/api/optimize/start').then(()=>{ if(timer)clearInterval(timer); timer=setInterval(poll,500); poll(); });
 }
 const b=$('o_run'); if(b) b.addEventListener('click',start);
})();
</script>"#;

fn controls_html(lines: &[String], c: &Controls) -> String {
    let mut opts = String::from("<option value=\"\">Totes les línies</option>");
    for l in lines {
        let sel = if c.line.as_deref() == Some(l.as_str()) { " selected" } else { "" };
        opts.push_str(&format!("<option value=\"{0}\"{1}>{0}</option>", esc(l), sel));
    }
    let chk = if c.random { " checked" } else { "" };
    format!(
        r#"<div class="controls">
    <div class="controls-grid">
      <div class="field"><label>Hora inici <b id="o_start">{start}</b>h</label>
        <input type="range" id="c_start" min="5" max="21" step="1" value="{start}" data-out="o_start"></div>
      <div class="field"><label>Durada <b id="o_dur">{dur}</b>h</label>
        <input type="range" id="c_dur" min="1" max="4" step="1" value="{dur}" data-out="o_dur"></div>
      <div class="field"><label>Línia</label>
        <select id="c_line">{opts}</select></div>
      <div class="field"><label>Bloqueig cantó <b id="o_block">{block}</b> min</label>
        <input type="range" id="c_block" min="0" max="20" step="1" value="{block}" data-out="o_block"></div>
      <div class="field"><label>Retard tren <b id="o_delay">{delay}</b> min</label>
        <input type="range" id="c_delay" min="0" max="15" step="1" value="{delay}" data-out="o_delay"></div>
      <div class="field"><label>Vies/andana <b id="o_cap">{cap}</b></label>
        <input type="range" id="c_cap" min="1" max="8" step="1" value="{cap}" data-out="o_cap"></div>
      <div class="field"><label>Sep. mín. bloc <b id="o_headway">{headway}</b> s</label>
        <input type="range" id="c_headway" min="60" max="300" step="30" value="{headway}" data-out="o_headway"></div>
      <div class="field"><label class="chk"><input type="checkbox" id="c_random"{chk}> Passatgers estocàstics</label></div>
      <div class="field"><button class="btn" id="c_run">▶ Simular</button></div>
    </div>
  </div>"#,
        start = c.start_h,
        dur = c.dur_h,
        opts = opts,
        block = c.block,
        delay = c.delay,
        cap = c.cap,
        headway = c.headway,
        chk = chk,
    )
}

/// Página estática (offline): estilos + cabecera + cuerpo, sin controles.
pub fn render_html(
    summary: &SummaryView,
    example: &Option<ExampleView>,
    sim: &SimView,
    res: &ResView,
    generated_at: &str,
) -> String {
    let body = render_body(summary, example, sim, res, generated_at);
    format!(
        "<title>rodalies-sim · dashboard</title>\n{STYLE}\n<div class=\"wrap\">\n  {HEADER}\n{body}\n</div>\n"
    )
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
) -> String {
    let body = render_body(summary, example, sim, res, generated_at);
    let ctrls = controls_html(lines, controls);
    let optp = optimizer_panel();
    format!(
        "<title>rodalies-sim · dashboard interactiu</title>\n{STYLE}\n<div class=\"wrap\">\n  {HEADER}\n  {ctrls}\n  {optp}\n  <div id=\"dashboard\">\n{body}\n  </div>\n</div>\n{SCRIPT}\n{OPT_SCRIPT}\n"
    )
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
        ("Vies / andanes", format!("{}", summary.n_stops), "nodes del graf"),
        ("Cantons", format!("{}", summary.n_edges), "seccions de via"),
        ("Serveis de tren", format!("{}", summary.n_services), "circulacions GTFS"),
        ("Línies", format!("{}", summary.n_routes), "R1, R2N, R4…"),
        ("Temps de càrrega", format!("{:.0} ms", summary.load_ms), "GTFS → graf"),
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
        "<p class=\"muted\" style=\"margin-top:14px\">Andanes amb <code>parent_station</code>: {} · cantó més ràpid: {}</p>",
        summary.with_parent, fastest
    ));

    // --- Ruta d'exemple ---
    let example_html = match example {
        None => "<p class=\"muted\">No hi ha serveis per mostrar.</p>".to_string(),
        Some(ex) => {
            let title = if ex.found {
                format!("Ruta del tren {}", esc(&ex.wanted))
            } else {
                format!(
                    "Tren {} no trobat → servei equivalent {} ({})",
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
                "<h3>{title}</h3><p class=\"muted\">Línia {} · route_id {} · trip_id {} · {} parades{}</p>\
                 <div class=\"scroll\"><table><thead><tr><th>seq</th><th>Estació</th><th>arribada</th><th>sortida</th><th>marxa</th><th>via</th></tr></thead><tbody>{rows}</tbody></table></div>",
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
        inc_html = "<li class=\"muted\">Cap incidència injectada.</li>".into();
    }

    // --- Log CTC ---
    let mut log_html = String::new();
    for e in &sim.events {
        let (cls, badge) = if e.kind == "INCIDÈNCIA" {
            ("ev-inc", "⛔")
        } else if e.kind == "ARRIBA" {
            ("ev-arr", "▼")
        } else if e.kind.starts_with("SURT") {
            ("ev-dep", "▲")
        } else {
            ("", "·")
        };
        let late = if e.delay >= 120 { " late" } else if e.delay > 0 { " warn" } else { "" };
        if e.kind == "INCIDÈNCIA" {
            log_html.push_str(&format!(
                "<div class=\"ev {cls}\"><span class=\"ev-badge\">{badge}</span><span class=\"ev-txt\">{}</span></div>",
                esc(&e.station)
            ));
        } else {
            log_html.push_str(&format!(
                "<div class=\"ev {cls}{late}\"><span class=\"ev-time mono\">{}</span><span class=\"ev-badge\">{badge}</span><span class=\"ev-kind\">{}</span><span class=\"ev-train\">{} <em>{}</em></span><span class=\"ev-sta\">{}</span><span class=\"ev-track mono\">via {}</span><span class=\"ev-delay mono\">{:+} s</span></div>",
                fmt_hms(e.time), esc(&e.kind), esc(&e.train), esc(&e.line), esc(&e.station), esc(&e.track), e.delay
            ));
        }
    }

    // --- Mètriques ---
    let recovery = sim.recovery.clone().unwrap_or_else(|| "no assolit".into());
    let metrics = [
        ("Trens en la finestra", format!("{}", sim.trains_run)),
        ("Arribades processades", format!("{}", sim.arrivals)),
        ("Retencions (senyalització)", format!("{}", sim.held)),
        ("Pic de retard acumulat", format!("{} s", sim.peak_total)),
        ("Hora del pic", sim.peak_time.clone()),
        ("Màx. trens retardats alhora", format!("{}", sim.peak_delayed)),
        ("Retard mitjà/tren al pic", format!("{:.0} s", sim.peak_mean)),
        ("Retorn a l'equilibri", recovery),
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
            None => "no recupera".into(),
        };
        let pct = (r.peak as f64 / max_peak) * 100.0;
        res_rows.push_str(&format!(
            "<tr><td class=\"mono\">+{} min</td><td class=\"num\">{}</td><td><span class=\"bar-track sm\"><span class=\"bar-fill\" style=\"width:{:.1}%\"></span></span></td><td class=\"num\">{}</td><td>{}</td><td class=\"num\">{}</td></tr>",
            r.mins, r.peak, pct, r.delayed, esc(&recov), r.held
        ));
    }

    let busiest = sim
        .busiest
        .as_ref()
        .map(|(a, b, c)| format!("{} → {} ({} circulacions)", esc(a), esc(b), c))
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
      <h2>Serveis per línia</h2>
      {lines_html}
    </section>
    <section class="card">
      <h2>Simulació · finestra {window}</h2>
      <p class="muted">service_id dominant <b>{service_id}</b> · cantó més carregat: {busiest}</p>
      <div class="metrics">{metrics_html}</div>
      <div class="chips">{key_stations}</div>
      <h3 style="margin-top:18px">Incidències injectades</h3>
      <ul class="inc">{inc_html}</ul>
    </section>
  </div>

  <section class="card">
    <h2>Mapa de la xarxa · trens en circulació</h2>
    <p class="muted">Estacions i cantons projectats des de lat/lon del GTFS; els punts de color són trens movent-se per la seva ruta real (bucle de la finestra simulada).</p>
    {map_svg}
  </section>

  <section class="card">
    <h2>Retard acumulat de la xarxa</h2>
    {chart}
  </section>

  <section class="card">
    <h2>Log CTC · entrades/sortides a trams clau</h2>
    <div class="log">{log_html}</div>
  </section>

  <section class="card">
    <h2>Anàlisi de resiliència</h2>
    <p class="muted">Escenari: bloqueig del cantó {segment}. Cada fila és una durada de bloqueig (Monte Carlo, rayon).</p>
    <div class="scroll"><table>
      <thead><tr><th>bloqueig</th><th>pic acumulat</th><th></th><th>trens ret.</th><th>recuperació</th><th>retencions</th></tr></thead>
      <tbody>{res_rows}</tbody>
    </table></div>
  </section>

  <section class="card">
    <h2>Ruta d'exemple</h2>
    {example_html}
  </section>

  <footer>Generat el {generated_at} · rodalies-sim · dades GTFS Rodalies/Cercanías</footer>
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
        log_html = log_html,
        segment = esc(&res.segment),
        res_rows = res_rows,
        example_html = example_html,
        generated_at = esc(generated_at),
    )
}
