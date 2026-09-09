// Rodalies Simulator — juego web (Fase 3).
//
// Reimplementación web-nativa (Canvas 2D + JS puro, sin frameworks) del simulador de red de
// Godot, alimentada dinámicamente desde el GTFS de Fomento_Transit vía la API del servidor Rust:
//   * /api/game/network             → topología (estaciones, secuencias reales, color oficial)
//   * /api/game/schedule?source=…   → horarios del día (GTFS «tal cual» u optimizados)
//
// Mantiene las mecánicas originales: mapa geográfico data-driven, trenes circulando por su
// ruta real siguiendo el horario, reloj de jornada 05:00→00:00 con velocidad (Pausa/×1/×2/×5/×20),
// desplazamiento y zoom de cámara, información por estación, e incidencias con KPI de retraso
// (minutos·pasajero). El proyecto Godot original se conserva intacto en game/godot-original/.

"use strict";

// ---- Constantes de simulación -------------------------------------------------------------
const DIA_INICIO = 5 * 3600;        // 05:00 (segundos desde medianoche)
const DIA_FIN = 24 * 3600;          // 00:00 del día siguiente
const FACTOR_BASE = 30;             // 1 s real = 30 s de juego a ×1 (como Global.FACTOR_BASE_TIEMPO)
const PAX_POR_TREN = 300;           // pasajeros supuestos por tren, para el KPI minutos·pasajero
const INCIDENCIA_MIN = 5;           // minutos de retraso por incidencia inyectada

// ---- Estado global ------------------------------------------------------------------------
const S = {
  net: null,                 // topología
  trains: [],                // horarios (con posiciones de mundo precalculadas)
  stationById: new Map(),    // id -> estación (+ x,y de mundo)
  cam: { x: 0, y: 0, zoom: 1 },
  simT: DIA_INICIO,
  speed: 0,                  // multiplicador (0 = pausa)
  meanLatRad: 0,
  activos: 0,
  kpi: 0,                    // minutos·pasajero acumulados
  incidencias: [],           // [{t, texto}]
  retrasoTren: new Map(),    // train -> minutos de retraso ya contabilizados
  hoverSt: null,
  selSt: null,
};

const canvas = document.getElementById("map");
const ctx = canvas.getContext("2d");
const $ = (id) => document.getElementById(id);

// ---- Proyección geográfica (equirectangular, corregida por latitud media) -----------------
function worldOf(lat, lon) {
  return { x: lon * Math.cos(S.meanLatRad), y: -lat };
}
function toScreen(wx, wy) {
  return {
    x: (wx - S.cam.x) * S.cam.zoom + canvas.clientWidth / 2,
    y: (wy - S.cam.y) * S.cam.zoom + canvas.clientHeight / 2,
  };
}
function fromScreen(sx, sy) {
  return {
    x: (sx - canvas.clientWidth / 2) / S.cam.zoom + S.cam.x,
    y: (sy - canvas.clientHeight / 2) / S.cam.zoom + S.cam.y,
  };
}

// ---- Carga de datos -----------------------------------------------------------------------
async function cargarRed() {
  const r = await fetch("/api/game/network");
  S.net = await r.json();
  // Latitud media para la proyección.
  const lats = S.net.stations.map((s) => s.lat);
  S.meanLatRad = ((lats.reduce((a, b) => a + b, 0) / lats.length) * Math.PI) / 180;
  // Posiciones de mundo de cada estación.
  S.stationById.clear();
  for (const st of S.net.stations) {
    const w = worldOf(st.lat, st.lon);
    S.stationById.set(st.id, Object.assign({}, st, w));
  }
  ajustarCamara();
  poblarFiltroLineas();
  pintarLeyenda();
  $("netInfo").textContent =
    `${S.net.n_stations} estaciones · ${S.net.n_lines} líneas · datos GTFS (Fomento_Transit).`;
}

async function cargarHorarios() {
  const source = $("source").value;
  const line = $("lineFilter").value;
  $("netInfo").textContent = "Cargando horarios…";
  const q = new URLSearchParams({ source });
  if (line) q.set("line", line);
  const r = await fetch("/api/game/schedule?" + q.toString());
  const data = await r.json();
  // Precalcular, por tren, la lista de puntos con posición de mundo y tiempos.
  S.trains = [];
  for (const t of data.trains) {
    const pts = [];
    for (const st of t.stops) {
      const s = S.stationById.get(st.s);
      if (!s) continue; // parada sin coordenadas → se omite del trazado
      pts.push({ x: s.x, y: s.y, a: st.a, d: st.d });
    }
    if (pts.length < 2) continue;
    S.trains.push({
      train: t.train, line: t.line, color: colorDe(t.line),
      pts, dep0: pts[0].d, arrN: pts[pts.length - 1].a, shift: 0,
    });
  }
  S.kpi = 0; S.incidencias = []; S.retrasoTren.clear();
  $("incList").innerHTML = "";
  actualizarKpi();
  $("netInfo").textContent =
    `${S.net.n_stations} estaciones · ${S.net.n_lines} líneas · ${S.trains.length} trenes ` +
    `(${data.source === "optimized" ? "horario optimizado" : "GTFS programado"}, ${data.service_id}).`;
}

function colorDe(line) {
  const l = S.net.lines.find((x) => x.line === line);
  return l ? l.color : "#5d6b78";
}

// ---- Cámara -------------------------------------------------------------------------------
function ajustarCamara() {
  let minx = Infinity, miny = Infinity, maxx = -Infinity, maxy = -Infinity;
  for (const s of S.stationById.values()) {
    minx = Math.min(minx, s.x); maxx = Math.max(maxx, s.x);
    miny = Math.min(miny, s.y); maxy = Math.max(maxy, s.y);
  }
  S.cam.x = (minx + maxx) / 2;
  S.cam.y = (miny + maxy) / 2;
  const w = canvas.clientWidth, h = canvas.clientHeight;
  const zx = (w * 0.9) / Math.max(maxx - minx, 1e-6);
  const zy = (h * 0.9) / Math.max(maxy - miny, 1e-6);
  S.cam.zoom = Math.min(zx, zy);
}

// ---- Interpolación de posición del tren según el horario ----------------------------------
function posTren(t, ahora) {
  const pts = t.pts;
  const sh = t.shift;
  if (ahora < pts[0].d + sh || ahora > pts[pts.length - 1].a + sh) return null; // no está en circulación
  for (let i = 0; i < pts.length - 1; i++) {
    const dep = pts[i].d + sh, arrNext = pts[i + 1].a + sh, arr = pts[i].a + sh;
    if (ahora >= arr && ahora <= dep) return { x: pts[i].x, y: pts[i].y }; // parado (dwell)
    if (ahora >= dep && ahora <= arrNext) {
      const f = arrNext > dep ? (ahora - dep) / (arrNext - dep) : 0;
      return { x: pts[i].x + (pts[i + 1].x - pts[i].x) * f, y: pts[i].y + (pts[i + 1].y - pts[i].y) * f };
    }
  }
  return { x: pts[pts.length - 1].x, y: pts[pts.length - 1].y };
}

// ---- Dibujo -------------------------------------------------------------------------------
function dibujar() {
  const w = canvas.clientWidth, h = canvas.clientHeight;
  ctx.clearRect(0, 0, w, h);
  if (!S.net) return;

  // Trazado de líneas (una polilínea por sentido, con su color oficial).
  ctx.lineWidth = Math.max(1.5, Math.min(4, S.cam.zoom * 0.02));
  ctx.lineJoin = "round";
  for (const l of S.net.lines) {
    ctx.strokeStyle = l.color;
    for (const d of l.directions) {
      ctx.beginPath();
      let first = true;
      for (const id of d.stations) {
        const s = S.stationById.get(id);
        if (!s) continue;
        const p = toScreen(s.x, s.y);
        if (first) { ctx.moveTo(p.x, p.y); first = false; } else ctx.lineTo(p.x, p.y);
      }
      ctx.stroke();
      break; // basta un sentido para el trazado esquemático
    }
  }

  // Estaciones.
  for (const s of S.stationById.values()) {
    const p = toScreen(s.x, s.y);
    if (p.x < -20 || p.x > w + 20 || p.y < -20 || p.y > h + 20) continue;
    const key = s.tracks >= 4;
    const r = key ? 4.5 : 2.6;
    ctx.beginPath();
    ctx.arc(p.x, p.y, s === S.selSt ? r + 2 : r, 0, Math.PI * 2);
    ctx.fillStyle = s === S.selSt || s === S.hoverSt ? "#d2231b" : (key ? "#2b333b" : "#8894a0");
    ctx.fill();
    if (key) {
      ctx.strokeStyle = "#fff"; ctx.lineWidth = 1; ctx.stroke();
    }
  }

  // Trenes en circulación.
  S.activos = 0;
  for (const t of S.trains) {
    const pos = posTren(t, S.simT);
    if (!pos) continue;
    S.activos++;
    const p = toScreen(pos.x, pos.y);
    const late = S.retrasoTren.has(t.train);
    ctx.beginPath();
    ctx.arc(p.x, p.y, 4, 0, Math.PI * 2);
    ctx.fillStyle = t.color;
    ctx.fill();
    ctx.lineWidth = 1.2;
    ctx.strokeStyle = late ? "#d2231b" : "#1b2127";
    ctx.stroke();
  }
  $("kActive").textContent = S.activos;
}

// ---- Bucle principal ----------------------------------------------------------------------
let ultimo = performance.now();
function bucle(now) {
  const dtReal = Math.min(0.1, (now - ultimo) / 1000);
  ultimo = now;
  if (S.speed > 0) {
    S.simT += dtReal * FACTOR_BASE * S.speed;
    if (S.simT >= DIA_FIN) S.simT = DIA_INICIO; // reinicia la jornada
  }
  $("clock").textContent = hhmm(S.simT);
  dibujar();
  requestAnimationFrame(bucle);
}

function hhmm(seg) {
  const s = Math.floor(seg) % 86400;
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60);
  return String(h).padStart(2, "0") + ":" + String(m).padStart(2, "0");
}

// ---- KPI e incidencias --------------------------------------------------------------------
function actualizarKpi() { $("kDelay").textContent = Math.round(S.kpi).toLocaleString("es-ES"); }

function inyectarIncidencia() {
  const activos = S.trains.filter((t) => posTren(t, S.simT));
  if (!activos.length) { flashInc("No hay trenes en circulación ahora mismo."); return; }
  const t = activos[Math.floor(Math.random() * activos.length)];
  t.shift += INCIDENCIA_MIN * 60;
  S.retrasoTren.set(t.train, (S.retrasoTren.get(t.train) || 0) + INCIDENCIA_MIN);
  S.kpi += INCIDENCIA_MIN * PAX_POR_TREN;
  actualizarKpi();
  flashInc(`${hhmm(S.simT)} · tren ${t.train} (${t.line}): +${INCIDENCIA_MIN} min`);
}
function flashInc(texto) {
  const li = document.createElement("li");
  li.textContent = texto;
  const ul = $("incList");
  ul.insertBefore(li, ul.firstChild);
  while (ul.children.length > 8) ul.removeChild(ul.lastChild);
}

// ---- Leyenda y filtro ---------------------------------------------------------------------
function pintarLeyenda() {
  const box = $("legend");
  box.innerHTML = "";
  for (const l of S.net.lines) {
    const row = document.createElement("div");
    row.className = "row";
    row.innerHTML = `<span class="sw" style="background:${l.color}"></span><b>${l.line}</b>`;
    box.appendChild(row);
  }
}
function poblarFiltroLineas() {
  const sel = $("lineFilter");
  for (const l of S.net.lines) {
    const o = document.createElement("option");
    o.value = l.line; o.textContent = l.line;
    sel.appendChild(o);
  }
}

// ---- Interacción con estaciones -----------------------------------------------------------
function estacionEn(sx, sy) {
  let best = null, bestD = 12 * 12;
  for (const s of S.stationById.values()) {
    const p = toScreen(s.x, s.y);
    const dx = p.x - sx, dy = p.y - sy, d = dx * dx + dy * dy;
    if (d < bestD) { bestD = d; best = s; }
  }
  return best;
}
function mostrarEstacion(s) {
  S.selSt = s;
  $("stationBox").hidden = false;
  $("stName").textContent = s.name;
  $("stInfo").innerHTML =
    `${s.tracks} vía${s.tracks === 1 ? "" : "s"} · líneas: ${s.lines.join(", ") || "—"}`;
}

// ---- Eventos de ratón (pan / zoom / hover / clic) -----------------------------------------
let arrastrando = false, movido = false, lx = 0, ly = 0;
canvas.addEventListener("mousedown", (e) => { arrastrando = true; movido = false; lx = e.clientX; ly = e.clientY; });
window.addEventListener("mouseup", () => { arrastrando = false; });
window.addEventListener("mousemove", (e) => {
  const rect = canvas.getBoundingClientRect();
  const mx = e.clientX - rect.left, my = e.clientY - rect.top;
  if (arrastrando) {
    movido = true;
    S.cam.x -= (e.clientX - lx) / S.cam.zoom;
    S.cam.y -= (e.clientY - ly) / S.cam.zoom;
    lx = e.clientX; ly = e.clientY;
    $("tip").hidden = true;
    return;
  }
  if (mx < 0 || my < 0 || mx > rect.width || my > rect.height) { $("tip").hidden = true; S.hoverSt = null; return; }
  const s = estacionEn(mx, my);
  S.hoverSt = s;
  const tip = $("tip");
  if (s) {
    tip.hidden = false;
    tip.textContent = `${s.name} · ${s.tracks} vías · ${s.lines.join(",")}`;
    tip.style.left = e.clientX + "px";
    tip.style.top = e.clientY + "px";
  } else tip.hidden = true;
});
canvas.addEventListener("click", (e) => {
  if (movido) return;
  const rect = canvas.getBoundingClientRect();
  const s = estacionEn(e.clientX - rect.left, e.clientY - rect.top);
  if (s) mostrarEstacion(s);
});
canvas.addEventListener("wheel", (e) => {
  e.preventDefault();
  const rect = canvas.getBoundingClientRect();
  const mx = e.clientX - rect.left, my = e.clientY - rect.top;
  const antes = fromScreen(mx, my);
  const factor = e.deltaY < 0 ? 1.12 : 1 / 1.12;
  S.cam.zoom = Math.max(1, Math.min(20000, S.cam.zoom * factor));
  const despues = fromScreen(mx, my);
  S.cam.x += antes.x - despues.x;
  S.cam.y += antes.y - despues.y;
}, { passive: false });

// ---- Controles de la barra ----------------------------------------------------------------
$("speeds").addEventListener("click", (e) => {
  const b = e.target.closest("button"); if (!b) return;
  S.speed = Number(b.dataset.speed);
  document.querySelectorAll("#speeds button").forEach((x) => x.classList.toggle("active", x === b));
});
$("reload").addEventListener("click", cargarHorarios);
$("source").addEventListener("change", cargarHorarios);
$("lineFilter").addEventListener("change", cargarHorarios);
$("incident").addEventListener("click", inyectarIncidencia);

// ---- Redimensionado del canvas (nítido en pantallas HiDPI) --------------------------------
function redimensionar() {
  const dpr = window.devicePixelRatio || 1;
  canvas.width = canvas.clientWidth * dpr;
  canvas.height = canvas.clientHeight * dpr;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
}
window.addEventListener("resize", redimensionar);

// ---- Arranque -----------------------------------------------------------------------------
async function iniciar() {
  redimensionar();
  // Permite abrir el juego ya en un origen de horarios concreto (p. ej. /game?source=optimized
  // desde el botón del panel de análisis, para cargar automáticamente los horarios optimizados).
  const src = new URLSearchParams(location.search).get("source");
  if (src === "optimized" || src === "gtfs") $("source").value = src;
  try {
    await cargarRed();
    await cargarHorarios();
  } catch (e) {
    $("netInfo").textContent = "Error cargando datos: " + e;
  }
  requestAnimationFrame(bucle);
}
iniciar();
