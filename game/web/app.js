// Rodalies Simulator — juego web (Fase 3).
//
// Simulador de red web-nativo sobre un mapa satélite (Leaflet + ortofoto Esri World Imagery,
// sin etiquetas de ciudades), alimentado dinámicamente desde el GTFS de Fomento_Transit vía la
// API del servidor Rust:
//   * /api/game/network             → topología (estaciones, secuencias reales, color oficial)
//   * /api/game/schedule?source=…   → horarios del día (GTFS «tal cual» u optimizados)
//
// Mecánicas: trenes circulando por su ruta real siguiendo el horario, reloj de jornada
// 05:00→00:00 con velocidad (Pausa/×1/×2/×5/×20), desplazamiento y zoom acotados a Cataluña,
// información por estación, e incidencias con KPI de retraso (minutos·pasajero). El proyecto
// Godot original se conserva intacto en game/godot-original/.

"use strict";

const DIA_INICIO = 5 * 3600;     // 05:00 (segundos desde medianoche)
const DIA_FIN = 24 * 3600;       // 00:00 del día siguiente
const FACTOR_BASE = 30;          // 1 s real = 30 s de juego a ×1
const PAX_POR_TREN = 300;        // pasajeros supuestos por tren (KPI minutos·pasajero)
const INCIDENCIA_MIN = 5;        // minutos de retraso por incidencia inyectada

const S = {
  net: null,
  trains: [],
  stationById: new Map(),   // id -> {name,lat,lon,tracks,lines}
  simT: DIA_INICIO,
  speed: 0,
  activos: 0,
  kpi: 0,
  retrasoTren: new Map(),   // train -> minutos ya contabilizados
  selSt: null,
};

const $ = (id) => document.getElementById(id);

// ---- Mapa (Leaflet + satélite) ------------------------------------------------------------
const map = L.map("map", { minZoom: 8, maxZoom: 14, zoomControl: true, attributionControl: true });
L.tileLayer(
  "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}",
  { maxZoom: 19, attribution: "Imagen: Esri, Maxar, Earthstar Geographics · datos: GTFS Rodalies" }
).addTo(map);
map.setView([41.7, 1.9], 8); // Cataluña, vista inicial provisional

const lineLayer = L.layerGroup().addTo(map);
const stationLayer = L.layerGroup().addTo(map);
const trainLayer = L.layerGroup().addTo(map);
const trainMarkers = new Map(); // train -> L.circleMarker

// ---- Carga de datos -----------------------------------------------------------------------
async function cargarRed() {
  const r = await fetch("/api/game/network");
  S.net = await r.json();
  S.stationById.clear();
  for (const st of S.net.stations) S.stationById.set(st.id, st);

  // Trazado de líneas (una polilínea por sentido, con su color oficial).
  lineLayer.clearLayers();
  const bounds = [];
  for (const l of S.net.lines) {
    const d = l.directions[0];
    if (!d) continue;
    const pts = [];
    for (const id of d.stations) {
      const s = S.stationById.get(id);
      if (s) { pts.push([s.lat, s.lon]); bounds.push([s.lat, s.lon]); }
    }
    if (pts.length >= 2) {
      L.polyline(pts, { color: l.color, weight: 3, opacity: 0.95 }).addTo(lineLayer);
    }
  }

  // Estaciones.
  stationLayer.clearLayers();
  for (const s of S.stationById.values()) {
    const key = s.tracks >= 4;
    const m = L.circleMarker([s.lat, s.lon], {
      radius: key ? 5 : 3,
      color: "#ffffff", weight: 1,
      fillColor: key ? "#830065" : "#26272d", fillOpacity: 0.95,
    });
    m.bindTooltip(`${s.name} · ${s.tracks} vías · ${s.lines.join(",")}`);
    m.on("click", () => mostrarEstacion(s));
    m.addTo(stationLayer);
  }

  if (bounds.length) {
    const b = L.latLngBounds(bounds);
    map.fitBounds(b, { padding: [20, 20] });
    map.setMaxBounds(b.pad(0.5)); // desplazamiento acotado a la zona de la red
  }
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
  const data = await (await fetch("/api/game/schedule?" + q.toString())).json();

  S.trains = [];
  for (const t of data.trains) {
    const pts = [];
    for (const st of t.stops) {
      const s = S.stationById.get(st.s);
      if (!s) continue;
      pts.push({ lat: s.lat, lon: s.lon, a: st.a, d: st.d });
    }
    if (pts.length < 2) continue;
    S.trains.push({ train: t.train, line: t.line, color: colorDe(t.line), pts, shift: 0 });
  }
  // Limpieza de estado por recarga.
  trainLayer.clearLayers(); trainMarkers.clear();
  S.kpi = 0; S.retrasoTren.clear(); $("incList").innerHTML = "";
  actualizarKpi();
  $("netInfo").textContent =
    `${S.net.n_stations} estaciones · ${S.net.n_lines} líneas · ${S.trains.length} trenes ` +
    `(${data.source === "optimized" ? "horario optimizado" : "GTFS programado"}, ${data.service_id}).`;
}

function colorDe(line) {
  const l = S.net.lines.find((x) => x.line === line);
  return l ? l.color : "#26272d";
}

// ---- Interpolación de posición del tren según el horario ----------------------------------
function posTren(t, ahora) {
  const pts = t.pts, sh = t.shift;
  if (ahora < pts[0].d + sh || ahora > pts[pts.length - 1].a + sh) return null;
  for (let i = 0; i < pts.length - 1; i++) {
    const arr = pts[i].a + sh, dep = pts[i].d + sh, arrNext = pts[i + 1].a + sh;
    if (ahora >= arr && ahora <= dep) return [pts[i].lat, pts[i].lon]; // parado (dwell)
    if (ahora >= dep && ahora <= arrNext) {
      const f = arrNext > dep ? (ahora - dep) / (arrNext - dep) : 0;
      return [pts[i].lat + (pts[i + 1].lat - pts[i].lat) * f, pts[i].lon + (pts[i + 1].lon - pts[i].lon) * f];
    }
  }
  return [pts[pts.length - 1].lat, pts[pts.length - 1].lon];
}

// ---- Actualización de los marcadores de tren ----------------------------------------------
function actualizarTrenes() {
  let n = 0;
  const vivos = new Set();
  for (const t of S.trains) {
    const pos = posTren(t, S.simT);
    if (!pos) continue;
    n++;
    vivos.add(t.train);
    const late = S.retrasoTren.has(t.train);
    let m = trainMarkers.get(t.train);
    if (!m) {
      m = L.circleMarker(pos, {
        radius: 4.5, color: late ? "#ec6e15" : "#26272d", weight: 1.4,
        fillColor: t.color, fillOpacity: 1,
      });
      m.bindTooltip(`Tren ${t.train} (${t.line})`);
      m.addTo(trainLayer);
      trainMarkers.set(t.train, m);
    } else {
      m.setLatLng(pos);
      if (late) m.setStyle({ color: "#ec6e15", weight: 2 });
    }
  }
  // Retirar trenes que ya no circulan.
  for (const [train, m] of trainMarkers) {
    if (!vivos.has(train)) { trainLayer.removeLayer(m); trainMarkers.delete(train); }
  }
  S.activos = n;
  $("kActive").textContent = n;
}

// ---- Bucle principal ----------------------------------------------------------------------
let ultimo = performance.now();
function bucle(now) {
  const dtReal = Math.min(0.1, (now - ultimo) / 1000);
  ultimo = now;
  if (S.speed > 0) {
    S.simT += dtReal * FACTOR_BASE * S.speed;
    if (S.simT >= DIA_FIN) S.simT = DIA_INICIO;
  }
  $("clock").textContent = hhmm(S.simT);
  if (S.net) actualizarTrenes();
  requestAnimationFrame(bucle);
}
function hhmm(seg) {
  const s = Math.floor(seg) % 86400;
  return String(Math.floor(s / 3600)).padStart(2, "0") + ":" + String(Math.floor((s % 3600) / 60)).padStart(2, "0");
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

// ---- Leyenda, filtro, panel de estación ---------------------------------------------------
function pintarLeyenda() {
  const box = $("legend"); box.innerHTML = "";
  for (const l of S.net.lines) {
    const row = document.createElement("div");
    row.className = "row";
    row.innerHTML = `<span class="sw" style="background:${l.color}"></span><b>${l.line}</b>`;
    box.appendChild(row);
  }
}
function poblarFiltroLineas() {
  const sel = $("lineFilter");
  sel.querySelectorAll("option:not([value=''])").forEach((o) => o.remove());
  for (const l of S.net.lines) {
    const o = document.createElement("option");
    o.value = l.line; o.textContent = l.line; sel.appendChild(o);
  }
}
function mostrarEstacion(s) {
  S.selSt = s;
  $("stationBox").hidden = false;
  $("stName").textContent = s.name;
  $("stInfo").innerHTML = `${s.tracks} vía${s.tracks === 1 ? "" : "s"} · líneas: ${s.lines.join(", ") || "—"}`;
}

// ---- Controles ----------------------------------------------------------------------------
$("speeds").addEventListener("click", (e) => {
  const b = e.target.closest("button"); if (!b) return;
  S.speed = Number(b.dataset.speed);
  document.querySelectorAll("#speeds button").forEach((x) => x.classList.toggle("active", x === b));
});
$("reload").addEventListener("click", cargarHorarios);
$("source").addEventListener("change", cargarHorarios);
$("lineFilter").addEventListener("change", cargarHorarios);
$("incident").addEventListener("click", inyectarIncidencia);
window.addEventListener("resize", () => map.invalidateSize());

// ---- Arranque -----------------------------------------------------------------------------
async function iniciar() {
  const src = new URLSearchParams(location.search).get("source");
  if (src === "optimized" || src === "gtfs") $("source").value = src;
  try {
    await cargarRed();
    await cargarHorarios();
    setTimeout(() => map.invalidateSize(), 100);
  } catch (e) {
    $("netInfo").textContent = "Error cargando datos: " + e;
  }
  requestAnimationFrame(bucle);
}
iniciar();
