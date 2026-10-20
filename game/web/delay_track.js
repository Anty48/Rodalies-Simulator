// ============================================================================================
// SEGUIMIENTO DE RETRASOS — módulo compartido por /numeros (varias runs) y la pantalla final del
// simulador visual. Sin dependencias: SVG a mano + MathML nativo (tipografía matemática tipo LaTeX,
// sin librerías ni red) + su propia hoja de estilo (inyectada una vez), que lee las variables de
// tema de la página que lo carga (report.rs claro/oscuro o el juego).
//
//   DelayTrack.open(run, {title})          → modal con los dos gráficos detallados (retraso medio
//                                            por tren y retraso acumulado de la red), «Analizar
//                                            gráficos» y «Descargar gráficos y datos» (ZIP).
//   DelayTrack.openIncidents(run, {title}) → incidencias del día ordenadas por gravedad REAL.
//
// `run` = lo que devuelve el servidor por jornada: { track:{t0,dt,mean,total,active},
// analysis:{mean:[…]}, incidents:[…] }. El análisis (picos, ajustes, R², estabilidad, huella,
// integrales) lo calcula el motor en Rust (src/recovery.rs); aquí solo se dibuja y se exporta.
// ============================================================================================
(function () {
  "use strict";

  const MATH_FONT = `"Latin Modern Math","STIX Two Math","Cambria Math","Times New Roman",serif`;
  const CSS = `
.dtk-ov{--dt-bg:var(--panel,#12151d);--dt-bg2:var(--panel2,rgba(255,255,255,.04));--dt-bd:var(--border,rgba(255,255,255,.1));
  --dt-tx:var(--text,#eef0f6);--dt-mu:var(--muted,#9aa1b2);--dt-ac:var(--accent,#ec6e15);--dt-grid:rgba(128,128,128,.16);
  --dt-inc:226,73,72;--dt-seg:rgba(236,110,21,.13);--dt-base:#8a8f9c;
  --dt-ink:#b9bfcc;--f-exp:#3987e5;--f-parab:#d95926;--f-lin:#199e70;
  position:fixed;inset:0;z-index:1200;background:rgba(5,7,11,.62);-webkit-backdrop-filter:blur(5px);backdrop-filter:blur(5px);
  display:flex;align-items:flex-start;justify-content:center;overflow-y:auto;padding:28px 16px;font-family:var(--sans,system-ui,sans-serif);color:var(--dt-tx)}
.dtk-ov.light{--dt-ink:#5d6170;--f-exp:#2a78d6;--f-parab:#eb6834;--f-lin:#1baf7a;--dt-inc:197,34,31;--dt-seg:rgba(236,110,21,.10);--dt-base:#7b7f8a}
.dtk{width:min(1080px,100%);background:var(--dt-bg);border:1px solid var(--dt-bd);border-top:3px solid var(--dt-ac);border-radius:16px;
  box-shadow:0 24px 70px rgba(0,0,0,.35);padding:22px 24px 26px;position:relative}
.dtk math{font-family:${MATH_FONT};font-size:1.1em}
.dtk-hd{display:flex;align-items:flex-start;gap:12px;margin-bottom:14px;flex-wrap:wrap}
.dtk-kick{font-size:.7rem;font-weight:700;letter-spacing:.08em;text-transform:uppercase;color:var(--dt-ac)}
.dtk h2{margin:2px 0 3px;font-family:var(--display,inherit);font-size:1.3rem}
.dtk-sub{color:var(--dt-mu);font-size:.85rem}
.dtk-hbtns{margin-left:auto;display:flex;gap:8px;align-items:center}
.dtk-x{flex:0 0 auto;width:36px;height:36px;border-radius:9px;border:1px solid var(--dt-bd);background:var(--dt-bg2);color:var(--dt-tx);font-size:20px;line-height:1;cursor:pointer}
.dtk-x:hover{border-color:var(--dt-ac)}
.dtk-dl{display:inline-flex;align-items:center;gap:7px;height:36px;padding:0 13px;border-radius:9px;border:1px solid var(--dt-bd);background:var(--dt-bg2);color:var(--dt-tx);font:600 .84rem var(--sans,inherit);cursor:pointer}
.dtk-dl:hover:not(:disabled){border-color:var(--dt-ac);color:var(--dt-ac)}
.dtk-dl:disabled{opacity:.55;cursor:progress}
.dtk-dl svg{width:15px;height:15px}
.dtk-card{border:1px solid var(--dt-bd);border-radius:12px;padding:14px 16px 10px;margin-top:14px;background:var(--dt-bg2)}
.dtk-card h3{margin:0;font-family:var(--display,inherit);font-size:1rem}
.dtk-note{margin:2px 0 8px;color:var(--dt-mu);font-size:.8rem;line-height:1.5}
.dtk-kpis{display:flex;gap:18px;flex-wrap:wrap;margin:4px 0 6px;font-size:.8rem;color:var(--dt-mu);align-items:baseline}
.dtk-kpis math{color:var(--dt-tx);font-size:1.15em}
.dtk-chart{position:relative;width:100%;user-select:none;touch-action:pan-y}
.dtk-chart svg{display:block;width:100%;overflow:visible}
.dtk-chart .ax{font:12.5px ${MATH_FONT};fill:var(--dt-mu)}
.dtk-chart .gr{stroke:var(--dt-grid);stroke-width:1}
.dtk-chart .base{stroke:var(--dt-bd);stroke-width:1}
.dtk-chart .xh{stroke:var(--dt-tx);stroke-width:1;stroke-dasharray:3 3;opacity:.55}
.dtk-chart .bandlbl{font:600 10px var(--sans,sans-serif);fill:var(--dt-mu)}
.dtk-chart .inclbl{font:700 10px var(--sans,sans-serif)}
.dtk-tip{position:absolute;pointer-events:none;z-index:3;min-width:160px;max-width:280px;padding:8px 10px;border-radius:9px;background:var(--dt-bg);
  border:1px solid var(--dt-bd);box-shadow:0 10px 26px rgba(0,0,0,.25);font-size:12px;line-height:1.5}
.dtk-tip .tx{font-weight:700;margin-bottom:3px}
.dtk-tip .row{display:flex;align-items:center;gap:7px}
.dtk-tip .row b{margin-left:auto;padding-left:10px;font-weight:400}
.dtk-tip .inc{color:var(--dt-mu);border-top:1px dashed var(--dt-bd);margin-top:4px;padding-top:4px}
.dtk-tip .inc b{color:var(--dt-tx)}
.dtk-sw{display:inline-block;width:16px;height:0;border-top:2px solid currentColor;flex:0 0 auto}
.dtk-sw.dot{width:8px;height:8px;border:none;border-radius:50%;background:currentColor}
.dtk-sw.band{width:12px;height:10px;border:none;border-radius:2px}
.dtk-legend{display:flex;gap:14px;flex-wrap:wrap;font-size:.78rem;color:var(--dt-mu);margin:4px 0 2px}
.dtk-legend span{display:inline-flex;align-items:center;gap:6px}
.dtk-actions{display:flex;justify-content:center;margin-top:18px}
.dtk-btn{display:inline-flex;align-items:center;gap:8px;font:600 .95rem var(--sans,inherit);padding:11px 22px;border-radius:9px;border:none;cursor:pointer;color:#fff;
  background:var(--grad-cta,var(--rod-grad-cta,linear-gradient(180deg,#ff8a3d,#ec6e15)));box-shadow:0 3px 14px rgba(236,110,21,.3)}
.dtk-btn:hover{filter:brightness(1.06)}
.dtk-btn svg{width:16px;height:16px}
.dtk-actions{gap:10px;flex-wrap:wrap}
.dtk-btn.alt{background:var(--dt-bg2);color:var(--dt-tx);border:1px solid var(--dt-ac);box-shadow:none}
.dtk-modes{display:flex;gap:10px;flex-wrap:wrap;margin:6px 0 2px}
.dtk-pick{display:flex;align-items:center;gap:12px;flex-wrap:wrap;margin:2px 0 8px;padding:8px 12px;border-radius:9px;border:1px dashed var(--dt-ac);font-size:.84rem}
.dtk-pick .err{color:rgb(var(--dt-inc));font-weight:600}
.dtk-lnk{margin-left:auto;background:none;border:1px solid var(--dt-bd);border-radius:7px;padding:3px 9px;font:600 .75rem var(--sans,inherit);color:var(--dt-mu);cursor:pointer}
.dtk-lnk:hover{border-color:rgb(var(--dt-inc));color:rgb(var(--dt-inc))}
.dtk-seg{display:inline-flex;border:1px solid var(--dt-bd);border-radius:9px;overflow:hidden;margin:2px 0 4px}
.dtk-seg button{font:600 .82rem var(--sans,inherit);padding:7px 14px;border:none;background:none;color:var(--dt-mu);cursor:pointer}
.dtk-seg button.on{background:var(--dt-ac);color:#fff}
.dtk-sum{display:grid;grid-template-columns:repeat(auto-fit,minmax(170px,1fr));gap:10px;margin:10px 0 2px}
.dtk-sum>div{border:1px solid var(--dt-bd);border-radius:10px;padding:9px 12px;background:var(--dt-bg)}
.dtk-sum .lb{display:block;font-size:.7rem;font-weight:700;letter-spacing:.05em;text-transform:uppercase;color:var(--dt-mu)}
.dtk-sum .lb math{text-transform:none;font-size:1.1em}
.dtk-sum .vl{font-size:1.05rem;font-weight:700}
.dtk-sum .vl math{font-size:1.2em;font-weight:400}
.dtk-peaks{display:flex;flex-direction:column;gap:14px;margin-top:14px}
@media(max-width:560px){.dtk{padding:18px 14px}}
.dtk-tblw{overflow-x:auto}
.dtk-pk h4{margin:0;font-family:var(--display,inherit);font-size:.95rem;display:flex;gap:8px;align-items:baseline;flex-wrap:wrap}
.dtk-pk h4 .n{display:inline-flex;align-items:center;justify-content:center;min-width:22px;height:22px;padding:0 6px;box-sizing:border-box;border-radius:11px;background:var(--dt-ac);color:#fff;font-size:.75rem;align-self:center}
.dtk-pk h4 small{color:var(--dt-mu);font-weight:500;font-family:var(--sans,inherit)}
.dtk-tbl{width:100%;border-collapse:collapse;font-size:12.5px;margin-top:6px}
.dtk-tbl th,.dtk-tbl td{padding:6px 7px;border-bottom:1px solid var(--dt-bd);text-align:left;vertical-align:top}
.dtk-tbl th{font-size:.68rem;text-transform:uppercase;letter-spacing:.04em;color:var(--dt-mu);vertical-align:bottom}
.dtk-tbl th math{text-transform:none}
.dtk-tbl td.num{text-align:right;white-space:nowrap}
.dtk-tbl th.num{text-align:right}
.dtk-tbl tr.best td{background:rgba(236,110,21,.08)}
.dtk-tbl tr.best td:first-child{box-shadow:inset 3px 0 0 var(--dt-ac)}
.dtk-tbl .mm{display:flex;align-items:center;gap:7px;white-space:nowrap}
.dtk-tbl small{color:var(--dt-mu)}
.dtk-tag{display:inline-block;margin-left:5px;font-size:.62rem;font-weight:700;letter-spacing:.05em;text-transform:uppercase;padding:1px 6px;border-radius:5px;background:var(--dt-ac);color:#fff}
.dtk-empty{color:var(--dt-mu);font-style:italic;padding:14px 0}
.dtk-expl{margin-top:16px;font-size:.8rem;line-height:1.6;color:var(--dt-mu);border-left:3px solid var(--dt-bd);padding:4px 0 4px 12px}
.dtk-expl b{color:var(--dt-tx)}
.dtk-expl p{margin:0 0 6px}
.dtk-int{display:grid;grid-template-columns:repeat(auto-fit,minmax(240px,1fr));gap:10px;margin:8px 0 4px}
.dtk-int>div{border:1px solid var(--dt-bd);border-radius:10px;padding:8px 11px;background:var(--dt-bg);overflow-x:auto}
.dtk-int math{font-size:1em}
.dtk-int span{display:block;font-size:.7rem;font-weight:700;letter-spacing:.04em;text-transform:uppercase;color:var(--dt-mu)}
.dtk-inc{list-style:none;margin:10px 0 0;padding:0;display:flex;flex-direction:column;gap:8px}
.dtk-inc li{display:grid;grid-template-columns:34px minmax(0,1fr) 200px;gap:12px;align-items:center;border:1px solid var(--dt-bd);border-radius:10px;padding:10px 12px;background:var(--dt-bg2)}
@media(max-width:640px){.dtk-inc li{grid-template-columns:30px minmax(0,1fr)}.dtk-inc .imp{grid-column:2}}
.dtk-inc .rk{font-weight:700;color:var(--dt-mu);text-align:center}
.dtk-inc .mt{font-weight:700}
.dtk-inc .mt small{font-weight:500;color:var(--dt-mu);margin-left:6px}
.dtk-inc .wh{font-size:.8rem;color:var(--dt-mu);margin-top:2px}
.dtk-inc .imp{font-size:.78rem;color:var(--dt-mu)}
.dtk-inc .bar{height:6px;border-radius:3px;background:var(--dt-bd);margin-top:5px;overflow:hidden}
.dtk-inc .bar i{display:block;height:100%;border-radius:3px;background:rgb(var(--dt-inc))}
.dtk-chips{display:flex;flex-wrap:wrap;gap:6px;margin:8px 0 6px;align-items:center}
.dtk-chip{display:inline-flex;align-items:center;gap:6px;font:700 .76rem var(--sans,inherit);padding:4px 10px;border-radius:999px;border:1.5px solid var(--lc);background:none;color:var(--dt-tx);cursor:pointer;opacity:.45}
.dtk-chip i{width:9px;height:9px;border-radius:50%;background:var(--lc)}
.dtk-chip.on{opacity:1;background:color-mix(in srgb,var(--lc) 16%,transparent)}
.dtk-mini{display:grid;grid-template-columns:repeat(auto-fill,minmax(300px,1fr));gap:10px;margin-top:6px}
.dtk-mini>div{border:1px solid var(--dt-bd);border-left:4px solid var(--lc);border-radius:10px;padding:8px 10px 4px;background:var(--dt-bg);min-width:0}
.dtk-mini h4{margin:0 0 2px;font-size:.88rem;display:flex;gap:8px;align-items:baseline;flex-wrap:wrap}
.dtk-mini h4 small{font-weight:500;color:var(--dt-mu);font-size:.74rem}
`;
  let cssDone = false;
  function injectCss() {
    if (cssDone) return;
    const st = document.createElement("style");
    st.textContent = CSS;
    document.head.appendChild(st);
    cssDone = true;
  }

  const EFF = { red_one: "señal roja (1 sentido)", red_both: "señal roja (2 sentidos)", single: "vía única", aviso: "aviso publicado (día real)" };
  // Peso NOMINAL del efecto (solo desempate de gravedad cuando el impacto medido coincide): cortar
  // los dos sentidos es peor que dejar vía única, y esta peor que un rojo en un sentido.
  const EFF_W = { red_both: 3, single: 2, red_one: 1 };
  const MODEL = {
    exp: { name: "Exponencial negativa", short: "Exponencial", color: "var(--f-exp)", dash: "" },
    parab: { name: "Parábola", short: "Parábola", color: "var(--f-parab)", dash: "7 4" },
    lin: { name: "Lineal", short: "Lineal", color: "var(--f-lin)", dash: "2 3" },
  };
  const esc = (x) => String(x == null ? "" : x).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  const fmt = (v, d = 1) => (Number.isFinite(v) ? v.toLocaleString((window.LOCALE?LOCALE():"es-ES"), { minimumFractionDigits: d, maximumFractionDigits: d }) : "–");
  const hhmm = (s) => {
    s = Math.max(0, Math.round(s));
    return String(Math.floor(s / 3600) % 24).padStart(2, "0") + ":" + String(Math.floor((s % 3600) / 60)).padStart(2, "0");
  };

  // ---- Tipografía matemática (MathML nativo = LaTeX sin dependencias) ----------------------
  // Todos los números del seguimiento se componen como matemáticas: signo menos real (−), coma
  // decimal española, potencias de 10 en notación científica y unidades en texto recto.
  const M = (inner) => `<math>${inner}</math>`;
  const mn = (v, d) => (v < 0 ? `<mo>−</mo><mn>${fmt(-v, d)}</mn>` : `<mn>${fmt(v, d)}</mn>`);
  const unit = (u) => (u ? `<mspace width="0.22em"/><mtext>${u}</mtext>` : "");
  // Número con cifras significativas; |v| < 0,01 → mantisa·10ⁿ.
  function sci(v, sigd = 3) {
    if (!Number.isFinite(v)) return "<mtext>–</mtext>";
    if (v === 0) return "<mn>0</mn>";
    const a = Math.abs(v), e = Math.floor(Math.log10(a)), neg = v < 0 ? "<mo>−</mo>" : "";
    if (e < -2 || e >= 5) {
      const m = a / Math.pow(10, e);
      return `${neg}<mn>${fmt(m, sigd - 1)}</mn><mo>·</mo><msup><mn>10</mn><mrow>${e < 0 ? "<mo>−</mo>" : ""}<mn>${Math.abs(e)}</mn></mrow></msup>`;
    }
    return neg + `<mn>${fmt(a, Math.max(0, sigd - 1 - e))}</mn>`;
  }
  const num = (v, d, u) => M((Number.isFinite(v) ? mn(v, d) : "<mtext>–</mtext>") + unit(u));
  const signed = (v, d, u) => M((Number.isFinite(v) ? (v >= 0 ? "<mo>+</mo>" : "") + mn(v, d) : "<mtext>–</mtext>") + unit(u));
  // Término con su signo para encadenar en una ecuación: "+ 0,31" / "− 0,31".
  const term = (v, inner) => (v < 0 ? "<mo>−</mo>" : "<mo>+</mo>") + inner(Math.abs(v));
  const durM = (m) => (m == null || !Number.isFinite(m) ? M("<mtext>–</mtext>") : m >= 90 ? num(m / 60, 1, "h") : num(Math.round(m), 0, "min"));
  const R2 = "<msup><mi>R</mi><mn>2</mn></msup>";
  const timeM = (s) => M(`<mn>${hhmm(s)}</mn>`);

  // "Techo bonito" del eje Y para ticks legibles.
  function niceMax(v) {
    if (!(v > 0)) return 1;
    const p = Math.pow(10, Math.floor(Math.log10(v)));
    for (const m of [1, 1.2, 1.5, 2, 2.5, 3, 4, 5, 6, 8, 10]) if (m * p >= v) return m * p;
    return 10 * p;
  }

  // ---- Gráfico de líneas con cursor tipo calculadora gráfica ------------------------------
  // cfg = { xs:[…] (eje X numérico, uniforme), xfmt(x), xticks(x0,x1), height, ydec, yunitShort,
  //         series:[{name, color, ys:[…] | fn(x), dash, width, dots, area}],
  //         bands:[{x0,x1,fill,label,labelPos,edge}], hlines:[{y,label,color}], tipExtra(x)->html,
  //         static (sin ResizeObserver: para exportar) }
  // El cursor se engancha al punto de datos más cercano en X y muestra el valor EXACTO de cada
  // serie en ese punto (como el TRACE de una calculadora gráfica), con un punto sobre cada curva.
  function lineChart(el, cfg) {
    el.classList.add("dtk-chart");
    const draw = () => {
      const W = Math.max(280, el.clientWidth || 600), H = cfg.height || 230;
      const padL = 52, padR = 12, padT = 12, padB = 26, iw = W - padL - padR, ih = H - padT - padB;
      const xs = cfg.xs, n = xs.length;
      const x0 = xs[0], x1 = xs[n - 1] > x0 ? xs[n - 1] : x0 + 1;
      const val = (s, i) => (s.fn ? s.fn(xs[i]) : s.ys[i]);
      let ymax = 0;
      for (const s of cfg.series) for (let i = 0; i < n; i++) { const v = val(s, i); if (Number.isFinite(v) && v > ymax) ymax = v; }
      for (const h of cfg.hlines || []) if (h.y > ymax) ymax = h.y;
      ymax = niceMax(ymax * 1.04);
      const X = (x) => padL + ((x - x0) / (x1 - x0)) * iw;
      const Y = (v) => padT + ih - (Math.max(0, Math.min(v, ymax)) / ymax) * ih;
      const ydec = ymax < 0.05 ? 3 : ymax < 4 ? 2 : ymax < 20 ? 1 : 0;
      let g = `<rect class="bg" x="0" y="0" width="${W}" height="${H}" fill="none"/>`;
      // Bandas (incidencias / tramos de pico) detrás de todo. `edge` = filo superior grueso (las
      // incidencias graves), para que destaquen aunque haya muchas pequeñas alrededor.
      for (const b of cfg.bands || []) {
        if (b.x0 > x1 || b.x1 < x0) continue;
        const a = X(Math.max(x0, b.x0)), c = X(Math.min(x1, b.x1)), w = Math.max(1.5, c - a);
        g += `<rect x="${a.toFixed(1)}" y="${padT}" width="${w.toFixed(1)}" height="${ih}" fill="${b.fill}"/>`;
        if (b.edge) g += `<rect x="${a.toFixed(1)}" y="${padT}" width="${w.toFixed(1)}" height="3" fill="${b.edge}"/>`;
        if (b.label) {
          const bottom = b.labelPos === "bottom";
          g += `<text class="${bottom ? "inclbl" : "bandlbl"}" x="${((a + c) / 2).toFixed(1)}" y="${bottom ? padT + ih - 4 : padT + 11}" text-anchor="middle"${b.edge ? ` fill="${b.edge}"` : ""}>${esc(b.label)}</text>`;
        }
      }
      // Selección manual: marca del inicio ya elegido + rectángulo de previsualización hasta el cursor.
      if (cfg.onPick && cfg.pickFrom != null) {
        const xp = X(xs[cfg.pickFrom]).toFixed(1);
        g += `<rect class="pv" x="${xp}" y="${padT}" width="0" height="${ih}" fill="var(--dt-ac)" opacity=".16"/>`;
        g += `<line x1="${xp}" x2="${xp}" y1="${padT}" y2="${padT + ih}" stroke="var(--dt-ac)" stroke-width="2"/>`;
        g += `<text class="bandlbl" x="${xp}" y="${padT + 11}" dx="4" style="fill:var(--dt-ac)">inicio</text>`;
      }
      for (let k = 0; k <= 4; k++) {
        const v = (ymax * k) / 4, y = Y(v).toFixed(1);
        g += `<line class="${k ? "gr" : "base"}" x1="${padL}" x2="${W - padR}" y1="${y}" y2="${y}"/>`;
        g += `<text class="ax" x="${padL - 6}" y="${+y + 4}" text-anchor="end">${fmt(v, ydec)}</text>`;
      }
      for (const t of cfg.xticks ? cfg.xticks(x0, x1, iw) : []) {
        const x = X(t.x).toFixed(1);
        g += `<line class="gr" x1="${x}" x2="${x}" y1="${padT}" y2="${padT + ih}"/><text class="ax" x="${x}" y="${H - 7}" text-anchor="middle">${esc(t.label)}</text>`;
      }
      for (const h of cfg.hlines || []) {
        const y = Y(h.y).toFixed(1), col = h.color || "var(--dt-base)";
        g += `<line x1="${padL}" x2="${W - padR}" y1="${y}" y2="${y}" stroke="${col}" stroke-width="1.3" stroke-dasharray="6 4"/>`;
      }
      // Series: línea (ys submuestreada a ~1 punto por píxel) o curva analítica (fn, muestreo fino).
      for (const s of cfg.series) {
        if (s.dots) {
          const r = n > 120 ? 1.7 : 2.7;
          for (let i = 0; i < n; i++) g += `<circle cx="${X(xs[i]).toFixed(1)}" cy="${Y(val(s, i)).toFixed(1)}" r="${r}" fill="${s.color}" opacity=".8"/>`;
          continue;
        }
        let d = "";
        if (s.fn) {
          const m = Math.max(40, Math.round(iw / 3));
          for (let k = 0; k <= m; k++) { const x = x0 + ((x1 - x0) * k) / m; d += (k ? "L" : "M") + X(x).toFixed(1) + "," + Y(s.fn(x)).toFixed(1); }
        } else {
          const step = Math.max(1, Math.floor(n / iw));
          for (let i = 0; i < n; i += step) d += (d ? "L" : "M") + X(xs[i]).toFixed(1) + "," + Y(s.ys[i]).toFixed(1);
          d += "L" + X(xs[n - 1]).toFixed(1) + "," + Y(s.ys[n - 1]).toFixed(1);
          if (s.area) g += `<path d="${d}L${X(xs[n - 1]).toFixed(1)},${Y(0)}L${X(xs[0]).toFixed(1)},${Y(0)}Z" fill="${s.color}" opacity=".12"/>`;
        }
        g += `<path d="${d}" fill="none" stroke="${s.color}" stroke-width="${s.width || 2}" stroke-linejoin="round" stroke-linecap="round"${s.dash ? ` stroke-dasharray="${s.dash}"` : ""}/>`;
      }
      el.innerHTML = `<svg viewBox="0 0 ${W} ${H}" width="${W}" height="${H}" role="img" aria-label="${esc(cfg.label || "")}">${g}
        <g class="cur" visibility="hidden"><line class="xh" y1="${padT}" y2="${padT + ih}"/>${cfg.series.map((s) => `<circle r="4.5" fill="${s.color}" stroke="var(--dt-bg)" stroke-width="2"/>`).join("")}</g>
        <rect class="hit" x="${padL}" y="${padT}" width="${iw}" height="${ih}" fill="transparent" style="cursor:crosshair"/></svg><div class="dtk-tip" hidden></div>`;
      if (cfg.static) return;
      const svg = el.firstElementChild, cur = svg.querySelector(".cur"), hit = svg.querySelector(".hit"), tip = el.lastElementChild;
      const line = cur.firstElementChild, dots = [...cur.querySelectorAll("circle")];
      const dec = cfg.ydec ?? 2;
      // Índice del punto de datos más cercano al puntero (eje uniforme → O(1)).
      const idxAt = (ev) => {
        const r = svg.getBoundingClientRect();
        const xv = x0 + ((((ev.clientX - r.left) / r.width) * W - padL) / iw) * (x1 - x0);
        return Math.max(0, Math.min(n - 1, Math.round(((xv - x0) / (x1 - x0)) * (n - 1))));
      };
      const pv = svg.querySelector(".pv");
      const move = (ev) => {
        const r = svg.getBoundingClientRect();
        const i = idxAt(ev);
        const cx = X(xs[i]);
        if (pv) { const xp = X(xs[cfg.pickFrom]); pv.setAttribute("x", Math.min(xp, cx)); pv.setAttribute("width", Math.abs(cx - xp)); }
        line.setAttribute("x1", cx); line.setAttribute("x2", cx);
        let rows = "";
        cfg.series.forEach((s, k) => {
          const v = val(s, i);
          dots[k].setAttribute("cx", cx); dots[k].setAttribute("cy", Y(v));
          rows += `<div class="row"><span class="dtk-sw${s.dots ? " dot" : ""}" style="color:${s.color}${s.dash ? ";border-top-style:dashed" : ""}"></span>${esc(s.name)}<b>${num(v, dec, cfg.yunitShort)}</b></div>`;
        });
        const hint = cfg.onPick ? `<div class="inc"><b>Clic</b>: marcar ${cfg.pickFrom == null ? "el inicio" : "el final"} del tramo</div>` : "";
        tip.innerHTML = `<div class="tx">${cfg.xfmt ? cfg.xfmt(xs[i]) : M(`<mn>${xs[i]}</mn>`)}</div>${rows}${cfg.tipExtra ? cfg.tipExtra(xs[i]) : ""}${hint}`;
        tip.hidden = false; cur.setAttribute("visibility", "visible");
        // Tooltip al lado del cursor, girando al otro lado cerca del borde derecho.
        const scale = r.width / W, left = cx * scale, tw = tip.offsetWidth;
        tip.style.left = (left + 14 + tw > r.width ? Math.max(0, left - 14 - tw) : left + 14) + "px";
        tip.style.top = Math.max(0, Math.min(Y(val(cfg.series[0], i)) * scale - 20, r.height - tip.offsetHeight)) + "px";
      };
      hit.addEventListener("pointermove", move);
      hit.addEventListener("pointerdown", move);
      if (cfg.onPick) hit.addEventListener("click", (ev) => cfg.onPick(idxAt(ev)));
      hit.addEventListener("pointerleave", () => { tip.hidden = true; cur.setAttribute("visibility", "hidden"); });
    };
    draw();
    if (!cfg.static && window.ResizeObserver) {
      let lastW = el.clientWidth;
      new ResizeObserver(() => { if (Math.abs(el.clientWidth - lastW) > 4) { lastW = el.clientWidth; draw(); } }).observe(el);
    }
  }

  // Ticks de hora para ejes de jornada en segundos: el paso (½, 1, 2, 3, 4, 6 h) se elige para que
  // quepan las etiquetas en el ANCHO del gráfico (≥ ~48 px entre ellas; en los gráficos pequeños «Por
  // separado» se pisaban).
  const hourTicks = (x0, x1, iw = 900) => {
    const span = (x1 - x0) / 3600, maxTicks = Math.max(2, Math.floor(iw / 48));
    const stepH = [0.5, 1, 2, 3, 4, 6, 12].find((h) => span / h <= maxTicks) || 12;
    const out = [];
    for (let h = Math.ceil(x0 / 3600 / stepH) * stepH; h * 3600 <= x1; h += stepH) out.push({ x: h * 3600, label: hhmm(h * 3600) });
    return out;
  };
  const relTicks = (a, b) => { const st = b > 90 ? 30 : b > 40 ? 10 : 5, o = []; for (let t = 0; t <= b; t += st) o.push({ x: t, label: "+" + t + "′" }); return o; };

  // ---- Incidencias: gravedad y marcado en los gráficos ------------------------------------
  // Gravedad REAL = subida del retraso acumulado durante la incidencia (+30 min), medida por el
  // motor; desempate por la nominal (efecto × duración).
  const byGravity = (incs) => (incs || []).slice().sort((a, b) =>
    (b.impact_min || 0) - (a.impact_min || 0) ||
    (EFF_W[b.effect] || 0) * (b.t1 - b.t0) - (EFF_W[a.effect] || 0) * (a.t1 - a.t0));
  // Una incidencia es "gorda" si está entre las 5 de más impacto Y su impacto es ≥ 40 % de la peor:
  // esas se pintan con más opacidad, filo superior y su número de gravedad (#1, #2…).
  function incidentMeta(incs) {
    const ranked = byGravity(incs), max = Math.max(0.01, ...ranked.map((i) => i.impact_min || 0));
    const meta = new Map();
    ranked.forEach((i, k) => {
      const rel = (i.impact_min || 0) / max;
      meta.set(i, { rank: k + 1, rel, big: k < 5 && rel >= 0.4 });
    });
    return meta;
  }
  function incidentBands(incs, meta, shift = 0, scale = 1) {
    return (incs || []).map((i) => {
      const m = meta.get(i) || { rel: 0, big: false, rank: 0 };
      const a = m.big ? 0.22 + 0.2 * m.rel : 0.05 + 0.1 * m.rel;
      return {
        x0: (i.t0 - shift) / scale, x1: (i.t1 - shift) / scale,
        fill: `rgba(var(--dt-inc),${a.toFixed(3)})`,
        edge: m.big ? "rgb(var(--dt-inc))" : null,
        label: m.big ? "#" + m.rank : null, labelPos: "bottom",
      };
    });
  }
  // Huecos sin datos de un día REAL (recolector parado o feed caído): la serie vale 0 ahí. Solo se
  // rotulan los de ≥ 20 min (dos huecos cortos seguidos pisaban sus etiquetas).
  const gapBands = (run, shift = 0, scale = 1) => (run.gaps || []).map(([a, b]) => ({ x0: (a - shift) / scale, x1: (b - shift) / scale, fill: "rgba(128,128,128,.2)", label: b - a >= 1200 ? "sin datos" : null }));
  const incTip = (incs, meta, toAbs = (x) => x) => (x) => {
    const xa = toAbs(x), act = (incs || []).filter((i) => xa >= i.t0 && xa <= i.t1);
    return act.length ? `<div class="inc">${act.map((i) => {
      const m = meta.get(i) || {};
      return `${m.big ? "<b>#" + m.rank + "</b> " : ""}${esc(i.motivo)} · ${signed(i.impact_min || 0, 1, "min")}`;
    }).join("<br>")}</div>` : "";
  };

  // Pico de una serie SOLO en horas de servicio (trenes circulando ≥ 25 % del máximo, el mismo
  // criterio que recovery.rs): al abrir y cerrar la jornada, con 3 trenes, la media salta con una
  // sola llegada tarde y no representa a la red. → {i, v}
  function servicePeak(tr, ys) {
    const act = tr.active || [], max = Math.max(0, ...act);
    let best = -1;
    for (let i = 0; i < ys.length; i++) {
      if (act.length && act[i] < 0.25 * max) continue;
      if (best < 0 || ys[i] > ys[best]) best = i;
    }
    return best < 0 ? { i: 0, v: null } : { i: best, v: ys[best] };
  }

  // ---- Modelo de datos del análisis ------------------------------------------------------
  const evalFit = (f) => (t) => f.model === "exp" ? f.p[0] + f.p[1] * Math.exp(-t / f.p[2]) : f.model === "parab" ? f.p[0] * t * t + f.p[1] * t + f.p[2] : f.p[0] * t + f.p[1];
  // Picos ORDENADOS por el R² de su mejor modelo (los mejor explicados arriba; los que menos
  // interesan, abajo). El número de cada pico = su puesto en este orden (también en la vista general).
  function rankedPeaks(run, mode = "auto") {
    const manual = mode === "manual";
    const src = manual ? run._manual || [] : (run.analysis && run.analysis.mean) || [];
    const ps = src.map((p) => ({ ...p, src: p, manual, bestFit: p.fits[p.best] }));
    ps.sort((a, b) => b.bestFit.r2 - a.bestFit.r2);
    ps.forEach((p, k) => { p.rank = k + 1; p.label = (manual ? "M" : "") + (k + 1); });
    return ps;
  }
  // Primitivas por trapecios (paso 1 min) de lo observado y lo predicho, en tren·min.
  function integrals(run, p) {
    const tr = run.track, f = evalFit(p.bestFit), cd = [0], cm = [0], cb = [0];
    for (let i = p.i_peak + 1; i <= p.i_end; i++) {
      const t = i - p.i_peak, na = tr.active[i - 1], nb = tr.active[i];
      cd.push(cd[cd.length - 1] + 0.5 * (tr.total[i - 1] + tr.total[i]));
      cm.push(cm[cm.length - 1] + 0.5 * (f(t - 1) * na + f(t) * nb));
      cb.push(cb[cb.length - 1] + 0.5 * p.pre_mean * (na + nb));
    }
    return { cd, cm, cb };
  }

  // ---- Configuración de cada gráfico (misma para la vista y para la exportación) ----------
  function cfgMean(run, meta, extra) {
    const tr = run.track, xs = tr.mean.map((_, i) => tr.t0 + i * tr.dt), incs = run.incidents || [];
    const tipTrains = (x) => {
      const i = Math.round((x - tr.t0) / tr.dt);
      return `<div class="row" style="color:var(--dt-mu)">Trenes en servicio<b>${num(tr.active[i] ?? NaN, 0)}</b></div>` + incTip(incs, meta)(x);
    };
    return { xs, xfmt: timeM, xticks: hourTicks, ydec: 2, yunitShort: "min", label: "Retraso medio por tren", bands: gapBands(run).concat(incidentBands(incs, meta)), tipExtra: tipTrains,
      series: [{ name: "Retraso medio", ys: tr.mean, color: "var(--dt-ac)", area: true }], ...extra };
  }
  function cfgTotal(run, meta, extra) {
    const tr = run.track, xs = tr.total.map((_, i) => tr.t0 + i * tr.dt), incs = run.incidents || [];
    return { xs, xfmt: timeM, xticks: hourTicks, ydec: 1, yunitShort: "min", label: "Retraso acumulado de la red", bands: gapBands(run).concat(incidentBands(incs, meta)), tipExtra: incTip(incs, meta),
      series: [{ name: "Retraso acumulado", ys: tr.total, color: "var(--dt-ac)", area: true }], ...extra };
  }
  function cfgOverview(run, meta, peaks, serie, extra) {
    const tr = run.track, xs = tr.mean.map((_, i) => tr.t0 + i * tr.dt), incs = run.incidents || [];
    const bands = peaks.map((p) => ({ x0: xs[p.i_peak], x1: xs[p.i_end], fill: "var(--dt-seg)", label: p.label }));
    return { xs, xfmt: timeM, xticks: hourTicks, height: 200, ydec: serie === "mean" ? 2 : 1, yunitShort: "min", label: "Tramos analizados",
      bands: bands.concat(incidentBands(incs, meta)), tipExtra: incTip(incs, meta),
      series: [{ name: serie === "mean" ? "Retraso medio" : "Retraso acumulado", ys: tr[serie], color: "var(--dt-ac)", width: 1.6 }], ...extra };
  }
  function cfgPeak(run, meta, p, extra) {
    const tr = run.track, incs = run.incidents || [];
    const seg = tr.mean.slice(p.i_peak, p.i_end + 1), ts = seg.map((_, i) => i), tPeak = tr.t0 + p.i_peak * tr.dt;
    const lvl = p.bestFit.new_level;
    return {
      xs: ts, height: 220, ydec: 2, yunitShort: "min", label: "Ajustes del pico " + p.label,
      xfmt: (t) => M(`<mi>t</mi><mo>=</mo><mo>+</mo><mn>${t}</mn><mspace width="0.2em"/><mtext>min</mtext><mspace width="0.6em"/><mn>${hhmm(tPeak + t * tr.dt)}</mn>`),
      xticks: relTicks,
      bands: incidentBands(incs, meta, tPeak, tr.dt), tipExtra: incTip(incs, meta, (t) => tPeak + t * tr.dt),
      hlines: [{ y: p.pre_mean, color: "var(--dt-base)", name: "Media previa" }].concat(lvl != null ? [{ y: lvl, color: MODEL[p.bestFit.model].color, name: "Nueva normalidad" }] : []),
      series: [{ name: "Datos", ys: seg, color: "var(--dt-ink)", dots: true }].concat(
        p.fits.map((f) => ({ name: MODEL[f.model].short, fn: evalFit(f), color: MODEL[f.model].color, dash: MODEL[f.model].dash, width: 2 }))),
      ...extra,
    };
  }
  function cfgIntegral(run, meta, p, extra) {
    const tr = run.track, incs = run.incidents || [], tPeak = tr.t0 + p.i_peak * tr.dt;
    const { cd, cm, cb } = integrals(run, p), ts = cd.map((_, i) => i), M0 = MODEL[p.bestFit.model];
    return {
      xs: ts, height: 220, ydec: 0, yunitShort: "tren·min", label: "Integrales del pico " + p.label,
      xfmt: (t) => M(`<mi>t</mi><mo>=</mo><mo>+</mo><mn>${t}</mn><mspace width="0.2em"/><mtext>min</mtext><mspace width="0.6em"/><mn>${hhmm(tPeak + t * tr.dt)}</mn>`),
      xticks: relTicks,
      bands: incidentBands(incs, meta, tPeak, tr.dt), tipExtra: incTip(incs, meta, (t) => tPeak + t * tr.dt),
      series: [
        { name: "∫ retraso acumulado (datos)", ys: cd, color: "var(--dt-ink)", width: 2.4 },
        { name: "∫ N·" + M0.short.toLowerCase(), ys: cm, color: M0.color, dash: M0.dash || "8 4", width: 2 },
        { name: "∫ N·media previa", ys: cb, color: "var(--dt-base)", dash: "3 4", width: 1.4 },
      ],
      ...extra,
    };
  }

  // ---- Modal genérico ---------------------------------------------------------------------
  const ICON_DL = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3v12"/><path d="m7 10 5 5 5-5"/><path d="M5 21h14"/></svg>`;
  function modal(kicker, title, sub, withDownload) {
    injectCss();
    const ov = document.createElement("div");
    ov.className = "dtk-ov" + (document.documentElement.getAttribute("data-theme") === "light" ? " light" : "");
    ov.setAttribute("role", "dialog");
    ov.setAttribute("aria-modal", "true");
    ov.innerHTML = `<div class="dtk"><div class="dtk-hd"><div><div class="dtk-kick">${esc(kicker)}</div><h2>${esc(title)}</h2><div class="dtk-sub">${sub}</div></div>
      <div class="dtk-hbtns">${withDownload ? `<button class="dtk-dl">${ICON_DL}<span>Descargar gráficos y datos</span></button>` : ""}<button class="dtk-x" aria-label="Cerrar">×</button></div></div><div class="dtk-body"></div></div>`;
    const close = () => { ov.remove(); document.removeEventListener("keydown", onKey); };
    const onKey = (e) => {
      if (e.key !== "Escape") return;
      if (ov.querySelector('[data-picking="1"]')) { ov.dispatchEvent(new CustomEvent("dtk-cancel")); return; }
      close();
    };
    ov.querySelector(".dtk-x").onclick = close;
    ov.addEventListener("pointerdown", (e) => { if (e.target === ov) close(); });
    document.addEventListener("keydown", onKey);
    document.body.appendChild(ov);
    return { body: ov.querySelector(".dtk-body"), ov, dl: ov.querySelector(".dtk-dl") };
  }

  // ---- Seguimiento de retrasos ------------------------------------------------------------
  function open(run, opts) {
    opts = opts || {};
    const tr = run && run.track;
    const incs = run.incidents || [];
    const meta = incidentMeta(incs);
    const nBig = [...meta.values()].filter((m) => m.big).length;
    const { body, ov, dl } = modal("Seguimiento de retrasos", opts.title || "Jornada",
      tr ? `Jornada ${hhmm(tr.t0)} → ${hhmm(tr.t0 + tr.dt * (tr.mean.length - 1))} · ${incs.length} incidencia${incs.length === 1 ? "" : "s"} · muestra cada minuto` : "", !!tr);
    if (!tr || !tr.mean || !tr.mean.length) { body.innerHTML = `<div class="dtk-empty">Esta run no trae la línea temporal de retrasos (vuelve a ejecutarla con el servidor actual).</div>`; return; }
    dl.onclick = () => download(run, opts, ov, dl);
    const xs = tr.mean.map((_, i) => tr.t0 + i * tr.dt);
    const iM = servicePeak(tr, tr.mean).i, iT = servicePeak(tr, tr.total).i;
    const avg = (a) => a.reduce((s, v) => s + v, 0) / a.length;
    body.innerHTML = `
      <div class="dtk-legend"><span><i class="dtk-sw band" style="background:rgba(var(--dt-inc),.12)"></i>Incidencia</span><span><i class="dtk-sw band" style="background:rgba(var(--dt-inc),.4);border-top:3px solid rgb(var(--dt-inc))"></i>Incidencia grave (#n = puesto por gravedad${nBig ? "" : "; hoy ninguna"})</span>${run.gaps && run.gaps.length ? `<span><i class="dtk-sw band" style="background:rgba(128,128,128,.35)"></i>Sin datos (recolector parado o feed caído)</span>` : ""}<span>Pasa el cursor por el gráfico para leer cada minuto</span></div>
      <section class="dtk-card"><h3>Retraso medio por tren en servicio</h3>
        <p class="dtk-note">Σ retrasos vivos ÷ trenes circulando en ese minuto (los que aún esperan su hora en cabecera no cuentan).</p>
        <div class="dtk-kpis"><span>Pico en horas de servicio ${num(tr.mean[iM], 2, "min")} a las ${timeM(xs[iM])}</span><span>Media del día ${num(avg(tr.mean), 2, "min")}</span></div>
        <div id="dtkMean"></div></section>
      <section class="dtk-card"><h3>Retraso acumulado de la red</h3>
        <p class="dtk-note">Suma de los retrasos vivos de todos los trenes en cada minuto: ${M("<mi>D</mi><mo stretchy='false'>(</mo><mi>t</mi><mo stretchy='false'>)</mo><mo>=</mo><mi>N</mi><mo stretchy='false'>(</mo><mi>t</mi><mo stretchy='false'>)</mo><mo>·</mo><mover><mi>d</mi><mo>‾</mo></mover><mo stretchy='false'>(</mo><mi>t</mi><mo stretchy='false'>)</mo>")} (trenes × retraso medio). Sube y baja: no es un contador.</p>
        <div class="dtk-kpis"><span>Pico en horas de servicio ${num(tr.total[iT], 0, "min")} a las ${timeM(xs[iT])}</span><span>Media del día ${num(avg(tr.total), 1, "min")}</span></div>
        <div id="dtkTotal"></div></section>
      ${tr.lines && tr.lines.length ? `<section class="dtk-card"><h3>Por línea</h3>
        <p class="dtk-note">El mismo seguimiento separado por línea, con su color. <b>Superpuestas</b> pone las líneas elegidas en un solo gráfico para ver cómo se contagian (cuándo sube una y después otra); <b>Por separado</b> da un gráfico por línea, todos a la misma escala. Pulsa una línea para mostrarla u ocultarla.</p>
        <div class="dtk-modes"><div class="dtk-seg" id="dtkLSerie"><button data-s="mean">Retraso medio por tren</button><button data-s="total">Retraso acumulado</button></div>
          <div class="dtk-seg" id="dtkLMode"><button data-m="over">Superpuestas</button><button data-m="sep">Por separado</button></div></div>
        <div class="dtk-chips" id="dtkLChips"></div><div id="dtkLines"></div></section>` : ""}
      <div class="dtk-actions"><button class="dtk-btn" id="dtkAn" data-m="auto"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 3v18h18"/><path d="M6 15c3-9 5-9 7-4s4 4 8-6"/></svg>Analizar automáticamente</button>
        <button class="dtk-btn alt" id="dtkMan" data-m="manual"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 4v16"/><path d="M20 4v16"/><path d="M4 12h16"/><path d="m8 9-4 3 4 3"/><path d="m16 9 4 3-4 3"/></svg>Seleccionar tramo a mano</button></div>
      <section id="dtkAnBox" hidden></section>`;
    const $ = (id) => body.querySelector("#" + id);
    lineChart($("dtkMean"), cfgMean(run, meta));
    lineChart($("dtkTotal"), cfgTotal(run, meta));
    if (tr.lines && tr.lines.length) renderLines(body, run, meta);
    const goAnalysis = (ev) => {
      const box = $("dtkAnBox");
      box.hidden = false;
      $("dtkAn").parentElement.hidden = true;
      renderAnalysis(box, run, meta, ev.currentTarget.dataset.m);
      box.scrollIntoView({ behavior: "smooth", block: "start" });
    };
    $("dtkAn").onclick = goAnalysis;
    $("dtkMan").onclick = goAnalysis;
  }

  // ---- Por línea: superpuestas o por separado ----------------------------------------------
  // Solo las líneas que llegan a tener retraso (las demás solo ensucian el gráfico); de entrada se
  // muestran las 6 con más retraso acumulado en el día.
  const lineSum = (l) => l.total.reduce((s, v) => s + v, 0);
  function cfgLines(run, meta, lines, serie, extra) {
    const tr = run.track, xs = tr.mean.map((_, i) => tr.t0 + i * tr.dt), incs = run.incidents || [];
    return { xs, xfmt: timeM, xticks: hourTicks, ydec: serie === "mean" ? 2 : 1, yunitShort: "min",
      label: serie === "mean" ? "Retraso medio por tren de cada línea" : "Retraso acumulado de cada línea",
      bands: gapBands(run).concat(incidentBands(incs, meta)), tipExtra: incTip(incs, meta),
      series: lines.map((l) => ({ name: l.line, ys: l[serie], color: l.color, width: 1.8 })), ...extra };
  }
  function renderLines(body, run, meta) {
    const tr = run.track, $ = (id) => body.querySelector("#" + id);
    const all = tr.lines.filter((l) => lineSum(l) > 0).sort((a, b) => lineSum(b) - lineSum(a));
    if (!all.length) { $("dtkLines").innerHTML = `<div class="dtk-empty">Ninguna línea ha tenido retraso en esta jornada.</div>`; return; }
    run._lineSel = run._lineSel || new Set(all.slice(0, 6).map((l) => l.line));
    let serie = run._lineSerie || "mean", mode = run._lineMode || "over";
    const draw = () => {
      run._lineSerie = serie; run._lineMode = mode;
      body.querySelectorAll("#dtkLSerie button").forEach((b) => b.classList.toggle("on", b.dataset.s === serie));
      body.querySelectorAll("#dtkLMode button").forEach((b) => b.classList.toggle("on", b.dataset.m === mode));
      $("dtkLChips").innerHTML = all.map((l) => `<button class="dtk-chip${run._lineSel.has(l.line) ? " on" : ""}" data-l="${esc(l.line)}" style="--lc:${l.color}" data-no-i18n><i></i>${esc(l.line)}</button>`).join("")
        + `<button class="dtk-lnk" data-all="1">Todas</button><button class="dtk-lnk" data-all="0" style="margin-left:0">Ninguna</button>`;
      $("dtkLChips").querySelectorAll("[data-l]").forEach((b) => (b.onclick = () => { const k = b.dataset.l; run._lineSel.has(k) ? run._lineSel.delete(k) : run._lineSel.add(k); draw(); }));
      $("dtkLChips").querySelectorAll("[data-all]").forEach((b) => (b.onclick = () => { run._lineSel = new Set(b.dataset.all === "1" ? all.map((l) => l.line) : []); draw(); }));
      const sel = all.filter((l) => run._lineSel.has(l.line)), box = $("dtkLines");
      if (!sel.length) { box.innerHTML = `<div class="dtk-empty">Elige al menos una línea.</div>`; return; }
      if (mode === "over") { box.innerHTML = ""; lineChart(box, cfgLines(run, meta, sel, serie, { height: 280 })); return; }
      // Por separado: misma escala vertical en todos (una línea invisible al máximo común).
      const ymax = Math.max(...sel.map((l) => Math.max(...l[serie])));
      box.innerHTML = `<div class="dtk-mini">${sel.map((l, k) => {
        const pk = servicePeak({ active: l.active }, l[serie]);
        return `<div style="--lc:${l.color}"><h4><span data-no-i18n>${esc(l.line)}</span> <small><span>pico</span> ${num(pk.v ?? 0, serie === "mean" ? 2 : 0, "min")} <span>a las</span> ${timeM(tr.t0 + pk.i * tr.dt)} · <span>media del día</span> ${num(l[serie].reduce((s, v) => s + v, 0) / l[serie].length, serie === "mean" ? 2 : 1, "min")}</small></h4><div id="dtkL${k}"></div></div>`;
      }).join("")}</div>`;
      // En los gráficos pequeños, bandas de incidencia y huecos SIN rótulo (se pisarían con el eje).
      sel.forEach((l, k) => {
        const cfg = cfgLines(run, meta, [l], serie, { height: 150, hlines: [{ y: ymax, color: "transparent" }] });
        cfg.bands = cfg.bands.map((b) => ({ ...b, label: null }));
        lineChart($("dtkL" + k), cfg);
      });
    };
    body.querySelectorAll("#dtkLSerie button").forEach((b) => (b.onclick = () => { serie = b.dataset.s; draw(); }));
    body.querySelectorAll("#dtkLMode button").forEach((b) => (b.onclick = () => { mode = b.dataset.m; draw(); }));
    draw();
  }

  // ---- Análisis de recuperación -----------------------------------------------------------
  function fitRows(p) {
    return p.fits.map((f, j) => {
      const Mo = MODEL[f.model], best = j === p.best;
      let eq = "", extra = "";
      if (f.model === "exp") {
        eq = `<mi>y</mi><mo>=</mo>${f.p[0] === 0 ? (f.p[1] < 0 ? "<mo>−</mo>" : "") + sci(Math.abs(f.p[1])) : sci(f.p[0]) + term(f.p[1], (v) => sci(v))}<mo>·</mo><msup><mi>e</mi><mrow><mo>−</mo><mi>t</mi><mo>/</mo>${sci(f.p[2])}</mrow></msup>`;
        extra = `<br><small>${M(`<mi>τ</mi><mo>=</mo><mfrac><mn>1</mn><mi>k</mi></mfrac><mo>=</mo>${sci(f.p[2])}${unit("min")}<mo>,</mo><mspace width="0.5em"/><mi>k</mi><mo>=</mo>${sci(1 / f.p[2])}${unit("")}<msup><mtext>min</mtext><mrow><mo>−</mo><mn>1</mn></mrow></msup>`)}</small>`;
      } else if (f.model === "parab") {
        eq = `<mi>y</mi><mo>=</mo>${sci(f.p[0])}<mo>·</mo><msup><mi>t</mi><mn>2</mn></msup>${term(f.p[1], (v) => sci(v))}<mo>·</mo><mi>t</mi>${term(f.p[2], (v) => sci(v))}`;
      } else {
        eq = `<mi>y</mi><mo>=</mo>${sci(f.p[0])}<mo>·</mo><mi>t</mi>${term(f.p[1], (v) => sci(v))}`;
        extra = `<br><small>drenaje ${M(`${sci(-f.p[0] * 60)}${unit("min/h")}`)}</small>`;
      }
      const tag = best ? (f.r2 < 0.5 ? '<span class="dtk-tag" style="background:var(--dt-mu)">mejor · ajuste pobre</span>' : '<span class="dtk-tag">mejor</span>') : "";
      // Estabilidad: en la exponencial se enseña que es 3τ (τ·ln 20); el origen del nivel se indica.
      const stab = f.t_stab != null
        ? durM(f.t_stab) + (f.model === "exp" ? `<br><small>${M("<mo>=</mo><mi>τ</mi><mo>·</mo><mi>ln</mi><mo>⁡</mo><mn>20</mn><mo>≈</mo><mn>3</mn><mi>τ</mi>")}</small>` : "")
        : `<small>no se estabiliza</small>`;
      const lvl = f.new_level != null ? num(f.new_level, 3, "min") + `<br><small>${f.level_src === "model" ? (f.model === "exp" ? "asíntota" : "vértice") : "meseta observada"}</small>` : "–";
      const hu = f.footprint != null ? signed(f.footprint, 3, "min") : "–";
      return `<tr class="${best ? "best" : ""}"><td><div class="mm"><span class="dtk-sw" style="color:${Mo.color}${Mo.dash ? ";border-top-style:dashed" : ""}"></span>${Mo.name}${tag}</div></td>
        <td>${M(eq)}${extra}</td><td class="num">${num(f.r2, 3)}</td><td class="num">${num(f.r2_adj, 3)}</td><td class="num">${stab}</td><td class="num">${lvl}</td><td class="num">${hu}</td></tr>`;
    }).join("");
  }
  const TBL_HEAD = `<thead><tr><th>Modelo</th><th>Ajuste (${M("<mi>t</mi>")} en min desde el pico)</th><th class="num">${M(R2)}</th><th class="num">${M(R2)} aj.</th><th class="num">Tiempo hasta estabilidad</th><th class="num">Nueva normalidad</th><th class="num">Huella</th></tr></thead>`;

  // Modo AUTOMÁTICO (episodios detectados por el motor) o MANUAL (tramos que marca el usuario con
  // el cursor sobre el gráfico: 1er clic = inicio del episodio, 2º clic = final). Los tramos
  // manuales se analizan en el servidor con la MISMA matemática (`recovery::analyze_manual`) y se
  // guardan en la run (`run._manual`), así que siguen ahí al reabrir y entran en la descarga.
  function renderAnalysis(box, run, meta, startMode) {
    const tr = run.track;
    const ov = box.closest(".dtk-ov");
    run._manual = run._manual || [];
    let serie = "mean", mode = startMode || "auto", pickFrom = null, msg = "";
    box.innerHTML = `<section class="dtk-card"><h3>Análisis de la recuperación</h3>
      <div class="dtk-modes"><div class="dtk-seg" id="dtkMode"><button data-m="auto">Automático</button><button data-m="manual">Selección manual</button></div>
      <div class="dtk-seg" id="dtkSerie"><button data-s="mean">Retraso medio · ajustes</button><button data-s="total">Retraso acumulado · integrales</button></div></div>
      <p class="dtk-note" id="dtkHow"></p><div class="dtk-pick" id="dtkPick" hidden></div>
      <div id="dtkSum"></div><div id="dtkOver"></div></section><div class="dtk-peaks" id="dtkPeaks"></div><div id="dtkExpl"></div>`;
    const $ = (id) => box.querySelector("#" + id);
    const med = (a) => { if (!a.length) return null; const s = [...a].sort((x, y) => x - y); return s[Math.floor(s.length / 2)]; };
    const xAt = (i) => tr.t0 + i * tr.dt;

    // Selección con el cursor: 1er clic fija el inicio, 2º el final → análisis en el servidor.
    async function onPick(i) {
      if (pickFrom == null) { pickFrom = i; msg = ""; draw(); return; }
      const a = Math.min(pickFrom, i), b = Math.max(pickFrom, i);
      pickFrom = null;
      if (b - a < 8) { msg = "Tramo demasiado corto (mín. 8 min): vuelve a marcar el inicio."; draw(); return; }
      msg = `Analizando ${hhmm(xAt(a))}–${hhmm(xAt(b))}…`; draw();
      try {
        const r = await (await fetch("/api/recovery/fit", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ total: tr.total, active: tr.active, i0: a, i1: b }) })).json();
        if (!r.ok) msg = r.error || "No se pudo analizar el tramo.";
        else { run._manual.push(r.peak); msg = ""; }
      } catch (e) { msg = "Error al analizar: " + e.message; }
      draw();
    }
    // Esc con un inicio marcado cancela la selección (en vez de cerrar el modal).
    ov.addEventListener("dtk-cancel", () => { pickFrom = null; msg = ""; draw(); });

    const draw = () => {
      box.querySelectorAll("#dtkMode button").forEach((b) => b.classList.toggle("on", b.dataset.m === mode));
      box.querySelectorAll("#dtkSerie button").forEach((b) => b.classList.toggle("on", b.dataset.s === serie));
      box.dataset.picking = mode === "manual" && pickFrom != null ? "1" : "";
      const peaks = rankedPeaks(run, mode);
      const manual = mode === "manual";
      $("dtkHow").innerHTML = manual
        ? `<b>Marca tú el episodio con el cursor:</b> haz clic en el gráfico en el <b>inicio</b> (antes de que el retraso empiece a subir) y luego en el <b>final</b> (cuando ya se ha estabilizado). Se ajusta la bajada desde el máximo del tramo hasta el final, y la media previa son los 30 min anteriores al inicio. Puedes marcar varios tramos; ${M("<mi>Esc</mi>")} cancela un inicio marcado.`
        : `Se detectan los picos destacables del retraso medio (prominencia ≥ 20 % del rango del día y ≥ 6σ del ruido, solo en horas de servicio). Los picos <b>pegados que forman un mismo episodio</b> se agrupan en uno (si están a ≤ 15 min o si entre ellos la red no llegó a recuperar ni la mitad de lo subido), y la bajada se ajusta desde el más alto. Van <b>ordenados por el ${M(R2)} de su mejor modelo</b> (el nº es su puesto).`;
      const pk = $("dtkPick");
      pk.hidden = !manual;
      if (manual) {
        const st = pickFrom != null
          ? `Inicio en <b>${timeM(xAt(pickFrom))}</b> — ahora haz clic en el <b>final</b>.`
          : `Haz clic en el <b>inicio</b> del episodio.`;
        pk.innerHTML = `<span>${st}</span>${msg ? `<span class="err">${esc(msg)}</span>` : ""}${run._manual.length ? `<button class="dtk-lnk" id="dtkClr">Quitar todos los tramos (${run._manual.length})</button>` : ""}`;
        const clr = $("dtkClr"); if (clr) clr.onclick = () => { run._manual = []; pickFrom = null; msg = ""; draw(); };
      }
      lineChart($("dtkOver"), cfgOverview(run, meta, peaks, serie, manual ? { onPick, pickFrom, height: 250 } : {}));
      const cont = $("dtkPeaks"); cont.innerHTML = "";
      if (!peaks.length) {
        $("dtkSum").innerHTML = manual ? `<div class="dtk-empty">Aún no has marcado ningún tramo.</div>` : `<div class="dtk-empty">No hay picos destacables: la red no se ha desviado de su vaivén normal.</div>`;
        $("dtkExpl").innerHTML = "";
        return;
      }
      const head = (p, extra) => {
        const tPeak = xAt(p.i_peak);
        const tag = p.manual ? ` · tramo manual ${timeM(xAt(p.i_start))}–${timeM(xAt(p.i_end))}` : p.merged > 1 ? ` · episodio de ${p.merged} picos agrupados` : "";
        const rm = p.manual ? `<button class="dtk-lnk" data-rm="${run._manual.indexOf(p.src)}">Quitar</button>` : "";
        return `<h4><span class="n">${p.label}</span>Pico a las ${timeM(tPeak)} <small>${extra}${tag}</small>${rm}</h4>`;
      };
      if (serie === "mean") {
        const wins = { exp: 0, parab: 0, lin: 0 }, taus = [], stabs = [], hus = [];
        for (const p of peaks) {
          wins[p.bestFit.model]++;
          const e = p.fits.find((f) => f.model === "exp"); if (e) taus.push(e.p[2]);
          if (p.bestFit.t_stab != null) stabs.push(p.bestFit.t_stab);
          if (p.bestFit.footprint != null) hus.push(p.bestFit.footprint);
        }
        const top = Object.entries(wins).sort((a, b) => b[1] - a[1])[0];
        $("dtkSum").innerHTML = `<div class="dtk-sum">
            <div><span class="lb">${manual ? "Tramos marcados" : "Episodios destacables"}</span><span class="vl">${num(peaks.length, 0)}</span></div>
            <div><span class="lb">Modelo que mejor ajusta</span><span class="vl">${MODEL[top[0]].short} ${M(`<mo stretchy='false'>(</mo><mn>${top[1]}</mn><mo>/</mo><mn>${peaks.length}</mn><mo stretchy='false'>)</mo>`)}</span></div>
            <div><span class="lb">${M("<mi>τ</mi>")} mediano (exponencial)</span><span class="vl">${num(med(taus), 1, "min")}</span></div>
            <div><span class="lb">Tiempo hasta estabilidad (mediana)</span><span class="vl">${durM(med(stabs))}</span></div>
            <div><span class="lb">Huella mediana</span><span class="vl">${signed(med(hus), 3, "min")}</span></div></div>`;
        for (const p of peaks) {
          const card = document.createElement("section");
          card.className = "dtk-card dtk-pk";
          const bf = p.bestFit, Mb = MODEL[bf.model];
          card.innerHTML = head(p, `${num(p.y_peak, 2, "min")} · media previa ${num(p.pre_mean, 3, "min")} · bajada ${num(p.i_end - p.i_peak, 0, "min")} · mejor ${M(R2 + "<mo>=</mo>" + mn(bf.r2, 3))}`) + `
            <div class="dtk-legend" style="margin-top:6px"><span><i class="dtk-sw dot" style="color:var(--dt-ink)"></i>Datos</span>${p.fits.map((f) => `<span><i class="dtk-sw" style="color:${MODEL[f.model].color}${MODEL[f.model].dash ? ";border-top-style:dashed" : ""}"></i>${MODEL[f.model].name}</span>`).join("")}<span><i class="dtk-sw" style="color:var(--dt-base);border-top-style:dashed"></i>Media previa</span>${bf.new_level != null ? `<span><i class="dtk-sw" style="color:${Mb.color};border-top-style:dashed"></i>Nueva normalidad (${Mb.short.toLowerCase()})</span>` : ""}</div>
            <div class="pkc"></div>
            <div class="dtk-tblw"><table class="dtk-tbl">${TBL_HEAD}<tbody>${fitRows(p)}</tbody></table></div>`;
          cont.appendChild(card);
          lineChart(card.querySelector(".pkc"), cfgPeak(run, meta, p));
        }
        $("dtkExpl").innerHTML = `<div class="dtk-expl">
          <p><b>Tiempo hasta estabilidad</b>: minutos desde el pico hasta que al modelo solo le queda el 5 % de su bajada por recorrer — a partir de ahí el retraso por minuto es prácticamente constante (la cosa se ha relajado). En la exponencial ${M("<mi>y</mi><mo>=</mo><mi>b</mi><mo>+</mo><mi>A</mi><msup><mi>e</mi><mrow><mo>−</mo><mi>k</mi><mi>t</mi></mrow></msup>")} la constante de tiempo es ${M("<mi>τ</mi><mo>=</mo><mn>1</mn><mo>/</mo><mi>k</mi>")}: en ${M("<mi>τ</mi>")} se ha relajado el 63 %, y la estabilidad llega en ${M("<mi>τ</mi><mo>·</mo><mi>ln</mi><mo>⁡</mo><mn>20</mn><mo>≈</mo><mn>3</mn><mi>τ</mi>")} (95 %).</p>
          <p><b>Nueva normalidad</b>: el nivel en que se estabiliza — la asíntota ${M("<mi>b</mi>")} de la exponencial, el vértice de la parábola si se frena (${M("<mi>a</mi><mo>&gt;</mo><mn>0</mn>")}), o la meseta observada al final del tramo para la recta y la parábola que se acelera, que no tienen suelo propio.</p>
          <p><b>Huella</b> = nueva normalidad − media previa (los 30 min antes de que empezara a subir): el retraso por tren que el pico deja instalado en el sistema. ${M("<mo>≈</mo><mn>0</mn>")}: se recompone del todo; ${M("<mo>&gt;</mo><mn>0</mn>")}: la red queda peor que antes; ${M("<mo>&lt;</mo><mn>0</mn>")}: queda mejor (p. ej. porque baja la demanda tras la punta). El ${M(R2)} ajustado penaliza el parámetro extra de la exponencial y la parábola frente a la recta.</p></div>`;
      } else {
        const errs = peaks.map((p) => (p.bestFit.area - p.area_data) / Math.max(1e-9, p.area_data));
        const exc = peaks.reduce((s, p) => s + p.area_excess, 0);
        $("dtkSum").innerHTML = `<div class="dtk-sum">
            <div><span class="lb">${manual ? "Tramos" : "Episodios"}</span><span class="vl">${num(peaks.length, 0)}</span></div>
            <div><span class="lb">Error mediano de la integral</span><span class="vl">${num(100 * med(errs.map(Math.abs)), 2, "%")}</span></div>
            <div><span class="lb">Peor error</span><span class="vl">${num(100 * Math.max(...errs.map(Math.abs)), 2, "%")}</span></div>
            <div><span class="lb">Exceso total sobre la normalidad previa</span><span class="vl">${num(exc, 0, "tren·min")}</span></div></div>`;
        for (const p of peaks) {
          const card = document.createElement("section");
          card.className = "dtk-card dtk-pk";
          const bf = p.bestFit, Mb = MODEL[bf.model];
          const err = (bf.area - p.area_data) / Math.max(1e-9, p.area_data);
          card.innerHTML = head(p, `mejor modelo: ${Mb.name.toLowerCase()} · bajada ${num(p.i_end - p.i_peak, 0, "min")}`) + `
            <div class="dtk-int">
              <div><span>Retraso acumulado real</span>${M(`<msubsup><mo>∫</mo><mn>0</mn><mi>T</mi></msubsup><mi>D</mi><mo stretchy='false'>(</mo><mi>t</mi><mo stretchy='false'>)</mo><mi>d</mi><mi>t</mi><mo>=</mo>${mn(p.area_data, 0)}${unit("tren·min")}`)}</div>
              <div><span>Predicho por el modelo</span>${M(`<msubsup><mo>∫</mo><mn>0</mn><mi>T</mi></msubsup><mi>N</mi><mo stretchy='false'>(</mo><mi>t</mi><mo stretchy='false'>)</mo><mi>f</mi><mo stretchy='false'>(</mo><mi>t</mi><mo stretchy='false'>)</mo><mi>d</mi><mi>t</mi><mo>=</mo>${mn(bf.area, 0)}${unit("tren·min")}`)}</div>
              <div><span>Diferencia</span>${signed(100 * err, 2, "%")}</div>
              <div><span>Exceso sobre la normalidad previa</span>${M(`<msubsup><mo>∫</mo><mn>0</mn><mi>T</mi></msubsup><mo stretchy='false'>(</mo><mi>D</mi><mo>−</mo><mi>N</mi><msub><mover><mi>d</mi><mo>‾</mo></mover><mtext>previa</mtext></msub><mo stretchy='false'>)</mo><mi>d</mi><mi>t</mi><mo>=</mo>${mn(p.area_excess, 0)}${unit("tren·min")}`)}</div>
            </div>
            <div class="dtk-legend"><span><i class="dtk-sw" style="color:var(--dt-ink)"></i>∫ datos</span><span><i class="dtk-sw" style="color:${Mb.color};border-top-style:dashed"></i>∫ N·${Mb.short.toLowerCase()}</span><span><i class="dtk-sw" style="color:var(--dt-base);border-top-style:dashed"></i>∫ N·media previa (lo que habría costado sin pico)</span></div>
            <div class="pkc"></div>`;
          cont.appendChild(card);
          lineChart(card.querySelector(".pkc"), cfgIntegral(run, meta, p));
        }
        $("dtkExpl").innerHTML = `<div class="dtk-expl">
          <p><b>Relación entre las dos series.</b> El retraso acumulado de la red no es la primitiva del retraso medio: es su producto por los trenes en servicio, ${M("<mi>D</mi><mo stretchy='false'>(</mo><mi>t</mi><mo stretchy='false'>)</mo><mo>=</mo><mi>N</mi><mo stretchy='false'>(</mo><mi>t</mi><mo stretchy='false'>)</mo><mo>·</mo><mover><mi>d</mi><mo>‾</mo></mover><mo stretchy='false'>(</mo><mi>t</mi><mo stretchy='false'>)</mo>")}. La primitiva sí aparece un nivel más arriba: ${M("<mo>∫</mo><mi>D</mi><mo stretchy='false'>(</mo><mi>t</mi><mo stretchy='false'>)</mo><mi>d</mi><mi>t</mi>")} es el retraso realmente sufrido durante la bajada, en tren·min, y su derivada es ${M("<mi>D</mi><mo stretchy='false'>(</mo><mi>t</mi><mo stretchy='false'>)</mo>")}. Por eso aquí se compara esa integral con la del mejor modelo del retraso medio multiplicado por los trenes que circulaban, ${M("<mo>∫</mo><mi>N</mi><mo stretchy='false'>(</mo><mi>t</mi><mo stretchy='false'>)</mo><mi>f</mi><mo stretchy='false'>(</mo><mi>t</mi><mo stretchy='false'>)</mo><mi>d</mi><mi>t</mi>")}.</p>
          <p><b>Ojo al leerlo:</b> un ajuste por mínimos cuadrados con término constante deja residuos de media casi nula, así que la integral casi siempre cuadra (errores de centésimas de %) aunque el ${M(R2)} sea bajo — confirma que el modelo conserva el «presupuesto» de retraso del pico, pero no discrimina entre modelos; para eso está el ${M(R2)}. El dato nuevo que aporta es el <b>exceso sobre la normalidad previa</b>: el área entre la curva real y la que habría habido sin pico = el coste del pico para la red.</p></div>`;
      }
      cont.querySelectorAll("[data-rm]").forEach((b) => (b.onclick = () => { run._manual.splice(+b.dataset.rm, 1); draw(); }));
    };
    box.querySelectorAll("#dtkMode button").forEach((b) => (b.onclick = () => { mode = b.dataset.m; pickFrom = null; msg = ""; draw(); }));
    box.querySelectorAll("#dtkSerie button").forEach((b) => (b.onclick = () => { serie = b.dataset.s; draw(); }));
    draw();
  }

  // ---- Incidencias del día por orden de gravedad ------------------------------------------
  function openIncidents(run, opts) {
    opts = opts || {};
    const list = byGravity(run.incidents);
    const meta = incidentMeta(run.incidents);
    const { body } = modal("Incidencias del día", opts.title || "Jornada",
      `${list.length} incidencia${list.length === 1 ? "" : "s"} · ordenadas por gravedad (impacto medido sobre la red)`, false);
    if (!list.length) { body.innerHTML = `<div class="dtk-empty">Sin incidencias este día.</div>`; return; }
    const max = Math.max(0.01, ...list.map((i) => i.impact_min || 0));
    body.innerHTML = `<p class="dtk-note">Impacto = cuánto subió el <b>retraso acumulado de la red</b> desde que empezó la incidencia hasta su máximo (durante la incidencia y los 30 min siguientes). Si dos incidencias se solapan comparten esa subida. Las marcadas en rojo son las graves (se resaltan en los gráficos del seguimiento).</p>
      <ul class="dtk-inc">${list.map((i, k) => {
        const d = Math.round((i.t1 - i.t0) / 60), big = (meta.get(i) || {}).big;
        return `<li${big ? ' style="border-color:rgb(var(--dt-inc))"' : ""}><div class="rk">${M(`<mo>#</mo><mn>${k + 1}</mn>`)}</div>
          <div><div class="mt"><span data-no-i18n>${esc(i.motivo)}</span><small>${EFF[i.effect] || esc(i.effect)} · ${num(d, 0, "min")}</small></div>
          <div class="wh" data-no-i18n>${esc(i.from_name || i.from)} → ${esc(i.to_name || i.to)} · ${timeM(i.t0)}–${timeM(i.t1)}</div></div>
          <div class="imp">Impacto en la red ${signed(i.impact_min || 0, 1, "min")}<div class="bar"><i style="width:${(100 * (i.impact_min || 0) / max).toFixed(1)}%"></i></div></div></li>`;
      }).join("")}</ul>`;
  }

  // ---- Descarga: gráficos (SVG + PNG) y datos (CSV) de la run, en un ZIP -------------------
  // Todo en el navegador: los gráficos se regeneran fuera de pantalla con la MISMA configuración
  // que la vista, se resuelven las variables CSS del tema a colores fijos (un SVG suelto no ve la
  // hoja de estilo) y se rasterizan a PNG ×2. ZIP sin compresión (método "store") escrito a mano.
  const CRC_T = (() => { const t = new Uint32Array(256); for (let n = 0; n < 256; n++) { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; t[n] = c >>> 0; } return t; })();
  const crc32 = (b) => { let c = 0xffffffff; for (let i = 0; i < b.length; i++) c = CRC_T[(c ^ b[i]) & 0xff] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0; };
  function zipStore(files) {
    const enc = new TextEncoder(), parts = [], central = [];
    const d = new Date(), tm = (d.getHours() << 11) | (d.getMinutes() << 5) | (d.getSeconds() >> 1), dt = ((d.getFullYear() - 1980) << 9) | ((d.getMonth() + 1) << 5) | d.getDate();
    let off = 0;
    for (const f of files) {
      const nm = enc.encode(f.name), data = f.data, crc = crc32(data), sz = data.length;
      const h = new DataView(new ArrayBuffer(30));
      h.setUint32(0, 0x04034b50, true); h.setUint16(4, 20, true); h.setUint16(6, 0x0800, true); h.setUint16(10, tm, true); h.setUint16(12, dt, true);
      h.setUint32(14, crc, true); h.setUint32(18, sz, true); h.setUint32(22, sz, true); h.setUint16(26, nm.length, true);
      parts.push(new Uint8Array(h.buffer), nm, data);
      const c = new DataView(new ArrayBuffer(46));
      c.setUint32(0, 0x02014b50, true); c.setUint16(4, 20, true); c.setUint16(6, 20, true); c.setUint16(8, 0x0800, true); c.setUint16(12, tm, true); c.setUint16(14, dt, true);
      c.setUint32(16, crc, true); c.setUint32(20, sz, true); c.setUint32(24, sz, true); c.setUint16(28, nm.length, true); c.setUint32(42, off, true);
      central.push(new Uint8Array(c.buffer), nm);
      off += 30 + nm.length + sz;
    }
    const csz = central.reduce((s, a) => s + a.length, 0);
    const e = new DataView(new ArrayBuffer(22));
    e.setUint32(0, 0x06054b50, true); e.setUint16(8, files.length, true); e.setUint16(10, files.length, true); e.setUint32(12, csz, true); e.setUint32(16, off, true);
    return new Blob([...parts, ...central, new Uint8Array(e.buffer)], { type: "application/zip" });
  }
  // CSV para Excel en español: separador ";", coma decimal, BOM UTF-8.
  const csvNum = (v, d = 4) => (v == null || !Number.isFinite(v) ? "" : v.toFixed(d).replace(".", ","));
  const csvTxt = (s) => `"${String(s == null ? "" : s).replace(/"/g, '""')}"`;
  const csv = (rows) => new TextEncoder().encode("﻿" + rows.map((r) => r.join(";")).join("\r\n") + "\r\n");

  // Leyenda dibujada DENTRO del SVG (un PNG suelto no tiene la leyenda HTML de la vista).
  function legendSvg(cfg, W) {
    const items = cfg.series.map((s) => ({ name: s.name, color: s.color, dash: s.dash, dot: s.dots }))
      .concat((cfg.hlines || []).filter((h) => h.name).map((h) => ({ name: h.name, color: h.color, dash: "6 4" })));
    const xa = cfg.xs[0], xb = cfg.xs[cfg.xs.length - 1];
    if ((cfg.bands || []).some((b) => b.edge && b.x1 >= xa && b.x0 <= xb)) items.push({ name: "Incidencia grave (#puesto)", color: "rgb(var(--dt-inc))", band: true });
    let x = W - 12, out = "";
    for (const it of items.reverse()) {
      const w = 26 + it.name.length * 6.2;
      x -= w;
      const sw = it.dot ? `<circle cx="${x + 8}" cy="16" r="3.5" fill="${it.color}"/>`
        : it.band ? `<rect x="${x + 1}" y="11" width="14" height="10" fill="rgba(var(--dt-inc),.4)"/><rect x="${x + 1}" y="11" width="14" height="3" fill="${it.color}"/>`
        : `<line x1="${x}" x2="${x + 16}" y1="16" y2="16" stroke="${it.color}" stroke-width="2"${it.dash ? ` stroke-dasharray="${it.dash}"` : ""}/>`;
      out += sw + `<text x="${x + 21}" y="20" class="lg">${esc(it.name)}</text>`;
      x -= 10;
    }
    return out;
  }
  function standaloneSvg(el, ov, title, cfg) {
    const cs = getComputedStyle(ov);
    const v = (name) => cs.getPropertyValue(name).trim();
    const svg = el.querySelector("svg").cloneNode(true);
    svg.querySelector(".cur")?.remove(); svg.querySelector(".hit")?.remove();
    const W = +svg.getAttribute("width"), H = +svg.getAttribute("height"), top = 30;
    let inner = svg.innerHTML + `<g transform="translate(0,${-top})">${legendSvg(cfg, W)}</g>`;
    // var(--x) → color fijo; rgba(var(--dt-inc),a) queda resuelto también (la variable son 3 canales).
    inner = inner.replace(/var\((--[a-z0-9-]+)\)/gi, (_, n) => v(n) || "#888");
    const bg = v("--dt-bg") || "#fff", mu = v("--dt-mu"), tx = v("--dt-tx");
    const style = `.ax{font:12.5px ${MATH_FONT};fill:${mu}}.gr{stroke:${v("--dt-grid")};stroke-width:1}.base{stroke:${v("--dt-bd")};stroke-width:1}.bandlbl{font:600 10px 'Segoe UI',sans-serif;fill:${mu}}.inclbl{font:700 10px 'Segoe UI',sans-serif}.lg{font:12px 'Segoe UI',sans-serif;fill:${mu}}`;
    return `<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H + top}" viewBox="0 0 ${W} ${H + top}"><style>${style}</style>` +
      `<rect width="100%" height="100%" fill="${bg}"/><text x="14" y="20" style="font:700 14px 'Segoe UI',sans-serif;fill:${tx}">${esc(title)}</text>` +
      `<g transform="translate(0,${top})">${inner}</g></svg>`;
  }
  async function svgToPng(svgText) {
    const img = new Image();
    img.src = "data:image/svg+xml;charset=utf-8," + encodeURIComponent(svgText);
    await img.decode();
    const c = document.createElement("canvas");
    c.width = img.width * 2; c.height = img.height * 2;
    const ctx = c.getContext("2d"); ctx.scale(2, 2); ctx.drawImage(img, 0, 0);
    const blob = await new Promise((res) => c.toBlob(res, "image/png"));
    return new Uint8Array(await blob.arrayBuffer());
  }

  async function download(run, opts, ov, btn) {
    const label = btn.querySelector("span"), txt0 = label.textContent;
    btn.disabled = true; label.textContent = "Preparando…";
    try {
      const tr = run.track, incs = run.incidents || [], meta = incidentMeta(incs), peaks = rankedPeaks(run, "auto").concat(rankedPeaks(run, "manual"));
      const enc = new TextEncoder(), files = [];
      const slug = String(opts.title || "run").normalize("NFD").replace(/[̀-ͯ]/g, "").replace(/[^a-z0-9]+/gi, "_").replace(/^_|_$/g, "").toLowerCase();
      // 1) Gráficos: se dibujan en un lienzo oculto de 1100 px con la configuración de la vista.
      const host = document.createElement("div");
      host.style.cssText = "position:absolute;left:-30000px;top:0;width:1100px";
      ov.appendChild(host);
      const charts = [
        ["01_retraso_medio", "Retraso medio por tren en servicio (min)", cfgMean(run, meta, { static: true, height: 300 })],
        ["02_retraso_acumulado", "Retraso acumulado de la red (min)", cfgTotal(run, meta, { static: true, height: 300 })],
        ["03_picos_analizados", "Picos analizados (nº = puesto por R² del mejor modelo)", cfgOverview(run, meta, rankedPeaks(run, "auto"), "mean", { static: true, height: 260 })],
      ];
      for (const p of peaks) {
        const hm = hhmm(tr.t0 + p.i_peak * tr.dt).replace(":", "h");
        const kind = p.manual ? "manual " : "";
        charts.push([`pico_${p.label}_${hm}_ajustes`, `Pico ${kind}${p.label} (${hhmm(tr.t0 + p.i_peak * tr.dt)}) · ajustes del retraso medio (min)`, cfgPeak(run, meta, p, { static: true, height: 300 })]);
        charts.push([`pico_${p.label}_${hm}_integrales`, `Pico ${kind}${p.label} (${hhmm(tr.t0 + p.i_peak * tr.dt)}) · integrales (tren·min)`, cfgIntegral(run, meta, p, { static: true, height: 300 })]);
      }
      for (const [name, title, cfg] of charts) {
        label.textContent = "Gráfico " + name + "…";
        const el = document.createElement("div"); host.appendChild(el);
        lineChart(el, cfg);
        const svg = standaloneSvg(el, ov, title, cfg);
        files.push({ name: `graficos/${name}.svg`, data: enc.encode(svg) });
        try { files.push({ name: `graficos/${name}.png`, data: await svgToPng(svg) }); } catch (e) { /* el SVG basta */ }
      }
      host.remove();
      // 2) Datos.
      label.textContent = "Datos…";
      const activeAt = (x) => incs.filter((i) => x >= i.t0 && x <= i.t1).map((i) => i.motivo).join(" | ");
      files.push({ name: "serie_por_minuto.csv", data: csv([["hora", "t_s", "retraso_medio_min", "retraso_acumulado_min", "trenes_en_servicio", "incidencias_activas"]]
        .concat(tr.mean.map((m, i) => { const x = tr.t0 + i * tr.dt; return [hhmm(x), x, csvNum(m, 2), csvNum(tr.total[i], 2), tr.active[i], csvTxt(activeAt(x))]; }))) });
      const rows = [["puesto", "tipo", "picos_agrupados", "inicio_tramo", "fin_tramo", "hora_pico", "retraso_pico_min", "media_previa_min", "tramo_min", "modelo", "mejor", "R2", "R2_ajustado", "p1", "p2", "p3", "tau_min", "tiempo_estabilidad_min", "nueva_normalidad_min", "origen_nivel", "huella_min", "integral_modelo_tren_min", "integral_datos_tren_min", "error_integral_pct", "exceso_sobre_previa_tren_min"]];
      for (const p of peaks) for (const [j, f] of p.fits.entries()) {
        rows.push([p.label, p.manual ? "manual" : "automático", p.merged || "", hhmm(tr.t0 + p.i_start * tr.dt), hhmm(tr.t0 + p.i_end * tr.dt), hhmm(tr.t0 + p.i_peak * tr.dt), csvNum(p.y_peak), csvNum(p.pre_mean), p.i_end - p.i_peak, MODEL[f.model].name, j === p.best ? "sí" : "",
          csvNum(f.r2), csvNum(f.r2_adj), csvNum(f.p[0], 8), csvNum(f.p[1], 8), csvNum(f.p[2], 8), f.model === "exp" ? csvNum(f.p[2], 3) : "",
          csvNum(f.t_stab, 2), csvNum(f.new_level), f.level_src === "model" ? "modelo" : "datos", csvNum(f.footprint),
          csvNum(f.area, 1), csvNum(p.area_data, 1), csvNum(100 * (f.area - p.area_data) / Math.max(1e-9, p.area_data), 3), csvNum(p.area_excess, 1)]);
      }
      files.push({ name: "analisis_picos.csv", data: csv(rows) });
      if (tr.lines && tr.lines.length) {
        const ls = tr.lines;
        files.push({ name: "serie_por_linea.csv", data: csv([["hora", "t_s"].concat(...ls.map((l) => [l.line + "_retraso_medio_min", l.line + "_retraso_acumulado_min", l.line + "_trenes"]))]
          .concat(tr.mean.map((_, i) => { const x = tr.t0 + i * tr.dt; return [hhmm(x), x].concat(...ls.map((l) => [csvNum(l.mean[i], 2), csvNum(l.total[i], 2), l.active[i]])); }))) });
      }
      files.push({ name: "incidencias_por_gravedad.csv", data: csv([["puesto", "grave", "motivo", "efecto", "desde", "hasta", "inicio", "fin", "duracion_min", "impacto_red_min"]]
        .concat(byGravity(incs).map((i, k) => [k + 1, (meta.get(i) || {}).big ? "sí" : "", csvTxt(i.motivo), csvTxt(EFF[i.effect] || i.effect), csvTxt(i.from_name || i.from), csvTxt(i.to_name || i.to), hhmm(i.t0), hhmm(i.t1), Math.round((i.t1 - i.t0) / 60), csvNum(i.impact_min || 0, 2)]))) });
      files.push({ name: "run.json", data: enc.encode(JSON.stringify(run)) });
      files.push({ name: "LEEME.txt", data: enc.encode([
        `Seguimiento de retrasos · ${opts.title || ""}`,
        "",
        "serie_por_minuto.csv       retraso medio por tren en servicio y retraso acumulado de la red (Σ retrasos vivos) cada minuto.",
        "serie_por_linea.csv        lo mismo separado por línea (retraso medio, acumulado y trenes en servicio de cada línea).",
        "analisis_picos.csv         una fila por pico y modelo. Parámetros: exponencial y = p1 + p2·e^(−t/p3) (p3 = τ = 1/k);",
        "                           parábola y = p1·t² + p2·t + p3; lineal y = p1·t + p2 (t en min desde el pico).",
        "                           Tiempo hasta estabilidad: cuando al modelo le queda el 5 % de su bajada (exp: τ·ln20 ≈ 3τ).",
        "                           Nueva normalidad: asíntota (exp), vértice (parábola convexa) o meseta observada (resto).",
        "                           Huella = nueva normalidad − media previa (30 min antes de la subida).",
        "                           Integrales en tren·min: ∫N(t)·f(t)dt (modelo) frente a ∫D(t)dt (datos) sobre el tramo.",
        "incidencias_por_gravedad.csv  gravedad = subida del retraso acumulado durante la incidencia (+30 min).",
        "graficos/                  cada gráfico en SVG (vectorial) y PNG. Los picos van numerados por el R² de su mejor modelo.",
        "run.json                   todo lo que devolvió el motor para esta run.",
      ].join("\r\n")) });
      const blob = zipStore(files), url = URL.createObjectURL(blob), a = document.createElement("a");
      a.href = url; a.download = `seguimiento_${slug}.zip`;
      document.body.appendChild(a); a.click(); a.remove();
      setTimeout(() => URL.revokeObjectURL(url), 4000);
      label.textContent = "Descargado";
      setTimeout(() => (label.textContent = txt0), 2500);
    } catch (e) {
      label.textContent = "Error: " + e.message;
      setTimeout(() => (label.textContent = txt0), 4000);
    } finally {
      btn.disabled = false;
    }
  }

  // `lineChart`, `hourTicks` y `timeM` se exportan para la comparación simulación ↔ realidad de /datos.
  window.DelayTrack = { open, openIncidents, servicePeak, lineChart, hourTicks, timeM, injectCss };
})();
