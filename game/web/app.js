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
  occ: new Map(),          // clave de recurso (canton dirigido o cantón único no dirigido) -> id_tren
  interlocks: new Map(),   // id_estacion -> {tracks,mainA,mainB,green[[A,B]],occ[]}
  incidentes: [],          // {tipo,nombre,kind,scope,target,label,whenSec,durSec,estado}
  singleTrack: new Set(),  // claves canónicas "idA|idB" (idA<idB) de tramos de vía única
  shearMap: new Map(),     // clave de recurso -> [claves de recurso en conflicto] (cizallamiento)
  singleTrackUsers: new Map(), // clave de recurso -> [{t,j}] trenes que la recorren (para prioridad)
  units: new Map(),        // id_unidad_física -> {line, parked:bool, atStation, freeAt}
  parked: [],              // unidades físicas actualmente apartadas (para dibujar)
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
const parkedLayer = L.layerGroup().addTo(map);
const trainMarkers = new Map();
const parkedMarkers = new Map();

// ---- Utilidades ---------------------------------------------------------------------------
const hhmm = (s) => { s = Math.floor(s) % 86400; return String(Math.floor(s / 3600)).padStart(2, "0") + ":" + String(Math.floor(s % 3600 / 60)).padStart(2, "0"); };
const key = (a, b) => a + ">" + b;
const canon = (a, b) => a < b ? a + "|" + b : b + "|" + a;
// Clave de RECURSO físico de un cantón: en vía única, ambos sentidos comparten la MISMA vía
// (bastón/testigo — no pueden cruzarse en plena vía), así que usan la clave no dirigida.
function resKey(a, b) { const c = canon(a, b); return S.singleTrack.has(c) ? c : key(a, b); }
function demanda(t) { const h = t / 3600; if (h <= PERFIL[0][0]) return PERFIL[0][1]; for (let i = 1; i < PERFIL.length; i++) { if (h <= PERFIL[i][0]) { const [h0, v0] = PERFIL[i - 1], [h1, v1] = PERFIL[i]; return v0 + (v1 - v0) * (h - h0) / (h1 - h0); } } return .03; }
function colorLinea(l) { const x = S.net.lines.find((z) => z.line === l); return x ? x.color : "#888"; }
function esRegional(l) { const x = S.net.lines.find((z) => z.line === l); return x ? !!x.regional : false; }

// ---- Carga de la red ----------------------------------------------------------------------
async function cargarRed() {
  S.net = await (await fetch("/api/game/network")).json();
  S.stationById.clear();
  for (const st of S.net.stations) S.stationById.set(st.id, st);
  S.edgeLines.clear();
  for (const e of S.net.edges) S.edgeLines.set(key(e.from, e.to), e.lines);

  // Vía única real: bastón/testigo — ambos sentidos comparten la MISMA vía física, así que
  // se indexan por clave canónica no dirigida (ver `resKey`).
  S.singleTrack.clear();
  for (const [a, b] of S.net.single_track) S.singleTrack.add(canon(a, b));

  // Cizallamientos: ocupar un cantón retiene en rojo el cantón con el que se cruza
  // físicamente (p. ej. el ramal único de R7 invadiendo la vía de R4 en Cerdanyola).
  S.shearMap.clear();
  const addShear = (k1, k2) => { if (!S.shearMap.has(k1)) S.shearMap.set(k1, []); S.shearMap.get(k1).push(k2); };
  for (const [[a1, b1], [a2, b2]] of S.net.shears || []) {
    const k1 = resKey(a1, b1), k2 = resKey(a2, b2);
    addShear(k1, k2); addShear(k2, k1);
  }

  // Trazado por CANTONES: cada arista se pinta como su propio segmento con el color de su
  // línea (o la primera si varias) — cubre toda sección con tráfico y evita rectas erróneas.
  // Vía única: trazo discontinuo (bastón único, sin cruces posibles en plena vía).
  lineLayer.clearLayers();
  const bounds = [];
  for (const e of S.net.edges) {
    const a = S.stationById.get(e.from), b = S.stationById.get(e.to);
    if (!a || !b) continue;
    const unica = S.singleTrack.has(canon(e.from, e.to));
    L.polyline([[a.lat, a.lon], [b.lat, b.lon]], {
      color: colorLinea(e.lines[0]), weight: unica ? 2.5 : 3, opacity: .9,
      dashArray: unica ? "6,5" : null,
    }).addTo(lineLayer);
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

// ---- Máquina de estados del material rodante: flota física persistente ---------------------
// Encadena servicios GTFS consecutivos del mismo origen/línea en unidades físicas con ID
// estable (aproximación greedy: mismo criterio que Godot — reutiliza la unidad libre más
// antigua en esa estación si el giro mínimo lo permite; si no, crea una unidad nueva).
const GIRO_MINIMO_SEG = 5 * 60;
function encadenarFlota() {
  S.units.clear();
  const porLinea = new Map();
  for (const t of S.trains) { if (!porLinea.has(t.line)) porLinea.set(t.line, []); porLinea.get(t.line).push(t); }
  let nextId = 1;
  for (const [line, trs] of porLinea) {
    trs.sort((a, b) => a.pts[0].d - b.pts[0].d);
    const libres = []; // {id, atStation, freeAt}
    for (const t of trs) {
      const origen = t.pts[0].id, salida = t.pts[0].d;
      const idx = libres.findIndex((u) => u.atStation === origen && u.freeAt <= salida);
      let unitId;
      if (idx >= 0) { unitId = libres[idx].id; libres.splice(idx, 1); }
      else { unitId = "U" + nextId++; S.units.set(unitId, { line, huecos: [] }); }
      t.unit = unitId;
      const destino = t.pts[t.pts.length - 1].id, llegada = t.pts[t.pts.length - 1].a;
      libres.push({ id: unitId, atStation: destino, freeAt: llegada + GIRO_MINIMO_SEG });
    }
    // Huecos entre servicios consecutivos de cada unidad (reposicionamiento/pernocta): si el
    // servicio termina y el siguiente de la misma unidad sale de la MISMA estación más tarde,
    // la unidad queda apartada ahí mientras tanto. El último hueco se extiende hasta el fin
    // de la jornada (pernocta si no vuelve a asignarse ningún servicio).
    const porUnidad = new Map();
    for (const t of trs) { if (!porUnidad.has(t.unit)) porUnidad.set(t.unit, []); porUnidad.get(t.unit).push(t); }
    for (const [unitId, servicios] of porUnidad) {
      servicios.sort((a, b) => a.pts[0].d - b.pts[0].d);
      const unit = S.units.get(unitId); if (!unit) continue;
      for (let i = 0; i < servicios.length - 1; i++) {
        const fin = servicios[i], ini = servicios[i + 1];
        const finSt = fin.pts[fin.pts.length - 1].id, finT = fin.pts[fin.pts.length - 1].a;
        const iniSt = ini.pts[0].id, iniT = ini.pts[0].d;
        if (finSt === iniSt && iniT > finT) unit.huecos.push({ station: finSt, from: finT, to: iniT });
      }
      const ult = servicios[servicios.length - 1];
      unit.huecos.push({ station: ult.pts[ult.pts.length - 1].id, from: ult.pts[ult.pts.length - 1].a, to: DIA_FIN });
    }
  }
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
    S.trains.push({ id: t.train, line: t.line, color: colorLinea(t.line), pts, shift: 0, side: latN >= lat0 ? "A" : "B", pax: 0, unit: null, esperando: false, esperaDesde: null, esperaHacia: null, esperaMotivo: null, cantonConcedido: null });
  }
  encadenarFlota();
  // Índice de vía única: qué trenes recorren cada recurso de vía única (para la prioridad
  // de cruce Regionals > cercanías, sin recorrer todos los trenes en cada comprobación).
  S.singleTrackUsers.clear();
  for (const t of S.trains) {
    for (let j = 0; j < t.pts.length - 1; j++) {
      const rk = resKey(t.pts[j].id, t.pts[j + 1].id);
      if (!S.singleTrack.has(canon(t.pts[j].id, t.pts[j + 1].id))) continue;
      if (!S.singleTrackUsers.has(rk)) S.singleTrackUsers.set(rk, []);
      S.singleTrackUsers.get(rk).push({ t, j });
    }
  }
  trainLayer.clearLayers(); trainMarkers.clear();
  parkedLayer.clearLayers(); parkedMarkers.clear();
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

// Prioridad en vía única: un Regional que se acerca por el mismo recurso en los próximos
// minutos retiene a un tren de cercanías aunque el tramo esté libre ahora mismo — el
// despachador real ya reserva el apartadero para el que se salta paradas.
const VENTANA_PRIORIDAD_SEC = 8 * 60;
function regionalEntrandoPronto(rk, exceptId) {
  const users = S.singleTrackUsers.get(rk); if (!users) return false;
  for (const { t, j } of users) {
    if (t.id === exceptId || !esRegional(t.line)) continue;
    const entrada = t.pts[j].d + t.shift;
    if (entrada >= S.simT && entrada <= S.simT + VENTANA_PRIORIDAD_SEC) return true;
  }
  return false;
}

// ¿El cantón A→B está en rojo para `id_tren` (línea `line`)? Cubre: modo manual/incidencia,
// ocupación física (compartida entre sentidos en vía única, ver `resKey`), cizallamiento
// (cantones que se cruzan físicamente) y prioridad Regionals>cercanías en vía única.
function cantonRojo(a, b, id_tren, line) {
  const k = key(a, b);
  const mode = S.cantonMode.get(k) || "AUTO";
  if (mode === "MR") return true;
  if (incidenteEdge(k, "block")) return true;         // avería mecánica: bloqueo total
  if (mode === "MV") return false;
  const rk = resKey(a, b);
  const o = S.occ.get(rk);
  if (o != null && o !== id_tren) return true;         // AUTO: rojo si otro tren lo ocupa
  for (const otroRk of S.shearMap.get(rk) || []) {
    const oo = S.occ.get(otroRk);
    if (oo != null && oo !== id_tren) return true;      // cizallamiento: el cruce está tomado
  }
  if (line && !esRegional(line) && S.singleTrack.has(canon(a, b)) && regionalEntrandoPronto(rk, id_tren)) return true;
  return false;
}

// Corredor de estación lleno (Montcada Bifurcació: R3 aislada / R4+R7 comparten vías): un
// tren no puede entrar si su corredor ya tiene tantos trenes parados como vías asignadas.
function corredorLleno(stId, line, exceptId) {
  const st = S.stationById.get(stId);
  if (!st || !st.corridors || !st.corridors.length) return false;
  const c = st.corridors.find((c) => c.lines.includes(line));
  if (!c) return false;
  let n = 0;
  for (const t of S.trains) {
    if (t.id === exceptId || !c.lines.includes(t.line)) continue;
    const te = S.simT - t.shift, seg = segmentoDe(t, te);
    if (seg && !seg.moving && t.pts[seg.i].id === stId) n++;
  }
  return n >= c.tracks;
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
// El bloqueo por semáforo se comprueba SIEMPRE contra la posición SIN BLOQUEAR de este
// fotograma (`teLibre`), no contra la posición ya clavada de fotogramas anteriores. A
// velocidades altas (×25) un solo fotograma puede avanzar varios segundos de juego — más que
// de sobra para saltarse por completo una ventana de comprobación estrecha, que es como un
// tren podía "atravesar" un semáforo en rojo sin que el motor llegase a mirarlo. Por eso aquí
// NO se usa el `moving`/`dwelling` de fotogramas anteriores para decidir si toca comprobar:
// se recalcula la clasificación de la posición libre y, si el semáforo de salida de la
// ÚLTIMA parada tocada está en rojo, el tren se clava exactamente ahí (fin de andén),
// cualquiera que sea la distancia que el salto de este fotograma pretendiera cubrir.
function paso(dtReal) {
  const dtGame = dtReal * FACTOR_BASE * S.speed;
  S.simT += dtGame;
  if (S.simT >= DIA_FIN && !S.finished) { S.simT = DIA_FIN; finDelDia(); }
  activarIncidencias();

  // Pass 1: posición SIN BLOQUEAR de cada tren este fotograma (a partir de su `shift`
  // acumulado hasta el fotograma ANTERIOR), ocupación de cantones y qué estación/lado está
  // ocupado por un tren parado (para el bloqueo "tren delante del semáforo, incluida la
  // estación siguiente").
  S.occ.clear();
  const dwellBySide = new Map(); // "stationId|side" -> id_tren
  const est = [];
  for (const t of S.trains) {
    const teLibre = S.simT - t.shift;
    const seg = segmentoDe(t, teLibre);
    if (!seg) { est.push(null); continue; }
    est.push({ teLibre, seg });
    if (seg.moving) {
      const rk = resKey(t.pts[seg.i].id, t.pts[seg.i + 1].id);
      if (!S.occ.has(rk)) S.occ.set(rk, t.id);
    } else {
      dwellBySide.set(t.pts[seg.i].id + "|" + t.side, t.id);
    }
  }
  // ¿La estación destino ya tiene otro tren de este lado parado (ocupando la vía por la que
  // entraría)? Si la estación tiene corredores definidos, se comprueba por corredor/línea en
  // `corredorLleno`; aquí cubrimos el caso general (pool único, la mayoría de estaciones).
  const destinoOcupado = (stId, side, exceptId) => {
    const o = dwellBySide.get(stId + "|" + side);
    return o != null && o !== exceptId;
  };

  // Pass 2: gating. IMPORTANTE — el semáforo de salida (interno/cantón/corredor/destino) solo
  // se valida en el instante en que el tren INTENTA entrar en un cantón nuevo; una vez
  // concedido, no se revalida cada fotograma (si no, un tren ya circulando por el cantón
  // podría "rebotar" hacia atrás porque la estación destino se ocupó DESPUÉS de que saliera,
  // lo cual no tiene sentido físico — el hueco se reservó al partir). Solo las incidencias a
  // mitad de trayecto (rojo manual, catenaria, avería) se comprueban en todo momento, como es
  // real: sí pueden detener a un tren que ya está circulando.
  let n = 0;
  for (let idx = 0; idx < S.trains.length; idx++) {
    const t = S.trains[idx], e = est[idx]; if (!e) continue;
    n++;
    const p = t.pts;
    // `i` = índice de la ÚLTIMA parada que el tren ya tocó (según su posición libre): si va
    // "moving", es el origen del cantón que cruza; si va "dwelling", es donde está parado.
    const i = e.seg.i;
    let bloqueado = false, motivo = null;
    if (!e.seg.moving && estacionParada(p[i].id)) { bloqueado = true; motivo = "averia"; }
    // `quiereSalir`: el reloj SIN BLOQUEAR ya alcanzó o pasó la hora de salida de i — en
    // dwell normal (aún no toca salir) no se evalúa nada, para no adelantar la salida.
    const quiereSalir = i < p.length - 1 && e.teLibre >= p[i].d;
    if (!bloqueado && quiereSalir) {
      const destino = p[i + 1].id, k = key(p[i].id, destino);
      if (t.cantonConcedido === k) {
        // Cantón ya concedido: solo incidencias/manual a mitad de trayecto pueden pararlo.
        if (S.cantonMode.get(k) === "MR" || incidenteEdge(k, "block")) { bloqueado = true; motivo = "semaforo"; }
        else if (edgeLento(k) && (Math.floor(S.simT) % 2 === 0)) { bloqueado = true; motivo = "catenaria"; }
      } else if (internoRojo(p[i].id, t.side)) { bloqueado = true; motivo = "interno"; }
      else if (cantonRojo(p[i].id, destino, t.id, t.line)) { bloqueado = true; motivo = "semaforo"; }
      else if (corredorLleno(destino, t.line, t.id)) { bloqueado = true; motivo = "corredor"; }
      else if (destinoOcupado(destino, t.side, t.id)) { bloqueado = true; motivo = "destino"; }
      else {
        t.cantonConcedido = k; // semáforo verde: concede el cantón, no se revalida hasta el siguiente
      }
    }
    if (bloqueado && S.speed > 0) {
      // Clava el tren EXACTAMENTE en el instante de salida de la parada i (frente al
      // semáforo, no a mitad de cantón), sea cual sea la distancia que este fotograma
      // pretendía avanzar.
      t.shift = S.simT - p[i].d;
      t.cantonConcedido = null;
      const pax = Math.round(NOMINAL_PAX * demanda(S.simT));
      S.kpi += (dtGame / 60) * pax;
    }
    t.esperando = bloqueado && motivo !== "averia";
    t.esperaDesde = t.esperando ? p[i].id : null;
    t.esperaHacia = t.esperando && i < p.length - 1 ? p[i + 1].id : null;
    t.esperaMotivo = t.esperando ? motivo : null;
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
    else m.setLatLng(pos);
    // Esperando en un semáforo: aro rojo grueso (bien distinguible de un simple retraso).
    if (t.esperando) m.setStyle({ color: "#c5221f", weight: 3, radius: 5.5 });
    else m.setStyle({ color: late ? "#ec6e15" : "#26272d", weight: 1.4, radius: 4.5 });
    const esperaTxt = t.esperando ? `<br><b>⏸ Esperando ${MOTIVO_LABEL[t.esperaMotivo] || t.esperaMotivo}</b> en ${nombre(t.esperaDesde)}` : "";
    m.setTooltipContent(`${t.line} · tren ${t.id}<br>Destino: ${nombre(t.pts[t.pts.length - 1].id)}<br>Retraso: ${t.shift > 60 ? "+" + Math.round(t.shift / 60) + " min" : "puntual"}${esperaTxt}`);
  }
  for (const [id, m] of trainMarkers) if (!vivos.has(id)) { trainLayer.removeLayer(m); trainMarkers.delete(id); }

  // Unidades apartadas (pernocta/reposicionamiento): al terminar un servicio, la unidad
  // física no desaparece — se aparta a una vía secundaria del terminal hasta su próximo
  // servicio (o hasta el fin de la jornada, si no tiene ninguno más asignado).
  const aparcados = new Set();
  for (const [unitId, u] of S.units) {
    const hueco = u.huecos.find((h) => S.simT >= h.from && S.simT < h.to);
    if (!hueco) continue;
    const st = S.stationById.get(hueco.station); if (!st) continue;
    aparcados.add(unitId);
    const ang = (hashStr(unitId) % 360) * Math.PI / 180, r = 0.0018;
    const pos = [st.lat + Math.sin(ang) * r, st.lon + Math.cos(ang) * r];
    let m = parkedMarkers.get(unitId);
    if (!m) {
      m = L.circleMarker(pos, { radius: 3.5, color: "#26272d", weight: 1, dashArray: "2,2", fillColor: colorLinea(u.line), fillOpacity: .85 });
      m.bindTooltip(""); m.addTo(parkedLayer); parkedMarkers.set(unitId, m);
    }
    m.setTooltipContent(`Unidad ${unitId} (${u.line}) apartada en ${st.name}<br>Próximo servicio: ${hueco.to >= DIA_FIN ? "fin de jornada" : hhmm(hueco.to)}`);
  }
  for (const [id, m] of parkedMarkers) if (!aparcados.has(id)) { parkedLayer.removeLayer(m); parkedMarkers.delete(id); }

  $("kActive").textContent = S.activos;
  $("clock").textContent = hhmm(S.simT);
}
const nombre = (id) => { const s = S.stationById.get(id); return s ? s.name : id; };
function hashStr(s) { let h = 0; for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) >>> 0; return h; }

// ---- Semáforos de tramo en el mapa (LOD: sólo al acercar y dentro de vista) ----------------
// Vía doble: un semáforo POR SENTIDO, desplazado perpendicularmente a cada lado de la vía
// (como en la realidad — no pueden compartir posición ni estado, cada sentido se gestiona por
// separado). Vía única: un único semáforo en el centro (el bastón/testigo es un recurso
// físico compartido, no tiene sentido un semáforo independiente por sentido).
function offsetPerp(a, b, lado) {
  const dLat = b.lat - a.lat, dLon = b.lon - a.lon;
  const len = Math.hypot(dLat, dLon) || 1;
  const OFFSET = 0.00035; // grados: separación visual entre las dos vías/semáforos
  const mid = [(a.lat + b.lat) / 2, (a.lon + b.lon) / 2];
  return [mid[0] + (dLon / len) * OFFSET * lado, mid[1] - (dLat / len) * OFFSET * lado];
}
// Trenes actualmente retenidos (t.esperando) en el cantón A→B — para el indicador visual y
// el tooltip del semáforo ("interfaz de trenes esperando en semáforo").
function trenesEsperandoEn(a, b) {
  const out = [];
  for (const t of S.trains) {
    if (t.esperando && t.esperaDesde === a && t.esperaHacia === b) out.push(t);
  }
  return out;
}
const MOTIVO_LABEL = { semaforo: "semáforo en rojo", interno: "semáforo interno", corredor: "corredor lleno", destino: "vía destino ocupada", catenaria: "catenaria (velocidad reducida)" };

function pintarSemaforos() {
  signalLayer.clearLayers();
  if (!$("showSignals").checked || map.getZoom() < 12 || !S.net) return;
  const vb = map.getBounds();
  for (const e of S.net.edges) {
    const a = S.stationById.get(e.from), b = S.stationById.get(e.to); if (!a || !b) continue;
    const unica = S.singleTrack.has(canon(e.from, e.to));
    const pos = unica ? [(a.lat + b.lat) / 2, (a.lon + b.lon) / 2] : offsetPerp(a, b, 1);
    if (!vb.contains(pos)) continue;
    const k = key(e.from, e.to), rojo = cantonRojo(e.from, e.to, null, e.lines[0]), manual = (S.cantonMode.get(k) || "AUTO") !== "AUTO";
    const esperando = trenesEsperandoEn(e.from, e.to);
    const m = L.circleMarker(pos, {
      radius: esperando.length ? (unica ? 8 : 7) : (unica ? 6 : 5),
      color: esperando.length ? "#ec6e15" : (manual ? "#ffd43b" : (unica ? "#26272d" : "#fff")),
      weight: esperando.length ? 3 : (manual ? 2 : (unica ? 1.5 : 1)),
      fillColor: rojo ? "#c5221f" : "#2f9e44", fillOpacity: 1,
    });
    const listaEspera = esperando.length
      ? "<br><b>Esperando aquí:</b><br>" + esperando.map((t) => `${t.line} tren ${t.id} (${MOTIVO_LABEL[t.esperaMotivo] || t.esperaMotivo}, +${Math.round(t.shift / 60)} min)`).join("<br>")
      : "";
    m.bindTooltip(`${e.lines.join(",")} · ${nombre(e.from)} → ${nombre(e.to)}${unica ? " · vía única (testigo compartido)" : " · sentido " + nombre(e.from) + "→" + nombre(e.to)}<br>${({ AUTO: "Automático", MV: "Manual verde", MR: "Manual rojo" })[S.cantonMode.get(k) || "AUTO"]}${listaEspera}`);
    m.on("click", () => { const cur = S.cantonMode.get(k) || "AUTO"; S.cantonMode.set(k, cur === "AUTO" ? "MV" : cur === "MV" ? "MR" : "AUTO"); pintarSemaforos(); });
    m.addTo(signalLayer);
  }
}
map.on("moveend zoomend", pintarSemaforos);

// ---- Panel de enclavamiento de estación ---------------------------------------------------
function abrirEstacion(s) { S.selSt = s; $("stationBox").hidden = false; buildStationPanel(); }

// Construye el panel UNA vez (o al cambiar un ajuste): crea el DOM y engancha los manejadores.
// No debe llamarse cada fotograma (destruiría los botones justo al pulsarlos).
function buildStationPanel() {
  const s = S.selSt; if (!s) return;
  const il = interlock(s.id);
  $("stName").textContent = s.name;
  $("stMeta").textContent = `${il.tracks} vía(s) · líneas ${s.lines.join(", ")}`;
  let html = `<div class="form" style="display:flex;gap:8px;margin:8px 0">
      <label style="flex:1">Ppal. A<select id="selMainA"></select></label>
      <label style="flex:1">Ppal. B<select id="selMainB"></select></label></div>`;
  for (let v = 0; v < il.tracks; v++) {
    const ga = il.green[v].A, gb = il.green[v].B;
    const tags = [il.mainA === v ? "◀A" : "", il.mainB === v ? "B▶" : ""].filter(Boolean).join(" ");
    html += `<div class="trackrow">
      <button class="sig ${ga ? "green" : "red"}" data-v="${v}" data-lado="A">A</button>
      <div class="viac ${(il.mainA === v || il.mainB === v) ? "main" : ""}" data-v="${v}">Vía ${v + 1} ${tags}<span class="occ-label"></span></div>
      <button class="sig ${gb ? "green" : "red"}" data-v="${v}" data-lado="B">B</button>
    </div>`;
  }
  const box = $("stTracks"); box.innerHTML = html;
  const opt = (sel, val) => { let o = ""; for (let v = 0; v < il.tracks; v++) o += `<option value="${v}"${v === val ? " selected" : ""}>Vía ${v + 1}</option>`; sel.innerHTML = o; };
  opt($("selMainA"), il.mainA); opt($("selMainB"), il.mainB);
  $("selMainA").onchange = (e) => { il.mainA = +e.target.value; buildStationPanel(); };
  $("selMainB").onchange = (e) => { il.mainB = +e.target.value; buildStationPanel(); };
  box.querySelectorAll(".sig").forEach((b) => b.onclick = () => { const v = +b.dataset.v, l = b.dataset.lado; il.green[v][l] = !il.green[v][l]; buildStationPanel(); });

  // Esquema(s) de vías real(es) (trenscat.com, ver reference/trenscat/) si los hay para esta
  // estación — referencia visual de cantones/agujas reales, clic para ver a tamaño completo.
  const diagBox = $("stDiagrams");
  diagBox.innerHTML = (s.diagrams || []).map((url) =>
    `<a href="${url}" target="_blank" rel="noopener"><img src="${url}" alt="Esquema de vías"><div class="cap">Esquema real (trenscat.com)</div></a>`).join("");

  actualizarEstacionDinamico();
}

// Refresca SOLO lo que cambia con el tiempo (ocupación, próximo tren) sin recrear los botones.
function actualizarEstacionDinamico() {
  const s = S.selSt; if (!s || $("stationBox").hidden) return;
  const il = interlock(s.id);
  for (let v = 0; v < il.tracks; v++) il.occ[v] = null;
  for (const t of S.trains) { const te = S.simT - t.shift, seg = segmentoDe(t, te); if (seg && !seg.moving && t.pts[seg.i].id === s.id) il.occ[t.side === "A" ? il.mainA : il.mainB] = t.id; }
  let prox = null, mejor = Infinity;
  for (const t of S.trains) { const te = S.simT - t.shift, seg = segmentoDe(t, te); if (!seg) continue; for (let j = Math.max(seg.i, 0); j < t.pts.length; j++) { if (t.pts[j].id === s.id) { const eta = t.pts[j].a - te; if (eta >= 0 && eta < mejor) { mejor = eta; prox = t; } break; } } }
  $("stNext").innerHTML = prox ? `<span class="stNextline">Próximo: <b>${prox.line}</b> tren ${prox.id} en ~${Math.round(mejor / 60)} min</span>` : `<span class="muted">Sin trenes próximos.</span>`;
  document.querySelectorAll("#stTracks .viac").forEach((cell) => {
    const v = +cell.dataset.v, occ = il.occ[v];
    cell.classList.toggle("occ", !!occ);
    const lbl = cell.querySelector(".occ-label"); if (lbl) lbl.textContent = occ ? " · " + occ : "";
  });
}

// ---- Actualización del GTFS (descarga oficial o subida manual) ----------------------------
let gtfsPolling = false;
async function iniciarActualizacionGtfs() {
  if (gtfsPolling) return;
  const r = await (await fetch("/api/gtfs/update")).json();
  if (!r.started) { $("gtfsStatus").textContent = r.reason || "No se pudo iniciar."; return; }
  pollGtfsStatus();
}
async function subirGtfs() {
  if (gtfsPolling) return;
  const f = $("gtfsFile").files[0];
  if (!f) { $("gtfsStatus").textContent = "Elige primero un fichero .zip."; return; }
  $("gtfsStatus").textContent = "Subiendo " + f.name + "…";
  const r = await (await fetch("/api/gtfs/upload", { method: "POST", body: f })).json();
  if (!r.started) { $("gtfsStatus").textContent = r.reason || "No se pudo iniciar."; return; }
  pollGtfsStatus();
}
async function pollGtfsStatus() {
  gtfsPolling = true;
  $("gtfsUpdate").disabled = true; $("gtfsUpload").disabled = true;
  const tick = async () => {
    const j = await (await fetch("/api/gtfs/status")).json();
    if (j.error) {
      $("gtfsStatus").textContent = "Error: " + j.error;
      gtfsPolling = false; $("gtfsUpdate").disabled = false; $("gtfsUpload").disabled = false;
      return;
    }
    $("gtfsStatus").textContent = j.step || "Actualizando…";
    if (j.done) {
      gtfsPolling = false; $("gtfsUpdate").disabled = false; $("gtfsUpload").disabled = false;
      const st = j.stats;
      if (st) $("gtfsStatus").textContent = `Actualizado: ${st.n_routes} líneas · ${st.n_trips} viajes · ${st.n_stops} estaciones. Recargando…`;
      await cargarRed(); await cargarHorarios(); pintarSemaforos();
      return;
    }
    setTimeout(tick, 1000);
  };
  tick();
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
  filas.sort((x, y) => (y.t.esperando - x.t.esperando) || (sort === "delay" ? y.retr - x.retr : x.t.line.localeCompare(y.t.line)));
  $("trenesList").innerHTML = filas.slice(0, 200).map((f) =>
    `<div class="t${f.t.esperando ? " esperando" : ""}"><span class="badge" style="background:${f.t.color}">${f.t.line}</span>
      <span>${f.t.esperando ? "⏸ " : ""}${f.ubic} → ${nombre(f.t.pts[f.t.pts.length - 1].id)}${f.t.esperando ? ` <span class="muted">(${MOTIVO_LABEL[f.t.esperaMotivo] || f.t.esperaMotivo})</span>` : ""}</span>
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
$("gtfsUpdate").addEventListener("click", iniciarActualizacionGtfs);
$("gtfsUpload").addEventListener("click", subirGtfs);
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
  if (acumUI > 0.5) { acumUI = 0; actualizarKpi(); refrescarIncidencias(); refrescarTrenes(); actualizarEstacionDinamico(); pintarSemaforos(); }
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
