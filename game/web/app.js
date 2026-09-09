// Rodalies Simulator — juego web (réplica de las mecánicas del simulador Godot).
//
// Mapa satélite (Leaflet + Esri World Imagery) alimentado desde el GTFS (Fomento_Transit):
//   /api/game/network  → estaciones, líneas (color oficial), CANTONES (aristas) y vía única.
//   /api/game/schedule → horarios del día (GTFS u optimizados).
//
// Replica: dibujo por cantones reales (corrige las líneas mal trazadas), semáforos de tramo
// (Automático/Manual verde/Manual rojo) que RETIENEN a los trenes, enclavamiento de estación
// (vías, semáforos internos por lado, vía principal, ocupación), reloj de jornada con las
// velocidades de Godot, panel de trenes (filtro/orden/ir), generador de incidencias
// (tipo/dónde/cuándo) y KPI de retraso (pax·min). El Godot original queda en game/godot-original/.

"use strict";

// ---- Constantes (de Global.gd) ------------------------------------------------------------
const DIA_INICIO = 5 * 3600, DIA_FIN = 24 * 3600, FACTOR_BASE = 30;
const VELOCIDADES = { "PAUSA": 0, "X0.05": 1 / 30, "X0.5": 10 / 30, "X1": 20 / 30, "X2": 40 / 30, "X5": 100 / 30, "X25": 500 / 30 };
const PERFIL = [[5, .05], [6, .3], [7, .9], [8, 1], [9, .8], [9.5, .4], [12, .35], [15, .35], [17, .6], [18, .9], [18.5, 1], [19.5, .7], [21, .3], [22.5, .15], [24, .03]];
const NOMINAL_PAX = 300;   // pasajeros nominales por tren para el KPI (Godot usa la ocupación real por lotes)

// ---- Estado -------------------------------------------------------------------------------
const S = {
  net: null, trains: [], stationById: new Map(), edgeLines: new Map(),
  simT: DIA_INICIO, speedKey: "PAUSA", speed: 0,
  cantonMode: new Map(),   // "A>B" -> 'AUTO'|'MV'|'MR'
  occ: new Map(),          // "A>B" -> id_tren que lo ocupa (recalculado por frame)
  interlocks: new Map(),   // id_estacion -> {tracks,mainA,mainB,green[[A,B]],occ[]}
  incidentes: [],          // {tipo,nombre,kind,scope,target,label,whenSec,durSec,estado}
  kpi: 0, activos: 0, selSt: null, finished: false,
};
const $ = (id) => document.getElementById(id);

// ---- Mapa ---------------------------------------------------------------------------------
const map = L.map("map", { minZoom: 8, maxZoom: 15, zoomControl: true });
L.tileLayer("https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}",
  { maxZoom: 19, attribution: "Esri, Maxar, Earthstar Geographics · GTFS Rodalies" }).addTo(map);
map.setView([41.7, 1.9], 8);
const lineLayer = L.layerGroup().addTo(map);
const stationLayer = L.layerGroup().addTo(map);
const signalLayer = L.layerGroup().addTo(map);
const trainLayer = L.layerGroup().addTo(map);
const trainMarkers = new Map();

// ---- Utilidades ---------------------------------------------------------------------------
const hhmm = (s) => { s = Math.floor(s) % 86400; return String(Math.floor(s / 3600)).padStart(2, "0") + ":" + String(Math.floor(s % 3600 / 60)).padStart(2, "0"); };
const key = (a, b) => a + ">" + b;
function demanda(t) { const h = t / 3600; if (h <= PERFIL[0][0]) return PERFIL[0][1]; for (let i = 1; i < PERFIL.length; i++) { if (h <= PERFIL[i][0]) { const [h0, v0] = PERFIL[i - 1], [h1, v1] = PERFIL[i]; return v0 + (v1 - v0) * (h - h0) / (h1 - h0); } } return .03; }
function colorLinea(l) { const x = S.net.lines.find((z) => z.line === l); return x ? x.color : "#888"; }

// ---- Carga de la red ----------------------------------------------------------------------
async function cargarRed() {
  S.net = await (await fetch("/api/game/network")).json();
  S.stationById.clear();
  for (const st of S.net.stations) S.stationById.set(st.id, st);
  S.edgeLines.clear();
  for (const e of S.net.edges) S.edgeLines.set(key(e.from, e.to), e.lines);

  // Trazado por CANTONES: cada arista se pinta como su propio segmento con el color de su
  // línea (o la primera si varias) — cubre toda sección con tráfico y evita rectas erróneas.
  lineLayer.clearLayers();
  const bounds = [];
  for (const e of S.net.edges) {
    const a = S.stationById.get(e.from), b = S.stationById.get(e.to);
    if (!a || !b) continue;
    L.polyline([[a.lat, a.lon], [b.lat, b.lon]], { color: colorLinea(e.lines[0]), weight: 3, opacity: .9 }).addTo(lineLayer);
    bounds.push([a.lat, a.lon], [b.lat, b.lon]);
  }
  // Estaciones.
  stationLayer.clearLayers();
  for (const s of S.stationById.values()) {
    const kv = s.tracks >= 4;
    const m = L.circleMarker([s.lat, s.lon], { radius: kv ? 5 : 3, color: "#fff", weight: 1, fillColor: kv ? "#830065" : "#26272d", fillOpacity: .95 });
    m.bindTooltip(`${s.name} · ${s.tracks} vías · ${s.lines.join(",")}`);
    m.on("click", () => abrirEstacion(s));
    m.addTo(stationLayer);
  }
  if (bounds.length) { const bb = L.latLngBounds(bounds); map.fitBounds(bb, { padding: [20, 20] }); map.setMaxBounds(bb.pad(.5)); }
  pintarLeyenda(); poblarSelectores();
  $("netInfo").textContent = `${S.net.n_stations} estaciones · ${S.net.n_lines} líneas · ${S.net.edges.length} cantones · GTFS.`;
}

async function cargarHorarios() {
  const source = $("source").value, line = $("lineFilter").value;
  const q = new URLSearchParams({ source }); if (line) q.set("line", line);
  const data = await (await fetch("/api/game/schedule?" + q.toString())).json();
  S.trains = [];
  for (const t of data.trains) {
    const pts = [];
    for (const st of t.stops) { const s = S.stationById.get(st.s); if (s) pts.push({ id: st.s, lat: s.lat, lon: s.lon, a: st.a, d: st.d }); }
    if (pts.length < 2) continue;
    const lat0 = pts[0].lat, latN = pts[pts.length - 1].lat;
    S.trains.push({ id: t.train, line: t.line, color: colorLinea(t.line), pts, shift: 0, side: latN >= lat0 ? "A" : "B", pax: 0 });
  }
  trainLayer.clearLayers(); trainMarkers.clear();
  S.kpi = 0; actualizarKpi();
  $("netInfo").textContent = `${S.net.n_stations} estaciones · ${S.net.n_lines} líneas · ${S.trains.length} trenes ` +
    `(${data.source === "optimized" ? "optimizado" : "GTFS"}, ${data.service_id}).`;
}

// ---- Enclavamiento (lazy) -----------------------------------------------------------------
function interlock(id) {
  let il = S.interlocks.get(id);
  if (!il) {
    const st = S.stationById.get(id); const n = st ? Math.max(1, st.tracks) : 1;
    il = { tracks: n, mainA: 0, mainB: Math.min(1, n - 1), green: [], occ: [] };
    for (let i = 0; i < n; i++) { il.green.push({ A: true, B: true }); il.occ.push(null); }
    S.interlocks.set(id, il);
  }
  return il;
}
function internoRojo(stId, side) { const il = interlock(stId); const via = side === "A" ? il.mainA : il.mainB; return !il.green[via][side]; }

// ---- Semáforos de tramo -------------------------------------------------------------------
function incidenteEdge(k, kind) { const now = S.simT; return S.incidentes.some((i) => i.estado === "activa" && i.kind === kind && i.scope === "edge" && i.target === k && now < i.whenSec + i.durSec); }
function cantonRojo(k, id_tren) {
  const mode = S.cantonMode.get(k) || "AUTO";
  if (mode === "MR") return true;
  if (incidenteEdge(k, "block")) return true;         // avería mecánica: bloqueo total
  if (mode === "MV") return false;
  const o = S.occ.get(k); return o != null && o !== id_tren;   // AUTO: rojo si otro tren lo ocupa
}
function estacionParada(stId) { const now = S.simT; return S.incidentes.some((i) => i.estado === "activa" && i.kind === "stop" && i.scope === "station" && i.target === stId && now < i.whenSec + i.durSec); }
function edgeLento(k) { const now = S.simT; return S.incidentes.some((i) => i.estado === "activa" && i.kind === "slow" && i.scope === "edge" && i.target === k && now < i.whenSec + i.durSec); }

// ---- Posición de un tren según su tiempo efectivo -----------------------------------------
function segmentoDe(t, te) {
  const p = t.pts;
  if (te < p[0].d || te > p[p.length - 1].a) return null;
  for (let i = 0; i < p.length - 1; i++) {
    if (te >= p[i].a && te <= p[i].d) return { i, moving: false };
    if (te >= p[i].d && te <= p[i + 1].a) return { i, moving: true };
  }
  return { i: p.length - 1, moving: false };
}
function posDe(t, seg, te) {
  const p = t.pts;
  if (!seg.moving) return [p[seg.i].lat, p[seg.i].lon];
  const A = p[seg.i], B = p[seg.i + 1], f = B.a > A.d ? (te - A.d) / (B.a - A.d) : 0;
  return [A.lat + (B.lat - A.lat) * f, A.lon + (B.lon - A.lon) * f];
}

// ---- Paso de simulación -------------------------------------------------------------------
function paso(dtReal) {
  const dtGame = dtReal * FACTOR_BASE * S.speed;
  S.simT += dtGame;
  if (S.simT >= DIA_FIN && !S.finished) { S.simT = DIA_FIN; finDelDia(); }
  activarIncidencias();

  // Pass 1: clasificar (sin mover) y calcular ocupación de cantones.
  S.occ.clear();
  const est = [];
  for (const t of S.trains) {
    const te = S.simT - t.shift;
    const seg = segmentoDe(t, te);
    if (!seg) { est.push(null); continue; }
    est.push({ te, seg });
    if (seg.moving) { const k = key(t.pts[seg.i].id, t.pts[seg.i + 1].id); if (!S.occ.has(k)) S.occ.set(k, t.id); }
  }
  // Pass 2: gating (congelar aumentando shift) + KPI.
  let n = 0;
  for (let idx = 0; idx < S.trains.length; idx++) {
    const t = S.trains[idx], e = est[idx]; if (!e) continue;
    n++;
    const p = t.pts, i = e.seg.i;
    let bloqueado = false;
    if (!e.seg.moving) {
      // Parado en estación i. Si va a partir (fin de dwell) comprobamos salida.
      if (i < p.length - 1 && e.te >= p[i].d - 0.5) {
        const k = key(p[i].id, p[i + 1].id);
        if (internoRojo(p[i].id, t.side) || cantonRojo(k, t.id)) bloqueado = true;
      }
      if (estacionParada(p[i].id)) bloqueado = true;  // avería de puertas: retenido en la estación
    } else {
      const k = key(p[i].id, p[i + 1].id);
      if (S.cantonMode.get(k) === "MR" || incidenteEdge(k, "block")) bloqueado = true;   // rojo manual/bloqueo a mitad
      else if (edgeLento(k) && (Math.floor(S.simT) % 2 === 0)) bloqueado = true;          // catenaria: media velocidad
    }
    if (bloqueado && S.speed > 0) {
      t.shift += dtGame;
      const pax = Math.round(NOMINAL_PAX * demanda(S.simT));
      S.kpi += (dtGame / 60) * pax;
    }
  }
  S.activos = n;
}

// ---- Render de trenes ---------------------------------------------------------------------
function render() {
  const vivos = new Set();
  for (const t of S.trains) {
    const te = S.simT - t.shift, seg = segmentoDe(t, te);
    if (!seg) continue;
    vivos.add(t.id);
    const pos = posDe(t, seg, te), late = t.shift > 90;
    let m = trainMarkers.get(t.id);
    if (!m) { m = L.circleMarker(pos, { radius: 4.5, color: late ? "#ec6e15" : "#26272d", weight: 1.4, fillColor: t.color, fillOpacity: 1 }); m.bindTooltip(""); m.addTo(trainLayer); trainMarkers.set(t.id, m); }
    else { m.setLatLng(pos); m.setStyle({ color: late ? "#ec6e15" : "#26272d" }); }
    m.setTooltipContent(`${t.line} · tren ${t.id}<br>Destino: ${nombre(t.pts[t.pts.length - 1].id)}<br>Retraso: ${t.shift > 60 ? "+" + Math.round(t.shift / 60) + " min" : "puntual"}`);
  }
  for (const [id, m] of trainMarkers) if (!vivos.has(id)) { trainLayer.removeLayer(m); trainMarkers.delete(id); }
  $("kActive").textContent = S.activos;
  $("clock").textContent = hhmm(S.simT);
  if (S.selSt) refrescarEstacion();
}
const nombre = (id) => { const s = S.stationById.get(id); return s ? s.name : id; };

// ---- Semáforos de tramo en el mapa (LOD: sólo al acercar y dentro de vista) ----------------
function pintarSemaforos() {
  signalLayer.clearLayers();
  if (!$("showSignals").checked || map.getZoom() < 12 || !S.net) return;
  const vb = map.getBounds();
  for (const e of S.net.edges) {
    const a = S.stationById.get(e.from), b = S.stationById.get(e.to); if (!a || !b) continue;
    const mid = [(a.lat + b.lat) / 2, (a.lon + b.lon) / 2];
    if (!vb.contains(mid)) continue;
    const k = key(e.from, e.to), rojo = cantonRojo(k, null), manual = (S.cantonMode.get(k) || "AUTO") !== "AUTO";
    const m = L.circleMarker(mid, { radius: 5, color: manual ? "#ffd43b" : "#fff", weight: manual ? 2 : 1, fillColor: rojo ? "#c5221f" : "#2f9e44", fillOpacity: 1 });
    m.bindTooltip(`${e.lines.join(",")} · ${nombre(e.from)} → ${nombre(e.to)}<br>${({ AUTO: "Automático", MV: "Manual verde", MR: "Manual rojo" })[S.cantonMode.get(k) || "AUTO"]}`);
    m.on("click", () => { const cur = S.cantonMode.get(k) || "AUTO"; S.cantonMode.set(k, cur === "AUTO" ? "MV" : cur === "MV" ? "MR" : "AUTO"); pintarSemaforos(); });
    m.addTo(signalLayer);
  }
}
map.on("moveend zoomend", pintarSemaforos);

// ---- Panel de enclavamiento de estación ---------------------------------------------------
function abrirEstacion(s) { S.selSt = s; $("stationBox").hidden = false; refrescarEstacion(); }
function refrescarEstacion() {
  const s = S.selSt; if (!s) return;
  const il = interlock(s.id);
  // Ocupación en vivo: trenes parados aquí ocupan la vía principal de su lado.
  for (let v = 0; v < il.tracks; v++) il.occ[v] = null;
  for (const t of S.trains) { const te = S.simT - t.shift, seg = segmentoDe(t, te); if (seg && !seg.moving && t.pts[seg.i].id === s.id) { il.occ[t.side === "A" ? il.mainA : il.mainB] = t.id; } }
  $("stName").textContent = s.name;
  $("stMeta").textContent = `${il.tracks} vía(s) · líneas ${s.lines.join(", ")}`;
  // Próximo tren que llega.
  let prox = null, mejor = Infinity;
  for (const t of S.trains) { const te = S.simT - t.shift, seg = segmentoDe(t, te); if (!seg) continue; for (let j = Math.max(seg.i, 0); j < t.pts.length; j++) { if (t.pts[j].id === s.id) { const eta = t.pts[j].a - te; if (eta >= 0 && eta < mejor) { mejor = eta; prox = t; } break; } } }
  $("stNext").innerHTML = prox ? `<span class="stNextline">Próximo: <b>${prox.line}</b> tren ${prox.id} en ~${Math.round(mejor / 60)} min</span>` : `<span class="muted">Sin trenes próximos.</span>`;
  // Selectores de vía principal + filas de vía con semáforos internos.
  let html = `<div class="form" style="display:flex;gap:8px;margin:8px 0">
      <label style="flex:1">Ppal. A<select id="selMainA"></select></label>
      <label style="flex:1">Ppal. B<select id="selMainB"></select></label></div>`;
  for (let v = 0; v < il.tracks; v++) {
    const ga = il.green[v].A, gb = il.green[v].B, occ = il.occ[v];
    const tags = [il.mainA === v ? "◀A" : "", il.mainB === v ? "B▶" : ""].filter(Boolean).join(" ");
    html += `<div class="trackrow">
      <button class="sig ${ga ? "green" : "red"}" data-v="${v}" data-lado="A">A</button>
      <div class="viac ${(il.mainA === v || il.mainB === v) ? "main" : ""} ${occ ? "occ" : ""}">Vía ${v + 1} ${tags}${occ ? " · " + occ : ""}</div>
      <button class="sig ${gb ? "green" : "red"}" data-v="${v}" data-lado="B">B</button>
    </div>`;
  }
  const box = $("stTracks"); box.innerHTML = html;
  const opt = (sel, val) => { let o = ""; for (let v = 0; v < il.tracks; v++) o += `<option value="${v}"${v === val ? " selected" : ""}>Vía ${v + 1}</option>`; sel.innerHTML = o; };
  opt($("selMainA"), il.mainA); opt($("selMainB"), il.mainB);
  $("selMainA").onchange = (e) => { il.mainA = +e.target.value; refrescarEstacion(); };
  $("selMainB").onchange = (e) => { il.mainB = +e.target.value; refrescarEstacion(); };
  box.querySelectorAll(".sig").forEach((b) => b.onclick = () => { const v = +b.dataset.v, l = b.dataset.lado; il.green[v][l] = !il.green[v][l]; refrescarEstacion(); });
}

// ---- Generador de incidencias -------------------------------------------------------------
const KIND = { averia_puertas: "stop", caida_catenaria: "slow", averia_mecanica: "block" };
const INCID = {
  averia_puertas: { nombre: "Avería de puertas", dur: 5 },
  caida_catenaria: { nombre: "Falta de tensión", dur: 30 },
  averia_mecanica: { nombre: "Avería mecánica grave", dur: 90 },
};
function poblarSelectores() {
  // Tipos.
  $("incType").innerHTML = Object.entries(INCID).map(([k, v]) => `<option value="${k}">${v.nombre} (${v.dur} min)</option>`).join("");
  // Líneas (leyenda + filtros).
  const ln = S.net.lines.map((l) => `<option value="${l.line}">${l.line}</option>`).join("");
  $("lineFilter").innerHTML = '<option value="">Todas las líneas</option>' + ln;
  $("tFiltLine").innerHTML = '<option value="">Todas</option>' + ln;
  rellenarDonde();
}
function rellenarDonde() {
  const scope = $("incScope").value, sel = $("incWhere");
  if (scope === "station") {
    sel.innerHTML = [...S.stationById.values()].sort((a, b) => a.name.localeCompare(b.name)).map((s) => `<option value="${s.id}">${s.name}</option>`).join("");
  } else {
    sel.innerHTML = S.net.edges.map((e) => `<option value="${key(e.from, e.to)}">${nombre(e.from)} → ${nombre(e.to)} (${e.lines.join(",")})</option>`).join("");
  }
}
function programarIncidencia(ahora) {
  const tipo = $("incType").value, scope = $("incScope").value, target = $("incWhere").value;
  const kind = KIND[tipo], durSec = INCID[tipo].dur * 60;
  let whenSec = S.simT;
  if (!ahora) { const [h, m] = $("incWhen").value.split(":").map(Number); whenSec = h * 3600 + m * 60; }
  const label = scope === "station" ? nombre(target) : target.split(">").map(nombre).join(" → ");
  S.incidentes.push({ tipo, nombre: INCID[tipo].nombre, kind, scope, target, label, whenSec, durSec, estado: whenSec <= S.simT ? "activa" : "programada" });
  refrescarIncidencias();
}
function activarIncidencias() {
  for (const i of S.incidentes) {
    if (i.estado === "programada" && S.simT >= i.whenSec) i.estado = "activa";
    if (i.estado === "activa" && S.simT >= i.whenSec + i.durSec) i.estado = "finalizada";
  }
}
function refrescarIncidencias() {
  $("incList").innerHTML = S.incidentes.slice().reverse().map((i) =>
    `<li class="${i.estado === "finalizada" ? "done" : ""}">${hhmm(i.whenSec)} · ${i.nombre} · ${i.label} <span class="muted">[${i.estado}]</span></li>`).join("");
}

// ---- Panel de trenes ----------------------------------------------------------------------
function refrescarTrenes() {
  if ($("trenesPanel").hidden) return;
  const fl = $("tFiltLine").value, onlyLate = $("tOnlyLate").checked, sort = $("tSort").value;
  const filas = [];
  for (const t of S.trains) {
    const te = S.simT - t.shift, seg = segmentoDe(t, te); if (!seg) continue;
    if (fl && t.line !== fl) continue;
    const retr = Math.round(t.shift / 60); if (onlyLate && retr <= 0) continue;
    const ubic = seg.moving ? `${nombre(t.pts[seg.i].id)} → ${nombre(t.pts[seg.i + 1].id)}` : `en ${nombre(t.pts[seg.i].id)}`;
    filas.push({ t, retr, ubic });
  }
  filas.sort((x, y) => sort === "delay" ? y.retr - x.retr : x.t.line.localeCompare(y.t.line));
  $("trenesList").innerHTML = filas.slice(0, 200).map((f) =>
    `<div class="t"><span class="badge" style="background:${f.t.color}">${f.t.line}</span>
      <span>${f.ubic} → ${nombre(f.t.pts[f.t.pts.length - 1].id)}</span>
      <span class="late">${f.retr > 0 ? "+" + f.retr + "m" : "·"}</span>
      <button class="goto" data-id="${f.t.id}">Ir</button></div>`).join("");
  $("trenesList").querySelectorAll(".goto").forEach((b) => b.onclick = () => irAlTren(b.dataset.id));
}
function irAlTren(id) {
  const t = S.trains.find((x) => x.id === id); if (!t) return;
  const te = S.simT - t.shift, seg = segmentoDe(t, te); if (!seg) return;
  map.setView(posDe(t, seg, te), Math.max(map.getZoom(), 12), { animate: true });
}

// ---- Fin del día --------------------------------------------------------------------------
function finDelDia() {
  S.finished = true; setSpeed("PAUSA");
  const retrasados = S.trains.filter((t) => t.shift > 60).length;
  $("finStats").innerHTML = `
    <div class="stat"><span>KPI de retraso</span><b>${Math.round(S.kpi).toLocaleString("es-ES")} pax·min</b></div>
    <div class="stat"><span>Trenes con retraso</span><b>${retrasados}</b></div>
    <div class="stat"><span>Incidencias del día</span><b>${S.incidentes.length}</b></div>`;
  $("finDia").hidden = false;
}

// ---- KPI, leyenda -------------------------------------------------------------------------
function actualizarKpi() { $("kDelay").textContent = Math.round(S.kpi).toLocaleString("es-ES"); }
function pintarLeyenda() { $("legend").innerHTML = S.net.lines.map((l) => `<div class="row"><span class="sw" style="background:${l.color}"></span>${l.line}</div>`).join(""); }

// ---- Controles ----------------------------------------------------------------------------
function setSpeed(k) { S.speedKey = k; S.speed = VELOCIDADES[k]; document.querySelectorAll("#speeds button").forEach((b) => b.classList.toggle("active", b.dataset.speed === k)); }
$("speeds").addEventListener("click", (e) => { const b = e.target.closest("button"); if (b) setSpeed(b.dataset.speed); });
$("reload").addEventListener("click", cargarHorarios);
$("source").addEventListener("change", cargarHorarios);
$("lineFilter").addEventListener("change", cargarHorarios);
$("showSignals").addEventListener("change", pintarSemaforos);
$("incScope").addEventListener("change", rellenarDonde);
$("incNow").addEventListener("click", () => programarIncidencia(true));
$("incSchedule").addEventListener("click", () => programarIncidencia(false));
$("btnTrenes").addEventListener("click", () => { const p = $("trenesPanel"); p.hidden = !p.hidden; refrescarTrenes(); });
$("trenesClose").addEventListener("click", () => $("trenesPanel").hidden = true);
["tFiltLine", "tOnlyLate", "tSort"].forEach((id) => $(id).addEventListener("change", refrescarTrenes));
$("finRepetir").addEventListener("click", () => location.reload());
window.addEventListener("resize", () => map.invalidateSize());

// ---- Bucle --------------------------------------------------------------------------------
let ultimo = performance.now(), acumUI = 0;
function bucle(now) {
  const dt = Math.min(0.1, (now - ultimo) / 1000); ultimo = now;
  if (S.speed > 0 && !S.finished) paso(dt);
  render();
  acumUI += dt;
  if (acumUI > 0.5) { acumUI = 0; actualizarKpi(); refrescarIncidencias(); refrescarTrenes(); }
  requestAnimationFrame(bucle);
}

// ---- Arranque -----------------------------------------------------------------------------
async function iniciar() {
  const src = new URLSearchParams(location.search).get("source");
  if (src === "optimized" || src === "gtfs") $("source").value = src;
  try { await cargarRed(); await cargarHorarios(); pintarSemaforos(); setTimeout(() => map.invalidateSize(), 120); }
  catch (e) { $("netInfo").textContent = "Error cargando datos: " + e; }
  requestAnimationFrame(bucle);
}
iniciar();
