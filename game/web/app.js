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

// Icono pixel de aparcamiento/depósito (ANDANA). Solo presentación (sustituye al glifo P de depósito).
const ICON_PARK = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" width="16" height="16" fill="currentColor" style="shape-rendering:crispEdges" role="img" aria-label="Depot / parking"><rect x="4" y="2" width="3" height="12"/><rect x="4" y="2" width="8" height="2"/><rect x="9" y="2" width="3" height="6"/><rect x="4" y="6" width="8" height="2"/></svg>';

// Icono de pausa (ANDANA) para trenes detenidos ante señal. Inline, hereda color del texto.
const ICON_PAUSE = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="11" height="11" fill="currentColor" style="vertical-align:-1px" aria-hidden="true"><rect x="6" y="5" width="4" height="14" rx="1"/><rect x="14" y="5" width="4" height="14" rx="1"/></svg>';

// Overlay de carga (tren 447 ANDANA). Solo presentación: muestra/oculta #rodLoad.
// `anim` elige el GIF según lo que pasa: "pantografo" = «Conectando…» (sube el pantógrafo y toma
// tensión, aún parado) · "circulando" = «Cargando…» (el tren ya rueda).
const ROD_GIF = { pantografo: "/game/tren_pantografo.gif", circulando: "/game/tren_circulando.gif" };
function rodLoad(show, txt, anim) {
  const el = document.getElementById("rodLoad"); if (!el) return;
  const t = document.getElementById("rodLoadTxt"); if (txt && t) t.textContent = txt;
  const g = document.getElementById("rodLoadGif");
  if (g && ROD_GIF[anim] && !g.src.endsWith(ROD_GIF[anim])) g.src = ROD_GIF[anim];
  if (show) { el.classList.remove("is-done"); el.hidden = false; }
  else { el.classList.add("is-done"); setTimeout(function () { el.hidden = true; }, 480); }
}

// ---- Señalización por BLOQUES FIJOS -------------------------------------------------------
// Cada cantón (tramo entre dos estaciones) se subdivide en N sub-bloques de vía por DISTANCIA.
// Cada bloque admite UN solo tren (como en la señalización real de bloque fijo): su señal se
// pone en ROJO en cuanto hay un tren dentro, y no vuelve a verde hasta que el tren libera el
// bloque cruzando la señal siguiente. Un bloque libre cuya señal SIGUIENTE está en rojo muestra
// ÁMBAR (aviso de "próxima en rojo", sin detener).
//
// DENSIDAD (petición del usuario, sesión 2026-09-12): la señalización de tramo NO debe ser
// excesiva. Se coloca ~1 señal cada `SIGNAL_SPACING_KM`, con un TOPE DURO de señales por tramo:
// 3 como máximo en tramos normales, hasta 5 en tramos muy largos (>7 km). Referencia del usuario:
// Fabra i Puig ↔ Torre Baró (≈3,4 km) = 2 señales → 3 bloques (round(3.4/1.6)=2). Se ELIMINA la
// densidad adaptativa por nº de líneas (`BLOCK_KM/√nLines`) de la versión anterior, que en el
// tronco (R2, muchas líneas) generaba muchísimas señales.
const SIGNAL_SPACING_KM = 1.6;   // separación objetivo entre señales de tramo (vía doble)
const MARGIN_KM = 0.5;           // no colocar una señal de tramo a menos de esto de una estación
// Distancia entre dos estaciones (km), Haversine.
function distKm(a, b) {
  const R = 6371, toR = Math.PI / 180;
  const dLat = (b.lat - a.lat) * toR, dLon = (b.lon - a.lon) * toR;
  const s = Math.sin(dLat / 2) ** 2 + Math.cos(a.lat * toR) * Math.cos(b.lat * toR) * Math.sin(dLon / 2) ** 2;
  return 2 * R * Math.asin(Math.min(1, Math.sqrt(s)));
}
// Nº de sub-bloques de un cantón a→b: SIEMPRE 1 en vía única (no hay señales de tramo, solo las
// de estación — ver la lógica de vía única). En vía doble, nº de señales de tramo = distancia /
// SIGNAL_SPACING_KM, ACOTADO a [0..maxSig] (maxSig = 3, o 5 si el tramo es muy largo). m sub-bloques
// ⇒ m-1 señales de tramo. Se reduce m si algún bloque quedaría más corto que MARGIN (señal pegada
// al andén). Independiente del nº de líneas: la densidad la fija la longitud, no el tráfico.
function calcSubBlocks(a, b, unica, lenM) {
  if (unica || !a || !b) return 1;
  const d = lenM ? lenM / 1000 : distKm(a, b);   // longitud de VÍA real (mismo dato que el motor)
  const maxSig = d > 7 ? 5 : 3;
  let signals = Math.max(0, Math.min(maxSig, Math.round(d / SIGNAL_SPACING_KM)));
  let m = signals + 1;
  while (m > 1 && d / m < MARGIN_KM) m--;
  return m;
}

// Logotipos oficiales de cada línea (game/web/lines/), usados en la leyenda y las listas en vez
// de un simple recuadro de color. Las variantes (R2N/R2S, R3a) caen en su línea madre.
const LINE_IMG = {
  R1: "R1_barcelona.svg.webp", R2: "R2_barcelona.svg.webp", R2N: "R2_Nord_Rodalies.png",
  R2S: "R2_Sud_Rodalies.png", R3: "R3.jpg", R3a: "R3.jpg", R4: "R4.jpg", R7: "R7.png",
  R8: "R8_barcelona.svg.webp", R11: "R11.jpg", R13: "R13_Rodalies_de_Catalunya.svg.png",
  R14: "R14.jpg", R15: "R15_Rodalies_de_Catalunya.svg.png", R16: "R16_Rodalies_de_Catalunya.svg.png",
  R17: "R17_Rodalies_de_Catalunya.svg.webp", RG1: "RG1_rodalies.png", RL3: "RL3.png",
  RL4: "RL4_Rodalies_de_Catalunya.svg.png", RT1: "RT1_tarragona.svg.webp", RT2: "RT2_tarragona.svg.webp",
};
// <img> del logo de una línea (o cadena vacía si no hay). `cls` = clase CSS de tamaño.
function lineImg(line, cls) {
  const f = LINE_IMG[line];
  return f ? `<img class="${cls}" src="/game/lines/${f}" alt="${line}" title="${line}" loading="lazy">` : "";
}

// ---- Estado -------------------------------------------------------------------------------
const S = {
  net: null, trains: [], stationById: new Map(), edgeLines: new Map(),
  edgeZones: new Map(),    // key(a,b) -> [{a,b,v,ltv}…] perfil de Vmax por zonas del cantón
  edgeGeom: new Map(),     // key(a,b) -> {pts:[[lat,lon]…], cum:[…]} trazado REAL de vía (OSM), ambos sentidos
  vmaxView: false,         // ¿mapa coloreado por Vmax en vez de por línea?
  dayIncidents: [],        // incidencias del día (roll del motor + lanzadas), para el registro/cajón
  simT: DIA_INICIO, speedKey: "PAUSA", speed: 0,
  cantonMode: new Map(),   // "A>B" -> 'AUTO'|'MV'|'MR' (manual por cantón entero, afecta a todos sus bloques)
  occ: new Map(),          // clave de bloque -> [id_tren...] (UN tren por bloque; vía única = clave canónica)
  subBlocks: new Map(),    // key(from,to) -> nº de sub-bloques del cantón en ese sentido (≥1)
  dwell: new Map(),        // id_estacion -> tiempo de parada reglamentario (s de juego), por densidad GTFS
  sigMarkers: [],          // señales de tramo dibujadas ahora (para repintar solo el aspecto por fotograma)
  interlocks: new Map(),   // id_estacion -> {tracks,mainA,mainB,green[[A,B]],occ[]}
  incidentes: [],          // {tipo,nombre,kind,scope,target,label,whenSec,durSec,estado}
  singleTrack: new Set(),  // claves canónicas "idA|idB" (idA<idB) de tramos de vía única
  shearMap: new Map(),     // clave de recurso -> [claves de recurso en conflicto] (cizallamiento)
  singleTrackUsers: new Map(), // clave de recurso -> [{t,j}] trenes que la recorren (para prioridad)
  units: new Map(),        // id_unidad_física -> {line, parked:bool, atStation, freeAt}
  parked: [],              // unidades físicas actualmente apartadas (para dibujar)
  kpi: 0, activos: 0, selSt: null, selSig: null, finished: false,
  arrivals: [],            // llegadas tarde a estación (del motor): {t, train, line, stop, delay}
  arrPtr: 0,               // puntero de avance en `arrivals` (contador incremental de min de retraso)
  terminals: new Set(),    // estaciones que solo son cabecera/fin de línea (nunca paso): apartadero
};

// CORREDORES (como en Godot): en ciertas estaciones las vías se reparten en grupos
// INDEPENDIENTES de líneas, cada uno con SU vía principal por lado (dos líneas que no se
// estorban → dos vías principales por dirección). Config del lado juego (el GTFS no lo trae);
// Montcada-Bifurcació ya viene con corredores del backend.
// Cada corredor: lista de líneas, u objeto {lines, tracks?, terminal?}: `tracks` fija cuántas vías
// tiene (si no, reparto equitativo); `terminal` = cabecera SIN vía principal propia: sus trenes dan
// la vuelta / aparcan en las vías NO principales (que comparten con la flota que pernocta).
const CORREDORES = {
  // Barcelona-Sants: R2 y regionales comparten las MISMAS vías principales (usuario 2026-09-24).
  "71801": [["R2", "R2N", "R2S", "R11", "R13", "R14", "R15", "R16", "R17"], ["R1", "R4"]],
  "79009": [["R1", "R11"], ["R2", "R2N"]],   // Barcelona El Clot
  "72210": [["R4"], ["R8"]],                 // Castellbisbal
  "72503": [["R7"], ["R8"]],                 // Cerdanyola Universitat
  // Sant Vicenç de Calders (14 vías): regionales (menos R13) por un par de principales, la R13 por
  // otro par, y R2S/R4 —su cabecera— dan la vuelta y aparcan en las no principales.
  "71600": [{ lines: ["R14", "R15", "R16", "R17"], tracks: 2 }, { lines: ["R13"], tracks: 2 }, { lines: ["R2S", "R4"], terminal: true }],
  // La Plana-Picamoixons: 4 vías, todas principales: 2 de la R13 (Valls) y 2 de la R14.
  "73100": [{ lines: ["R13"], tracks: 2 }, { lines: ["R14", "RT1"], tracks: 2 }],
};
// Grupos de corredor de una estación (config JS, o los del backend, o uno solo por defecto).
function corredoresDe(stId, st) {
  if (CORREDORES[stId]) return CORREDORES[stId].map((c) => (Array.isArray(c) ? { lines: c } : c));
  if (st && st.corridors && st.corridors.length) return st.corridors.map((c) => ({ lines: c.lines }));
  return [{ lines: st ? st.lines : [] }];
}
function tieneCorredores(stId) { const st = S.stationById.get(stId); return !!(CORREDORES[stId] || (st && st.corridors && st.corridors.length > 1)); }
function corredorDe(stId, line) { return corrIdx(interlock(stId), line); }
const $ = (id) => document.getElementById(id);

// ---- Mapa ---------------------------------------------------------------------------------
const map = L.map("map", { minZoom: 8, maxZoom: 15, zoomControl: true });
L.tileLayer("https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}",
  { maxZoom: 19, attribution: "Esri, Maxar, Earthstar Geographics · GTFS Rodalies" }).addTo(map);
map.setView([41.7, 1.9], 8);
// Panes de vía: la FRANJA gris (casing) va debajo del color, y ambas debajo de las estaciones
// (overlayPane, zIndex 400) para que el punto de la estación no quede tapado por la línea.
map.createPane("trackCasing"); map.getPane("trackCasing").style.zIndex = 380;
map.createPane("trackLine"); map.getPane("trackLine").style.zIndex = 390;
const lineLayer = L.layerGroup().addTo(map);
const stationLayer = L.layerGroup().addTo(map);
const signalLayer = L.layerGroup().addTo(map);
const trainLayer = L.layerGroup().addTo(map);
const parkedLayer = L.layerGroup().addTo(map);
const singleIncLayer = L.layerGroup().addTo(map);   // resalte de cantones en vía única por incidencia
const trainMarkers = new Map();
const parkedMarkers = new Map();
// Solo para el aislado visual de líneas de la nueva UI (chrome, no afecta a la simulación).
const linePolys = [];

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
  S.edgeZones.clear();
  for (const e of S.net.edges) {
    S.edgeLines.set(key(e.from, e.to), e.lines);
    // Perfil de Vmax por zonas del cantón (para la vista "mapa por velocidad máxima").
    if (e.zones) S.edgeZones.set(key(e.from, e.to), e.zones);
  }
  // Trazado REAL de vía (polilínea OSM del motor): se precalcula UNA vez la distancia acumulada de
  // cada cantón en los dos sentidos; por fotograma solo se hace una búsqueda binaria por tren.
  S.edgeGeom.clear();
  for (const e of S.net.edges) {
    if (!e.geom || e.geom.length < 2) continue;
    S.edgeGeom.set(key(e.from, e.to), geomIndex(e.geom));
    if (!S.edgeGeom.has(key(e.to, e.from))) S.edgeGeom.set(key(e.to, e.from), geomIndex(e.geom.slice().reverse()));
  }

  // Vía única real: bastón/testigo — ambos sentidos comparten la MISMA vía física, así que
  // se indexan por clave canónica no dirigida (ver `resKey`).
  S.singleTrack.clear();
  for (const [a, b] of S.net.single_track) S.singleTrack.add(canon(a, b));

  // Cizallamientos: ocupar un cantón retiene en rojo el cantón con el que se cruza
  // físicamente (p. ej. el ramal único de R7 invadiendo la vía de R4 en Cerdanyola).
  S.shearMap.clear();
  // shearMap: key(from,to) -> [[c,d]…] pares de estación de los cantones que se cruzan con este.
  const addShear = (a, b, c, d) => {
    for (const [x, y] of [[a, b], [b, a]]) { const k = key(x, y); if (!S.shearMap.has(k)) S.shearMap.set(k, []); S.shearMap.get(k).push([c, d]); }
  };
  for (const [[a1, b1], [a2, b2]] of S.net.shears || []) { addShear(a1, b1, a2, b2); addShear(a2, b2, a1, b1); }

  // Sub-bloques por cantón y sentido: cada uno es un bloque de UN tren. Vía única = 1 (testigo).
  S.subBlocks.clear();
  for (const e of S.net.edges) {
    const a = S.stationById.get(e.from), b = S.stationById.get(e.to);
    // + semáforos extra de topología (`extra_sig`, mismo dato que usa el motor Rust).
    const unica = S.singleTrack.has(canon(e.from, e.to));
    S.subBlocks.set(key(e.from, e.to), calcSubBlocks(a, b, unica, e.len_m) + (unica ? 0 : e.extra_sig || 0));
  }

  // Trazado por CANTONES (ver `dibujarLineas`): franja gris de fondo + color; vía doble = dos
  // líneas paralelas al acercar el zoom; vía única = una sola.
  dibujarLineas();
  const bounds = [];
  for (const s of S.stationById.values()) bounds.push([s.lat, s.lon]);
  // Estaciones.
  stationLayer.clearLayers();
  for (const s of S.stationById.values()) {
    if (s.junction) continue;   // bifurcación fuera de estación (aguja): no es una parada
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

// ---- Trazado de vías (franja gris de fondo + color; doble = 2 líneas al acercar) -----------
const LINE_SPLIT_ZOOM = 12;   // zoom a partir del cual la vía DOBLE se desdobla en 2 líneas y el
                              // tren circula por SU vía (coincide con la aparición de semáforos).
const TRACK_OFFSET = 0.0003;  // separación perpendicular (grados) entre las 2 vías de un tramo doble.
// Desplazamiento perpendicular a la DERECHA del sentido de marcha a→b (grados lat/lon). Los dos
// sentidos de un mismo tramo dan lados opuestos ⇒ dos vías paralelas. Lo COMPARTEN el trazado de
// vías, los semáforos de tramo y la posición del tren, así que todo cae exactamente sobre su vía.
function perpOffset(a, b) {
  const dLat = b.lat - a.lat, dLon = b.lon - a.lon, len = Math.hypot(dLat, dLon) || 1;
  return [(dLon / len) * TRACK_OFFSET, -(dLat / len) * TRACK_OFFSET];
}
// ---- Trazado REAL de la vía (curvas) -------------------------------------------------------
// El motor mide cada cantón sobre la polilínea OSM de la vía y el trace da la FRACCIÓN recorrida
// sobre esa longitud: aquí se convierte en punto + rumbo sobre la misma polilínea, así el tren va
// por la curva real (no por la recta entre estaciones). Distancias en plano local (cos lat): basta
// para fracciones proporcionales dentro de un cantón.
function geomIndex(pts) {
  const cum = [0], k = Math.cos(pts[0][0] * Math.PI / 180);
  for (let i = 1; i < pts.length; i++) cum.push(cum[i - 1] + Math.hypot(pts[i][0] - pts[i - 1][0], (pts[i][1] - pts[i - 1][1]) * k));
  return { pts, cum };
}
// Polilínea del cantón a→b: la real si existe; si no, la recta entre estaciones.
function viaPts(aId, bId) {
  const g = S.edgeGeom.get(key(aId, bId));
  if (g) return g.pts;
  const A = S.stationById.get(aId), B = S.stationById.get(bId);
  return A && B ? [[A.lat, A.lon], [B.lat, B.lon]] : [];
}
// Punto a la fracción f∈[0,1] de la vía a→b + rumbo local (grados desde el norte) + desplazamiento
// perpendicular a la derecha del sentido a→b (para la vía de ese sentido en tramo doble).
function puntoEnVia(aId, bId, f) {
  const g = S.edgeGeom.get(key(aId, bId));
  let p0, p1, t;
  if (g) {
    const cum = g.cum, L = cum[cum.length - 1], d = Math.max(0, Math.min(1, f)) * L;
    let lo = 0, hi = cum.length - 1;
    while (hi - lo > 1) { const m = (lo + hi) >> 1; if (cum[m] <= d) lo = m; else hi = m; }
    p0 = g.pts[lo]; p1 = g.pts[hi];
    t = cum[hi] > cum[lo] ? (d - cum[lo]) / (cum[hi] - cum[lo]) : 0;
  } else {
    const A = S.stationById.get(aId), B = S.stationById.get(bId);
    if (!A || !B) return null;
    p0 = [A.lat, A.lon]; p1 = [B.lat, B.lon]; t = Math.max(0, Math.min(1, f));
  }
  const pos = [p0[0] + (p1[0] - p0[0]) * t, p0[1] + (p1[1] - p0[1]) * t];
  const a = { lat: p0[0], lon: p0[1] }, b = { lat: p1[0], lon: p1[1] };
  return { pos, brg: bearing(a, b), off: perpOffset(a, b) };
}
// Polilínea desplazada `lado`·TRACK_OFFSET a la derecha (vía de un sentido en tramo doble): cada
// vértice se mueve según la normal media de sus dos segmentos, para que las curvas no se quiebren.
function desplazarVia(pts, lado) {
  if (pts.length < 2) return pts;
  const nrm = (p, q) => { const dLat = q[0] - p[0], dLon = q[1] - p[1], l = Math.hypot(dLat, dLon) || 1; return [dLon / l, -dLat / l]; };
  return pts.map((p, i) => {
    const n0 = nrm(pts[Math.max(0, i - 1)], pts[Math.max(1, i)]), n1 = nrm(pts[Math.min(i, pts.length - 2)], pts[Math.min(i + 1, pts.length - 1)]);
    let nx = n0[0] + n1[0], ny = n0[1] + n1[1]; const l = Math.hypot(nx, ny) || 1; nx /= l; ny /= l;
    return [p[0] + nx * TRACK_OFFSET * lado, p[1] + ny * TRACK_OFFSET * lado];
  });
}
// Trozo de la vía a→b entre las fracciones fa y fb (para colorear zonas de Vmax sobre la curva).
function tramoVia(aId, bId, fa, fb) {
  const g = S.edgeGeom.get(key(aId, bId));
  if (!g) { const p = puntoEnVia(aId, bId, fa), q = puntoEnVia(aId, bId, fb); return p && q ? [p.pos, q.pos] : []; }
  const L = g.cum[g.cum.length - 1], da = fa * L, db = fb * L, out = [puntoEnVia(aId, bId, fa).pos];
  for (let i = 0; i < g.pts.length; i++) if (g.cum[i] > da && g.cum[i] < db) out.push(g.pts[i]);
  out.push(puntoEnVia(aId, bId, fb).pos);
  return out;
}
// Dibuja una vía: una FRANJA gris oscura de fondo (para que el color resalte sobre el satélite al
// hacer zoom) y encima el trazo de color de la línea. Vía única = trazo discontinuo (bastón).
function trazarVia(pts, color, unica, entry) {
  const casing = L.polyline(pts, { pane: "trackCasing", color: "#050507", weight: unica ? 8 : 9, opacity: 0.72, lineCap: "round", lineJoin: "round" }).addTo(lineLayer);
  const linea = L.polyline(pts, { pane: "trackLine", color, weight: unica ? 2.5 : 3, opacity: 0.95, dashArray: unica ? "6,5" : null, lineCap: "round", lineJoin: "round" }).addTo(lineLayer);
  entry.casing.push(casing); entry.color.push(linea);
}
// Dibuja un cantón en la vista Vmax: casing gris + un segmento de color por cada ZONA de Vmax del
// perfil ADIF (los sub-tramos varían de color a lo largo del cantón; una LTV ocupa solo sus metros
// reales, resaltada con color brillante y trazo más grueso). Sin datos → gris.
function trazarViaVmax(aId, bId, zones, entry) {
  const casing = L.polyline(viaPts(aId, bId), { pane: "trackCasing", color: "#050507", weight: 9, opacity: 0.72, lineCap: "round", lineJoin: "round" }).addTo(lineLayer);
  entry.casing.push(casing);
  const zs = (zones && zones.length) ? zones : [{ a: 0, b: 1, v: 0, ltv: false }];
  for (const z of zs) {
    // Las fracciones a/b de la zona son de la longitud de VÍA → se pintan sobre la curva real.
    const line = L.polyline(tramoVia(aId, bId, z.a, z.b), { pane: "trackLine", color: vmaxColor(z), weight: z.ltv ? 5.5 : 3.2, opacity: 0.98, lineCap: "butt" }).addTo(lineLayer);
    entry.color.push(line);
  }
}
// JERARQUÍA DE COLOR en cantones compartidos: qué línea "pinta" un tramo por el que pasan varias
// (orden fijado por el usuario, 2026-09-22). Las no listadas (R3, R3a…) van detrás, en su orden.
const LINE_COLOR_PRIORITY = ["R4", "R2", "R2N", "R2S", "R1", "R8", "R7", "R11", "RG1", "R15", "R13", "R14", "R16", "R17", "RL3", "RL4", "RT1", "RT2"];
const LINE_PRIO_IDX = new Map(LINE_COLOR_PRIORITY.map((l, i) => [l, i]));
function lineaDominante(lines) {
  let best = lines[0], bi = Infinity;
  for (const l of lines) { const i = LINE_PRIO_IDX.has(l) ? LINE_PRIO_IDX.get(l) : 1000; if (i < bi) { bi = i; best = l; } }
  return best;
}
// (Re)dibuja todas las vías. A zoom bajo: una sola línea centrada por cantón (rápido y legible).
// A zoom ≥ LINE_SPLIT_ZOOM: la vía doble se desdobla en DOS líneas paralelas (una por sentido),
// mientras la vía única sigue siendo UNA sola. Se llama al cargar la red y al cruzar el umbral.
let lineasDesdobladas = null;
function dibujarLineas() {
  if (!S.net) return;
  const split = map.getZoom() >= LINE_SPLIT_ZOOM;
  lineasDesdobladas = split;
  lineLayer.clearLayers();
  linePolys.length = 0;
  const vistos = new Set();
  for (const e of S.net.edges) {
    const c = canon(e.from, e.to);
    if (vistos.has(c)) continue; vistos.add(c);
    const a = S.stationById.get(e.from), b = S.stationById.get(e.to);
    if (!a || !b) continue;
    const unica = S.singleTrack.has(c);
    const lines = S.edgeLines.get(key(e.from, e.to)) || S.edgeLines.get(key(e.to, e.from)) || e.lines;
    const entry = { lines, unica, canon: c, casing: [], color: [] };   // para el aislado visual (chrome UI)
    if (S.vmaxView) {
      // Vista Vmax: una sola línea centrada, coloreada por SUB-TRAMOS según el perfil ADIF del cantón.
      trazarViaVmax(e.from, e.to, S.edgeZones.get(key(e.from, e.to)), entry);
    } else {
      const color = colorLinea(lineaDominante(lines));
      const pts = viaPts(e.from, e.to);    // trazado real de vía (curvas); sin él, la recta
      if (unica || !split) {
        trazarVia(pts, color, unica, entry);
      } else {
        // Vía del sentido a→b (a la derecha) y la del sentido opuesto, paralelas a la curva.
        trazarVia(desplazarVia(pts, 1), color, false, entry);
        trazarVia(desplazarVia(pts, -1), color, false, entry);
      }
    }
    linePolys.push(entry);
  }
}
map.on("zoomend", () => { if ((map.getZoom() >= LINE_SPLIT_ZOOM) !== lineasDesdobladas) dibujarLineas(); });

// ---- Máquina de estados del material rodante: flota física persistente ---------------------
// Encadena servicios GTFS consecutivos del mismo origen/línea en unidades físicas con ID
// estable (aproximación greedy: mismo criterio que Godot — reutiliza la unidad libre más
// antigua en esa estación si el giro mínimo lo permite; si no, crea una unidad nueva).
const GIRO_MINIMO_SEG = 5 * 60;
function encadenarFlota() {
  S.units.clear();
  S.rutaLinea = new Map();
  const porLinea = new Map();
  for (const t of S.trains) { if (!porLinea.has(t.line)) porLinea.set(t.line, []); porLinea.get(t.line).push(t); }
  let nextId = 1;
  for (const [line, trs] of porLinea) {
    trs.sort((a, b) => a.pts[0].d - b.pts[0].d);
    // Ruta canónica de la línea (para la dispersión `_buscar_deposito_cercano`): el servicio con
    // más paradas, con los tiempos de tramo — recorriéndola a los dos lados se busca el depósito
    // más cercano con apartadero libre.
    let best = null; for (const t of trs) if (!best || t.pts.length > best.pts.length) best = t;
    if (best) {
      const ids = best.pts.map((p) => p.id), tiempos = [];
      for (let i = 0; i < best.pts.length - 1; i++) tiempos.push(Math.max(60, best.pts[i + 1].a - best.pts[i].d));
      S.rutaLinea.set(line, { ids, tiempos });
    }
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
      // Hueco INICIAL: la unidad ya EXISTE al empezar la jornada (05:00), durmiendo en el
      // origen de su primer servicio hasta que le toca salir. Así toda la flota está desde el
      // principio (ningún tren se crea a mitad del día), como en Godot.
      const pri = servicios[0];
      if (pri.pts[0].d > DIA_INICIO) unit.huecos.push({ station: pri.pts[0].id, from: DIA_INICIO, to: pri.pts[0].d, svc: pri });
      for (let i = 0; i < servicios.length - 1; i++) {
        const fin = servicios[i], ini = servicios[i + 1];
        const finSt = fin.pts[fin.pts.length - 1].id, finT = fin.pts[fin.pts.length - 1].a;
        const iniSt = ini.pts[0].id, iniT = ini.pts[0].d;
        if (finSt === iniSt && iniT > finT) unit.huecos.push({ station: finSt, from: finT, to: iniT, svc: ini });
      }
      const ult = servicios[servicios.length - 1];
      unit.huecos.push({ station: ult.pts[ult.pts.length - 1].id, from: ult.pts[ult.pts.length - 1].a, to: DIA_FIN, svc: null });
    }
  }
  dispersarFlota();
}

// Reparte la flota durmiente por APARTADEROS (vías no principales) respetando la capacidad de
// cada estación (= nº de vías no principales, como plazas de parking). Si una estación se
// satura, el sobrante (los menos urgentes) se dispersa a la estación con apartadero libre MÁS
// CERCANA de su línea (`_buscar_deposito_cercano` de Godot: recorre la ruta a los dos lados y
// compara el tiempo de viaje real). Último recurso: se queda donde estaba (marcado overflow).
function dispersarFlota() {
  const slots = new Map();   // stId -> Array<freeAt> (una por apartadero)
  const slotsDe = (stId) => { let a = slots.get(stId); if (!a) { a = new Array(sidingsDe(stId)).fill(-Infinity); slots.set(stId, a); } return a; };
  const slotLibre = (stId, t0) => { const a = slotsDe(stId); for (let i = 0; i < a.length; i++) if (a[i] <= t0) return i; return -1; };
  const todos = [];
  for (const [unitId, u] of S.units) for (const h of u.huecos) todos.push({ unitId, line: u.line, h });
  // Urgentes primero (salida más próxima) → cogen el apartadero local; los menos urgentes se dispersan.
  todos.sort((a, b) => (a.h.from - b.h.from) || (a.h.to - b.h.to));
  for (const { line, h } of todos) {
    let st = h.station, i = slotLibre(st, h.from);
    if (i < 0) {
      const dep = buscarDepositoCercano(line, st, (s) => slotLibre(s, h.from) >= 0);
      if (dep) { h.depositoDe = h.station; h.station = dep; st = dep; i = slotLibre(dep, h.from); }
    }
    if (i >= 0) { slotsDe(st)[i] = h.to; } else { h.overflow = true; }
  }
}

// Horario de TODA la red (sin filtro de línea: el aislado de líneas es solo visual). La fuente se
// fija al arrancar (`S.source`, GTFS salvo `?source=optimized`); los horarios se eligen en la
// pantalla previa (GTFS guardado / nuevo / importado), que recarga red + horarios si cambian.
async function cargarHorarios() {
  const q = new URLSearchParams({ source: S.source || "gtfs" });
  // Horario cadenciado del creador (/horarios), ya cargado en el servidor con `cargarCadencia`.
  if (S.cadId) q.set("cad", S.cadId);
  const data = await (await fetch("/api/game/schedule?" + q.toString())).json();
  if (data.error) throw new Error(data.error);
  S.trains = [];
  for (const t of data.trains) {
    const pts = [];
    for (const st of t.stops) { const s = S.stationById.get(st.s); if (s) pts.push({ id: st.s, lat: s.lat, lon: s.lon, a: st.a, d: st.d, pass: !!st.pass }); }
    if (pts.length < 2) continue;
    const lat0 = pts[0].lat, latN = pts[pts.length - 1].lat;
    S.trains.push({ id: t.train, line: t.line, color: colorLinea(t.line), pts, shift: 0, side: latN >= lat0 ? "A" : "B", pax: 0, unit: null, esperando: false, esperaDesde: null, esperaHacia: null, esperaMotivo: null, seq: null, cb: 0 });
  }
  imponerParadaMinima();
  encadenarFlota();
  // Terminales: estaciones que SOLO aparecen como extremo de recorrido (nunca de paso). Los
  // trenes que finalizan allí se apartan a vías no principales, así que no bloquean a los que
  // siguen (Vilafranca, Martorell, Maçanet…). Se deduce del propio horario.
  const extremo = new Set(), paso = new Set();
  for (const t of S.trains) {
    for (let j = 0; j < t.pts.length; j++) {
      if (j === 0 || j === t.pts.length - 1) extremo.add(t.pts[j].id); else paso.add(t.pts[j].id);
    }
  }
  S.terminals = new Set([...extremo].filter((id) => !paso.has(id)));
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
  construirSecuenciaBloques();
  trainLayer.clearLayers(); trainMarkers.clear();
  parkedLayer.clearLayers(); parkedMarkers.clear();
  S.kpi = 0; S.arrPtr = 0; actualizarKpi();
  $("netInfo").textContent = `${S.net.n_stations} estaciones · ${S.net.n_lines} líneas · ${S.trains.length} trenes ` +
    (data.source === "cadence" ? `(horario cadenciado «${S.cadName || ""}»).` : `(${data.source === "optimized" ? "optimizado" : "GTFS"}, ${data.service_id}).`);
}

// ---- Enclavamiento (lazy) -----------------------------------------------------------------
function interlock(id) {
  let il = S.interlocks.get(id);
  if (!il) {
    const st = S.stationById.get(id); const n = st ? Math.max(1, st.tracks) : 1;
    const grupos = corredoresDe(id, st), nc = grupos.length;
    // Reparto de vías en bloques consecutivos entre corredores; cada corredor tiene su vía
    // principal por lado (las dos primeras del grupo) → varias vías principales por dirección.
    // Los corredores con `tracks` fijo se sirven primero; el resto se reparte a partes iguales entre
    // los demás. Un corredor `terminal` no tiene principal: todas sus vías son no principales.
    const fijas = grupos.reduce((s, g) => s + (g.tracks || 0), 0);
    const libres = grupos.filter((g) => !g.tracks).length;
    const resto = Math.max(0, n - fijas);
    const corrDeVia = new Array(n).fill(0), corr = [];
    let cursor = 0, k0 = 0;
    for (let c = 0; c < nc; c++) {
      const g = grupos[c];
      const cuantas = g.tracks || Math.max(1, Math.floor(resto / Math.max(1, libres)) + (k0++ < resto % Math.max(1, libres) ? 1 : 0));
      const vias = [];
      for (let k = 0; k < cuantas && cursor < n; k++) { vias.push(cursor); corrDeVia[cursor] = c; cursor++; }
      const term = !!g.terminal;
      corr.push({ lines: g.lines || [], vias, terminal: term,
        mainA: term ? null : (vias[0] || 0), mainB: term ? null : (vias[1] != null ? vias[1] : (vias[0] || 0)) });
    }
    while (cursor < n) { corrDeVia[cursor] = nc - 1; corr[nc - 1].vias.push(cursor); cursor++; }
    const corrDeLinea = {};
    corr.forEach((g, c) => (g.lines || []).forEach((l) => { corrDeLinea[l] = c; }));
    il = { tracks: n, corr, corrDeVia, corrDeLinea, green: [], occ: [] };
    for (let i = 0; i < n; i++) { il.green.push({ A: true, B: true }); il.occ.push(null); }
    S.interlocks.set(id, il);
  }
  return il;
}
function corrIdx(il, line) { return il.corrDeLinea[line] != null ? il.corrDeLinea[line] : 0; }
// Vía principal hacia `side` para `line` (la de su corredor: cada corredor tiene la suya). null en
// un corredor de cabecera (sin principal: el tren para en una vía no principal).
function viaPrincipal(il, side, line) { const g = il.corr[corrIdx(il, line)] || il.corr[0]; return side === "A" ? g.mainA : g.mainB; }
// Semáforo interno de salida: rojo si el manual de la vía principal (del corredor de la línea)
// hacia ese lado está en rojo (verde por defecto → automático via bloque/destino en el gating).
function internoRojo(stId, side, line) { const il = interlock(stId); const via = viaPrincipal(il, side, line); return via != null && !il.green[via][side]; }
// Vías NO principales (apartaderos/plazas de depósito) de una estación: las que no son la
// principal de ningún corredor y lado. Su número = capacidad de trenes durmiendo (parking).
function sidingTracksDe(stId) {
  const il = interlock(stId); const mains = new Set();
  for (const g of il.corr) { if (g.mainA != null) mains.add(g.mainA); if (g.mainB != null) mains.add(g.mainB); }
  const out = []; for (let v = 0; v < il.tracks; v++) if (!mains.has(v)) out.push(v);
  return out;
}
function sidingsDe(stId) { return sidingTracksDe(stId).length; }
// Depósito más cercano de `line` con apartadero libre (`tieneHueco`), recorriendo la ruta
// canónica a los dos lados desde `stSat` y comparando tiempo de viaje real (como Godot).
function buscarDepositoCercano(line, stSat, tieneHueco) {
  const ruta = S.rutaLinea.get(line); if (!ruta) return null;
  const idx = ruta.ids.indexOf(stSat); if (idx < 0) return null;
  let candIzq = null, tIzq = Infinity, t = 0;
  for (let i = idx; i > 0;) { t += ruta.tiempos[i - 1]; i--; if (tieneHueco(ruta.ids[i])) { candIzq = ruta.ids[i]; tIzq = t; break; } }
  let candDer = null, tDer = Infinity; t = 0;
  for (let j = idx; j < ruta.ids.length - 1;) { t += ruta.tiempos[j]; j++; if (tieneHueco(ruta.ids[j])) { candDer = ruta.ids[j]; tDer = t; break; } }
  if (!candIzq) return candDer;
  if (!candDer) return candIzq;
  return tIzq <= tDer ? candIzq : candDer;
}

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

// Nº de sub-bloques del cantón a→b (≥1) y clave del BLOQUE físico (sub-bloque `sb`): en vía
// única ambos sentidos comparten el MISMO bloque-testigo (clave canónica, sin sentido ni `sb`).
function subM(a, b) { return S.subBlocks.get(key(a, b)) || 1; }
function sbRes(a, b, sb) { return esViaUnica(a, b) ? canon(a, b) : key(a, b) + "#" + sb; }

// Tiempo de parada reglamentario por estación, derivado de la DENSIDAD de servicio del GTFS (nº
// de paradas reales al día ∝ demanda de pasajeros): 20 s en las de menor afluencia, hasta 60 s
// en los grandes intercambiadores. Rebaquea los horarios imponiendo esa parada mínima en cada
// estación real intermedia (los puntos de paso sintéticos de los exprés NO paran); el retraso se
// propaga a las paradas siguientes, como en la explotación real. Los tiempos del tren PASAN a
// ser su horario (el KPI mide el retraso contra este, no contra el feed). Sin esto los trenes
// apenas se detenían (el feed lista muchas paradas con llegada≈salida).
const DWELL_MIN = 20, DWELL_MAX = 60;
function imponerParadaMinima() {
  const freq = new Map(); let maxFreq = 1;
  for (const t of S.trains) for (const p of t.pts) if (!p.pass) { const c = (freq.get(p.id) || 0) + 1; freq.set(p.id, c); if (c > maxFreq) maxFreq = c; }
  S.dwell.clear();
  for (const [id, c] of freq) S.dwell.set(id, Math.round(DWELL_MIN + (DWELL_MAX - DWELL_MIN) * Math.sqrt(c / maxFreq)));
  for (const t of S.trains) {
    let delta = 0;
    for (let i = 0; i < t.pts.length; i++) {
      const p = t.pts[i]; p.a += delta; p.d += delta;
      if (i > 0 && i < t.pts.length - 1 && !p.pass) {
        const need = S.dwell.get(p.id) || DWELL_MIN, have = p.d - p.a;
        if (have < need) { const extra = need - have; p.d += extra; delta += extra; }
      }
    }
  }
}

// Secuencia ordenada de BLOQUES de vía que recorre cada tren. CLAVE: el ANDÉN de una estación
// es la cola del ÚLTIMO sub-bloque del cantón que llega a ella (los tramos acaban en la SALIDA
// de la estación, no en la entrada; las dos señales internas por vía son de SALIDA). Por eso un
// tren parado en un andén sigue ocupando el bloque anterior (su señal sigue en rojo) hasta que
// arranca cruzando la señal de salida. `first` = sub-bloque 0 (salir de una estación → gating de
// salida). Un centinela de origen (índice 0) rige la salida de la primera estación. `tEnter` =
// tiempo efectivo al que el tren alcanza la señal de ese bloque.
function construirSecuenciaBloques() {
  for (const t of S.trains) {
    const p = t.pts, seq = [{ origin: true, st: p[0].id, tEnter: p[0].a }];
    for (let i = 0; i < p.length - 1; i++) {
      const m = subM(p[i].id, p[i + 1].id), run = Math.max(1, p[i + 1].a - p[i].d);
      for (let sb = 0; sb < m; sb++)
        seq.push({ from: p[i].id, to: p[i + 1].id, sb, m, first: sb === 0, last: sb === m - 1, dest: p[i + 1].id, arrIdx: i + 1, tEnter: p[i].d + run * sb / m });
    }
    t.seq = seq; t.cb = 0;   // cb = índice del bloque en el que el tren está comprometido ahora
  }
}

// Vías de andén por SENTIDO de una estación (para la capacidad del último sub-bloque, que
// incluye el andén). Una estación de 2 vías (1 por sentido) → 1: un tren parado llena el bloque
// y su señal anterior queda en rojo. Un gran intercambiador → varias: caben varios trenes
// andanados a la vez sin colapsar el tronco (varias líneas comparten el cantón de entrada).
function platCap(b) { const st = S.stationById.get(b); return Math.max(1, Math.floor((st ? st.tracks : 2) / 2)); }

// ¿Hay una incidencia del MOTOR con señal ROJA vigente en el cantón a→b ahora mismo? (red_one =
// solo ese sentido; red_both = ambos). La "vía única" no fija una señal roja. Alimenta el color de
// las señales del mapa para que las incidencias (rodadas o lanzadas) se VEAN en rojo.
function incidenteRojoEng(a, b) {
  const now = S.simT;
  for (const i of (S.dayIncidents || [])) {
    if (now < i.t0 || now >= i.t1 || i.effect === "single") continue;
    if (i.from === a && i.to === b) return true;
    if (i.effect === "red_both" && i.from === b && i.to === a) return true;
  }
  return false;
}
// ¿El cantón a↔b es VÍA ÚNICA por una incidencia vigente ahora? (ambos sentidos comparten la vía).
function incidenteSingleEng(a, b) {
  const now = S.simT;
  for (const i of (S.dayIncidents || [])) {
    if (i.effect !== "single" || now < i.t0 || now >= i.t1) continue;
    if ((i.from === a && i.to === b) || (i.from === b && i.to === a)) return true;
  }
  return false;
}
// Vía única EFECTIVA ahora: estática (topología) o por incidencia. Ambos sentidos comparten el
// testigo → un tren en la vía pone en rojo las señales del sentido CONTRARIO.
function esViaUnica(a, b) {
  return S.singleTrack.has(canon(a, b)) || incidenteSingleEng(a, b);
}

// ¿El bloque físico (sub-bloque `sb` del cantón a→b) está OCUPADO/cerrado? Capacidad 1 en vía
// abierta; el ÚLTIMO sub-bloque (incluye el andén de `b`) admite `platCap(b)` trenes. Rojo si el
// bloque está lleno, o manual rojo, o avería de bloqueo, o el cruce (cizallamiento) está tomado.
function bloqueOcupado(a, b, sb, exceptId) {
  const k = key(a, b), mode = S.cantonMode.get(k) || "AUTO";
  if (mode === "MR" || incidenteEdge(k, "block")) return true;
  if (mode === "MV") return false;
  // Incidencia del MOTOR (señal roja) vigente en este cantón: la señal es una barrera sin tren
  // ocupando la vía, así que hay que reflejarla aquí para que el semáforo se vea ROJO de verdad.
  if (incidenteRojoEng(a, b)) return true;
  // Vía única: capacidad SIEMPRE 1 (un tren en la vía, en cualquier sentido — es el bastón/testigo;
  // no aplica la capacidad de andén, porque la estación es un apartadero que se libera al parar).
  // Capacidad 1 por bloque (igual que el motor): la señal se pone roja con UN tren en el bloque.
  // Los trenes andanados no ocupan el bloque de aproximación (ver pasoRepro), así que la estación
  // no serializa las paradas aunque el último bloque sea cap 1.
  const occ = S.occ.get(sbRes(a, b, sb));
  if (occ && occ.filter((x) => x !== exceptId).length >= 1) return true;
  for (const [c, d] of S.shearMap.get(k) || []) if (edgeOcupadoRes(c, d, exceptId)) return true;
  return false;
}
// ¿Algún bloque del cantón c→d (o su testigo de vía única) está ocupado? (para el cizallamiento).
function edgeOcupadoRes(c, d, exceptId) {
  if (esViaUnica(c, d)) { const o = S.occ.get(canon(c, d)); return !!(o && o.some((x) => x !== exceptId)); }
  const m = subM(c, d);
  for (let s = 0; s < m; s++) { const o = S.occ.get(key(c, d) + "#" + s); if (o && o.some((x) => x !== exceptId)) return true; }
  return false;
}

// Aspecto de la señal del bloque `sb` del cantón a→b (misma regla en tramo y en estación):
//   ROJO  = su propio bloque ocupado/cerrado.  ÁMBAR = su bloque libre pero el SIGUIENTE en
//   rojo (solo aviso, no detiene).  VERDE = su bloque libre y el siguiente libre.
function aspectoBloque(a, b, sb, m, line) {
  if (bloqueOcupado(a, b, sb, null)) return "red";
  if ((S.cantonMode.get(key(a, b)) || "AUTO") === "MV") return "green";
  return siguienteRojo(a, b, sb, m, line) ? "amber" : "green";
}
// ¿El bloque SIGUIENTE (mismo sentido) está en rojo? Es el sub-bloque sb+1 del mismo cantón o,
// si es el último (el que incluye el andén destino), la señal de SALIDA de esa estación.
function siguienteRojo(a, b, sb, m, line) {
  if (sb < m - 1) return bloqueOcupado(a, b, sb + 1, null);
  return salidaOcupada(b, a);
}
// ¿La SALIDA de la estación `stId` está ocupada? = el primer sub-bloque de alguna vía de salida
// (excluida la de vuelta por donde se vino) tiene un tren. Es la señal que ve un tren que llega.
function salidaOcupada(stId, cameFrom) {
  for (const e of S.net.edges) {
    if (e.from !== stId || e.to === cameFrom) continue;
    if (bloqueOcupado(e.from, e.to, 0, null)) return true;
  }
  return false;
}
// Estado del bloque de SALIDA de una estación por lado (A=norte, B=sur), para las señales
// internas del panel: rojo si el primer sub-bloque de alguna vía de salida de ese lado está
// ocupado; ámbar si están libres pero la señal siguiente aguas abajo está en rojo.
function salidaBloqueada(stId, lado) {
  let rojo = false, ambar = false;
  for (const e of S.net.edges) {
    if (e.from !== stId || ladoEdge(e.from, e.to) !== lado) continue;
    if (bloqueOcupado(e.from, e.to, 0, null)) rojo = true;
    else if (siguienteRojo(e.from, e.to, 0, subM(e.from, e.to), e.lines[0])) ambar = true;
  }
  return { rojo, ambar };
}
// Sentido geométrico de una arista (A = hacia el norte, B = hacia el sur), para el semáforo interno.
function ladoEdge(a, b) { const A = S.stationById.get(a), B = S.stationById.get(b); return (A && B && B.lat >= A.lat) ? "A" : "B"; }

function estacionParada(stId) { const now = S.simT; return S.incidentes.some((i) => i.estado === "activa" && i.kind === "stop" && i.scope === "station" && i.target === stId && now < i.whenSec + i.durSec); }
function edgeLento(k) { const now = S.simT; return S.incidentes.some((i) => i.estado === "activa" && i.kind === "slow" && i.scope === "edge" && i.target === k && now < i.whenSec + i.durSec); }

// ---- Posición de un tren según su tiempo efectivo -----------------------------------------
function segmentoDe(t, te) {
  const p = t.pts;
  if (te < p[0].d || te > p[p.length - 1].a) return null;
  for (let i = 0; i < p.length - 1; i++) {
    if (te >= p[i].a && te <= p[i].d) return { i, moving: false };   // parado en la estación i
    // En marcha SOLO estrictamente entre la salida de i y la llegada a i+1. El límite superior
    // debe ser EXCLUSIVO: si te == p[i+1].a el tren YA HA LLEGADO a i+1 (parado), no sigue en el
    // tramo. Con `<=` un tren clavado en una estación de dwell 0 (a==d, p. ej. punto de paso de un
    // exprés) se contaba como "aún circulando" y NO liberaba el bloque anterior en vía única →
    // deadlock (dos trenes de frente que no llegaban a hacer el cruce). Ver segmentoDe/Pass 1.
    if (te > p[i].d && te < p[i + 1].a) return { i, moving: true };
  }
  return { i: p.length - 1, moving: false };
}
function posDe(t, seg, te) {
  const p = t.pts;
  if (!seg.moving) return [p[seg.i].lat, p[seg.i].lon];
  const A = p[seg.i], B = p[seg.i + 1], f = B.a > A.d ? (te - A.d) / (B.a - A.d) : 0;
  return [A.lat + (B.lat - A.lat) * f, A.lon + (B.lon - A.lon) * f];
}
// Rumbo del tren (grados clockwise desde el norte = arriba en pantalla) según el cantón que
// recorre; parado, usa la dirección del cantón siguiente (o el anterior si es terminal). Solo
// se usa para ORIENTAR el triángulo en el mapa — no interviene en la física.
function bearingDe(t, seg) {
  const p = t.pts; let a, b;
  if (seg.i < p.length - 1) { a = p[seg.i]; b = p[seg.i + 1]; }
  else if (seg.i > 0) { a = p[seg.i - 1]; b = p[seg.i]; }
  else return 0;
  return bearing(a, b);
}
function bearing(a, b) {
  const latR = a.lat * Math.PI / 180;
  const dy = b.lat - a.lat, dx = (b.lon - a.lon) * Math.cos(latR);
  return Math.atan2(dx, dy) * 180 / Math.PI;
}

// ---- REPRODUCTOR del motor unificado (Rust) -----------------------------------------------
// El mapa visual deja de calcular física propia: reproduce el TRACE del motor Rust (posición y
// velocidad de cada tren muestreadas en el tiempo). Así "lo que ves = lo que se simula en masa":
// los trenes aceleran/frenan de verdad (Vmax de vía + LTV + física del material). Si el trace no
// carga (binario antiguo), se cae al motor JS de bloques (`paso`).
async function cargarTrace(seed, selQ) {
  S.traceSeed = seed || 0;
  // Selección de plantillas del MODO VISUAL (`&sel=…`, independiente de /numeros). Se guarda para
  // que "lanzar incidencia ahora" re-simule exactamente el mismo día (mismas incidencias rodadas).
  S.traceSel = selQ || "";
  const tr = await (await fetch("/api/game/trace?seed=" + S.traceSeed + S.traceSel)).json();
  if (tr && tr.error) throw new Error(tr.error);
  if (!tr || !tr.trains) throw new Error("trace vacío");
  tr.byId = new Map();
  for (const t of tr.trains) tr.byId.set(t.id, t);
  S.trace = tr; S.repro = true; S.simT = tr.start;
  construirTimelineRetraso(tr);
  // Rótulo de la jornada: el motor DERIVA el inicio de la 1ª salida por horario (no un 05:00 fijo),
  // así que reflejamos el rango real del trace en el subtítulo del reloj.
  const sub = document.querySelector(".clock-sub");
  if (sub) sub.textContent = `Jornada · ${hhmm(tr.start)} → ${hhmm(tr.end)}`;
  // Llegadas tarde a estación (registro de CTC + contador de minutos de retraso). Reinicia el
  // contador: el bucle de reproducción lo va sumando conforme el reloj pasa por cada llegada.
  S.arrivals = tr.arrivals || []; S.arrPtr = 0; S.kpi = 0;
  // Incidencias del día que rodó el motor (para el registro y el cajón que se pone rojo).
  S.dayIncidents = (tr.incidents || []).map((i) => ({ ...i, launched: false }));
  pintarIncidenciasDia();
}
// Serie del RETRASO VIVO de la red por minuto de jornada, precalculada UNA vez del trace (no por
// fotograma): para cada minuto, la media de D(t) = Σ retraso de los trenes en el trace. Cada muestra
// (cada `dt` s) aporta d/60 min × dt/60 → media del minuto. Coste O(muestras) ≈ 1M sumas al cargar.
// Es la misma magnitud que el chip "min en la red ahora", así el gráfico y el KPI cuadran.
function construirTimelineRetraso(tr) {
  const n = Math.max(1, Math.ceil((tr.end - tr.start) / 60) + 1);
  const tl = new Float32Array(n), w = tr.dt / 60;
  for (const rt of tr.trains) {
    if (!rt.d) continue;
    for (let i = 0; i < rt.d.length; i++) {
      const b = Math.floor((rt.t0 + i * tr.dt - tr.start) / 60);
      if (b >= 0 && b < n) tl[b] += (rt.d[i] / 60) * w;
    }
  }
  let max = 0; for (const v of tl) if (v > max) max = v;
  S.delayTL = { start: tr.start, end: tr.end, vals: tl, max };
  S._dcKey = "";   // fuerza a redibujar el gráfico
}
// Gráfico compacto del retraso vivo de la red (panel izquierdo): área desde el inicio de la jornada
// hasta AHORA (no enseña el futuro de la grabación), eje de horas y cursor del instante actual.
// Solo redibuja cuando cambia el minuto de juego (barato).
function pintarGraficoRetraso() {
  const box = $("delayChart"), tl = S.delayTL; if (!box || !tl) return;
  const idx = Math.max(0, Math.min(tl.vals.length - 1, Math.floor((S.simT - tl.start) / 60)));
  const W = box.clientWidth || 290, H = box.clientHeight || 104;
  const k = idx + "|" + W;
  if (k === S._dcKey) return;
  S._dcKey = k;
  const padL = 26, padB = 13, padT = 4, iw = W - padL - 2, ih = H - padB - padT;
  const n = tl.vals.length;
  let pico = 0;
  for (let i = 0; i <= idx; i++) if (tl.vals[i] > pico) pico = tl.vals[i];   // pico HASTA AHORA (sin spoilers)
  const ymax = Math.max(10, Math.ceil(pico / 10) * 10);   // escala de lo ya visto, no del día entero
  const X = (i) => padL + (i / (n - 1)) * iw, Y = (v) => padT + ih - (v / ymax) * ih;
  let d = "";
  const step = Math.max(1, Math.floor(n / iw));   // ~1 punto por píxel
  for (let i = 0; i <= idx; i += step) d += (d ? "L" : "M") + X(i).toFixed(1) + "," + Y(tl.vals[i]).toFixed(1);
  if (idx % step) d += "L" + X(idx).toFixed(1) + "," + Y(tl.vals[idx]).toFixed(1);
  const area = d ? d + `L${X(idx).toFixed(1)},${Y(0)}L${X(0)},${Y(0)}Z` : "";
  let grid = "";
  for (const f of [0, 0.5, 1]) {
    const y = Y(ymax * f).toFixed(1);
    grid += `<line class="dc-grid" x1="${padL}" x2="${W - 2}" y1="${y}" y2="${y}"/><text class="dc-lbl" x="${padL - 4}" y="${+y + 3}" text-anchor="end">${Math.round(ymax * f)}</text>`;
  }
  for (let h = Math.ceil(tl.start / 3600 / 3) * 3; h * 3600 <= tl.end; h += 3) {
    const x = X((h * 3600 - tl.start) / 60).toFixed(1);
    grid += `<text class="dc-lbl" x="${x}" y="${H - 2}" text-anchor="middle">${String(h % 24).padStart(2, "0")}h</text>`;
  }
  const xn = X(idx).toFixed(1);
  box.innerHTML = `<svg viewBox="0 0 ${W} ${H}" role="img" aria-label="Retraso vivo de la red a lo largo de la jornada">
    <defs><linearGradient id="dcGrad" x1="0" x2="0" y1="0" y2="1"><stop offset="0" stop-color="#ff8a3d" stop-opacity=".55"/><stop offset="1" stop-color="#ff8a3d" stop-opacity=".03"/></linearGradient></defs>
    ${grid}<path class="dc-area" d="${area}"/><path class="dc-line" d="${d}"/>
    <line class="dc-now" x1="${xn}" x2="${xn}" y1="${padT}" y2="${padT + ih}"/></svg>`;
  const now = $("tlNow"); if (now) now.textContent = Math.round(tl.vals[idx]) + " min ahora · pico " + Math.round(pico);
}
// Retraso VIVO por línea (min sumados de sus trenes en el trace ahora) en la lista de líneas del
// panel izquierdo. O(trenes) cada ~0,5 s; solo toca texto/anchura de barras ya creadas.
function actualizarRetrasoLineas() {
  const rows = S.legendRows; if (!rows) return;
  const por = new Map();
  for (const t of S.trains) if (t.repro) por.set(t.line, (por.get(t.line) || 0) + (t.reproDelay || 0) / 60);
  let max = 1; for (const v of por.values()) if (v > max) max = v;
  for (const [line, r] of rows) {
    const v = por.get(line) || 0, m = Math.round(v);
    if (r.last === m && r.lastMax === Math.round(max)) continue;
    r.last = m; r.lastMax = Math.round(max);
    r.val.textContent = m + " min";
    r.val.classList.toggle("warn", m >= 10 && m < 25);
    r.val.classList.toggle("crit", m >= 25);
    r.bar.style.width = (100 * v / max).toFixed(1) + "%";
  }
}
// Pinta el registro de incidencias del día (panel derecho) y la lista del cajón inferior.
function pintarIncidenciasDia() {
  const list = S.dayIncidents || [];
  const EFFL = { red_one: "señal roja (1 sentido)", red_both: "señal roja (2 sentidos)", single: "vía única" };
  const fila = (i) => {
    const donde = (i.from_name || i.from) + " → " + (i.to_name || i.to);
    const cuando = hhmm(i.t0) + "–" + hhmm(i.t1);
    const activa = S.simT >= i.t0 && S.simT < i.t1;
    return `<li class="inc-row${activa ? " activa" : ""}${i.launched ? " manual" : ""}">
      <div class="inc-motivo">${i.motivo}${i.launched ? ' <span class="tag">lanzada</span>' : ""}</div>
      <div class="inc-meta">${EFFL[i.effect] || i.effect} · ${donde}</div>
      <div class="inc-when">${cuando}${activa ? " · <b>EN CURSO</b>" : ""}</div></li>`;
  };
  const inc = $("incList");
  if (inc) inc.innerHTML = list.length ? list.map(fila).join("") : '<li class="empty">Sin incidencias hoy.</li>';
  const dl = $("incidDiaList");
  if (dl) dl.innerHTML = list.length ? list.map(fila).join("") : '<div class="empty">Sin incidencias hoy.</div>';
  const cnt = $("incidDiaCount"); if (cnt) cnt.textContent = list.length;
}
// Marca el cajón de incidencias en ROJO mientras haya alguna EN CURSO (por fotograma, barato).
function actualizarIncidenciasDia() {
  const list = S.dayIncidents || [];
  const activa = list.some((i) => S.simT >= i.t0 && S.simT < i.t1);
  const btn = $("btnIncidDia"); if (btn) btn.classList.toggle("alerta", activa);
  dibujarViaUnicaIncidencias();
}
// Resalta en el mapa los cantones que están en VÍA ÚNICA por una incidencia vigente (línea gruesa
// punteada ámbar). Solo redibuja cuando cambia el conjunto activo (barato por fotograma).
let _singleSig = "";
function dibujarViaUnicaIncidencias() {
  const now = S.simT;
  const act = (S.dayIncidents || []).filter((i) => i.effect === "single" && now >= i.t0 && now < i.t1);
  const sig = act.map((i) => i.from + ">" + i.to).sort().join("|");
  if (sig === _singleSig) return;
  _singleSig = sig;
  singleIncLayer.clearLayers();
  for (const i of act) {
    const A = S.stationById.get(i.from), B = S.stationById.get(i.to);
    if (!A || !B) continue;
    L.polyline([[A.lat, A.lon], [B.lat, B.lon]], { color: "#ffb400", weight: 6, opacity: 0.9, dashArray: "3,8", lineCap: "round" }).addTo(singleIncLayer);
  }
}
// Nombre de una estación para la lista/buscador de tramos: nombre COMPLETO (con "Barcelona"
// entero, para poder buscarlo escribiéndolo), solo normaliza los guiones con espacios.
function nombreCorto(id) {
  return nombre(id).replace(/\s+-\s+/g, " ").trim();
}
// Un sub-paso del reproductor: sitúa cada tren interpolando su trace y reconstruye la ocupación de
// bloques (para colorear las señales del mapa exactamente como el motor). Vía única + parado =
// apartadero liberado (misma regla que el motor). No recalcula física: la lee.
function pasoRepro(dtGame) {
  S.simT += dtGame;
  if (S.simT >= DIA_FIN && !S.finished) { S.simT = DIA_FIN; finDelDia(); }
  const tr = S.trace; if (!tr) return;
  S.occ.clear();
  const platPark = new Map();   // recurso del andén → trenes parados (estaciones con varias vías por acceso)
  let activos = 0, liveDelay = 0;
  for (const t of S.trains) {
    t.reproActive = false; t.repro = null; t.reproHeld = false; t.reproDelay = 0;
    const rt = tr.byId.get(t.id); if (!rt) continue;
    const idx = (S.simT - rt.t0) / tr.dt;
    if (idx < 0 || idx > rt.a.length - 1) continue;
    const i0 = Math.floor(idx), i1 = Math.min(i0 + 1, rt.a.length - 1), fr = idx - i0;
    const a = tr.ids[rt.a[i0]], b = tr.ids[rt.b[i0]];
    // Interpola la fracción sólo si ambos extremos van por el MISMO cantón; si cruzó estación
    // entre muestras, se queda con la muestra i0 (salto imperceptible a dt=5 s).
    let frac = (rt.a[i0] === rt.a[i1] && rt.b[i0] === rt.b[i1])
      ? (rt.f[i0] + (rt.f[i1] - rt.f[i0]) * fr) / 1000
      : rt.f[i0] / 1000;
    const v = rt.v[i0] + (rt.v[i1] - rt.v[i0]) * fr;
    // Retraso REAL del tren (s) desde el trace del motor (no derivado del horario ni del proxy de
    // retenidos). Alimenta el KPI de pax·min, el panel de estación y el tooltip por tren.
    const dly = rt.d ? (rt.d[i0] + (rt.d[i1] - rt.d[i0]) * fr) : 0;
    t.reproDelay = dly;
    liveDelay += dly / 60;
    activos++;
    // El andén (vía doble) es una EXTENSIÓN del tramo de aproximación: un tren parado ahí SIGUE
    // ocupando ese tramo → la señal de entrada queda en ROJO mientras está en la estación. Solo en
    // VÍA ÚNICA el andén es apartadero y libera el testigo (para el cruce en sentido contrario).
    const stopped = frac >= 0.995;
    if (!(stopped && esViaUnica(a, b))) {
      const m = subM(a, b); let sb = Math.min(Math.floor(frac * m), m - 1); if (sb < 0) sb = 0;
      const rk = sbRes(a, b, sb);
      // Estación con varias vías principales por acceso (Sants: 2, como `anden_lleno` del motor): los
      // trenes PARADOS solo cierran la señal de entrada cuando ocupan todas las vías de ese acceso.
      const vias = stopped ? ((S.stationById.get(b) || {}).approach_tracks || 1) : 1;
      const parados = vias > 1 ? (platPark.get(rk) || 0) + 1 : 0;
      if (vias > 1) platPark.set(rk, parados);
      if (vias === 1 || parados >= vias) { let arr = S.occ.get(rk); if (!arr) { arr = []; S.occ.set(rk, arr); } arr.push(t.id); }
    }
    t.repro = { a, b, frac, v };
    t.reproActive = !stopped;
    t.reproHeld = v < 1 && frac > 0.02 && frac < 0.98;   // clavado en plena vía = retenido por señal
    t.esperando = t.reproHeld;
    if (t.reproHeld) { t.esperaDesde = a; t.esperaHacia = b; t.esperaMotivo = "semaforo"; }
    else { t.esperaDesde = t.esperaHacia = t.esperaMotivo = null; }
  }
  S.activos = activos;
  S.delayNow = liveDelay; // retraso VIVO de la red (min sumados de los trenes en marcha) → alerta
  // Contador de minutos de retraso: suma la tardanza (min) de cada llegada tarde a estación cuyo
  // instante ya ha pasado. Avanza un puntero sobre `arrivals` (orden temporal) → O(1) amortizado por
  // fotograma. Es la suma de los "+N min" que van saliendo en el registro de CTC. Sin pasajeros.
  const arr = S.arrivals;
  while (S.arrPtr < arr.length && arr[S.arrPtr].t <= S.simT) { S.kpi += arr[S.arrPtr].delay / 60; S.arrPtr++; }
}


// ---- Render de trenes ---------------------------------------------------------------------
function render() {
  const vivos = new Set();
  for (const t of S.trains) {
    // Posición/orientación/estado del tren, desde el REPRODUCTOR (trace del motor Rust unificado).
    // El motor visual es ÚNICO: el frontend solo LEE y dibuja el trace; no calcula física.
    let pos, bearingDeg, late, hold, tip;
    const r = t.repro;
    if (!r || !t.reproActive) continue;   // parado en estación (dentro): no se dibuja
    const pv = puntoEnVia(r.a, r.b, r.frac);   // sobre la curva real de la vía
    if (!pv) continue;
    pos = pv.pos;
    if (lineasDesdobladas && !S.singleTrack.has(canon(r.a, r.b))) { pos[0] += pv.off[0]; pos[1] += pv.off[1]; }
    bearingDeg = pv.brg;
    hold = t.reproHeld;
    const dlyMin = Math.round((t.reproDelay || 0) / 60);
    late = dlyMin >= 2 && !hold;   // en marcha pero ≥2 min por detrás de su horario
    const kmh = Math.round(r.v);
    tip = `${t.line} · tren ${t.id}<br>Destino: ${nombre(t.pts[t.pts.length - 1].id)}<br>` +
      (hold ? `<b>${ICON_PAUSE} Detenido ante señal</b> en ${nombre(r.a)}` : `Velocidad: <b>${kmh} km/h</b>`) +
      (dlyMin > 0 ? `<br>Retraso: <b>+${dlyMin}′</b>` : ``);
    vivos.add(t.id);
    let m = trainMarkers.get(t.id);
    if (!m) {
      m = L.marker(pos, { icon: L.divIcon({ className: "trn-ic", html: '<span class="trn-tri"></span>', iconSize: [18, 18], iconAnchor: [9, 9] }), keyboard: false });
      m._line = t.line; m.bindTooltip(""); m.addTo(trainLayer); trainMarkers.set(t.id, m);
    } else m.setLatLng(pos);
    const elm = m.getElement();
    if (elm && elm.firstChild) {
      const tri = elm.firstChild;
      tri.style.setProperty("--c", t.color);
      tri.style.transform = "rotate(" + bearingDeg.toFixed(1) + "deg)";
      elm.classList.toggle("late", late && !hold);
      elm.classList.toggle("hold", !!hold);
    }
    m.setTooltipContent(tip);
  }
  for (const [id, m] of trainMarkers) if (!vivos.has(id)) { trainLayer.removeLayer(m); trainMarkers.delete(id); }

  $("kActive").textContent = S.activos;
  $("clock").textContent = hhmm(S.simT);
}
const nombre = (id) => { const s = S.stationById.get(id); return s ? s.name : id; };
function hashStr(s) { let h = 0; for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) >>> 0; return h; }

// ---- Flota DURMIENDO en apartaderos (vías no principales) ----------------------------------
// La flota es FINITA: toda unidad existe de 05:00 a 00:00 y, cuando no da servicio, DUERME en
// una vía no principal (apartadero) de una estación. NO se dibuja en el mapa: los durmientes se
// ven en la INTERFAZ de la estación, aparcados en sus vías no principales (ver el HUD).
// Unidades durmiendo ahora mismo en `stId` (su hueco activo las sitúa aquí, ya dispersadas).
function durmientesEn(stId) {
  const out = [];
  for (const [unitId, u] of S.units) {
    const h = u.huecos.find((h) => S.simT >= h.from && S.simT < h.to);
    if (h && h.station === stId) out.push({ unitId, line: u.line, h });
  }
  return out;
}

// ---- Semáforos de tramo en el mapa (LOD: sólo al acercar y dentro de vista) ----------------
// Vía doble: un semáforo POR SENTIDO, desplazado perpendicularmente a cada lado de la vía
// (como en la realidad — no pueden compartir posición ni estado, cada sentido se gestiona por
// separado). Vía única: un único semáforo en el centro (el bastón/testigo es un recurso
// físico compartido, no tiene sentido un semáforo independiente por sentido).
// Geometría de las señales de tramo: se dibujan en CONJUNTOS (los dos sentidos juntos, uno a
// cada lado de la vía), usando el MISMO desplazamiento perpendicular (`TRACK_OFFSET`) que el
// trazado de las dos vías, de modo que cada semáforo cae exactamente sobre su vía.
// Punto a fracción `f` del cantón a→b, con desplazamiento perpendicular `lado` (0 = sin él).
// Punto de la vía a→b (ids) a la fracción f, desplazado `lado` vías (0 = eje) → [lat, lon]. Sigue
// la curva real, así el semáforo cae sobre SU vía también en los tramos revirados.
function posEnArista(aId, bId, f, lado) {
  const p = puntoEnVia(aId, bId, f); if (!p) return null;
  return lado ? [p.pos[0] + p.off[0] * lado, p.pos[1] + p.off[1] * lado] : p.pos;
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
const MOTIVO_LABEL = { semaforo: "bloque ocupado", interno: "semáforo interno", corredor: "corredor lleno", destino: "vía destino ocupada", catenaria: "catenaria (velocidad reducida)" };

// Crea el MARKER de una señal de bloque (triángulo) que guarda el sub-bloque `sb` (de `m`) del
// sentido from→to, y lo registra en `S.sigMarkers`. El COLOR (aspecto) NO se fija aquí: lo
// repinta `actualizarSemaforosAspecto` cada fotograma (barato). El tooltip se calcula al vuelo
// (función) para que salga siempre fresco sin coste por fotograma.
function crearSemaforo(from, to, sb, m, pos, brg) {
  const k = key(from, to), lines = S.edgeLines.get(k) || S.edgeLines.get(key(to, from)) || [];
  const mk = L.marker(pos, { icon: L.divIcon({ className: "sig-ic green", html: `<span class="sig-tri" style="transform:rotate(${brg.toFixed(1)}deg)"></span>`, iconSize: [16, 16], iconAnchor: [8, 8] }), keyboard: false });
  mk.bindTooltip(() => {
    const asp = aspectoBloque(from, to, sb, m, lines[0]);
    const esperando = trenesEsperandoEn(from, to);
    const listaEspera = esperando.length ? "<br><b>Detenido:</b> " + esperando.map((t) => `${t.line} ${t.id}`).join(", ") : "";
    const aspTxt = asp === "red" ? "ROJO — bloque ocupado" : asp === "amber" ? "ÁMBAR — próxima señal en rojo" : "VERDE — vía libre";
    return `${lines.join(",")} · ${nombre(from)} → ${nombre(to)} · bloque ${sb + 1}/${m}<br>${aspTxt} · ${MODO_LABEL[S.cantonMode.get(k) || "AUTO"]}${listaEspera}<br><span style="opacity:.7">Clic: panel del semáforo</span>`;
  });
  mk.on("click", () => abrirSemaforo({ from, to, k, sb, m, lines, unica: S.singleTrack.has(canon(from, to)) }));
  mk.addTo(signalLayer);
  S.sigMarkers.push({ mk, from, to, sb, m, k, line: lines[0], cls: "" });
}

// Reconstruye los MARKERS de señal visibles. Solo se llama al mover/zoom o al (des)activar las
// señales — NO por fotograma.
//   · VÍA DOBLE: señales de TRAMO = fronteras INTERNAS de cada cantón (sb=1..m-1), una por sentido,
//     cada una sobre su vía (mismo desplazamiento perpendicular que el trazado). Calibrado por
//     distancia (Fabra i Puig ↔ Torre Baró = 2 señales), con tope de densidad (ver calcSubBlocks).
//   · VÍA ÚNICA: SIN señales de tramo. Se dibujan las dos señales de SALIDA de estación (una en
//     cada extremo del tramo), centradas sobre la vía única. Cada una gobierna la entrada al bastón
//     en su sentido y se pone en rojo si hay un tren en el tramo EN CUALQUIER SENTIDO — así los
//     trenes se esperan en la estación para hacer el cruce sin chocar.
function construirSemaforos() {
  signalLayer.clearLayers();
  S.sigMarkers = [];
  if (!$("showSignals").checked || map.getZoom() < 12 || !S.net) return;
  const vb = map.getBounds();
  const vistos = new Set();
  for (const e of S.net.edges) {
    const c = canon(e.from, e.to);
    if (vistos.has(c)) continue; vistos.add(c);
    const a = S.stationById.get(e.from), b = S.stationById.get(e.to); if (!a || !b) continue;
    if (S.singleTrack.has(c)) {
      // Señales de salida de estación (bloque-testigo único: sb=0, m=1), una cerca de cada extremo.
      for (const [from, to, gf] of [[e.from, e.to, 0.14], [e.to, e.from, 0.86]]) {
        const pos = posEnArista(e.from, e.to, gf, 0);
        if (!pos || !vb.contains(pos)) continue;
        const pv = puntoEnVia(from, to, from === e.from ? gf : 1 - gf);
        crearSemaforo(from, to, 0, 1, pos, pv ? pv.brg : bearing(S.stationById.get(from), S.stationById.get(to)));
      }
      continue;
    }
    const dirs = [];
    if (S.edgeLines.has(key(e.from, e.to))) dirs.push([e.from, e.to, 1]);
    if (S.edgeLines.has(key(e.to, e.from))) dirs.push([e.to, e.from, -1]);
    for (const [from, to, lado] of dirs) {
      const m = subM(from, to);
      for (let sb = 1; sb < m; sb++) {
        const gf = lado > 0 ? sb / m : 1 - sb / m;            // fracción medida desde a=e.from
        const pos = posEnArista(e.from, e.to, gf, lado);
        if (!pos || !vb.contains(pos)) continue;
        const pv = puntoEnVia(from, to, sb / m);              // rumbo LOCAL en el sentido de marcha
        crearSemaforo(from, to, sb, m, pos, pv ? pv.brg : 0);
      }
    }
  }
  actualizarSemaforosAspecto();
}
// Repinta SOLO el color de las señales visibles — barato (O(trenes)+O(señales)) → cada fotograma,
// por eso a ×25 los cambios rojo/ámbar/verde son inmediatos y no un espectáculo de luces con retardo.
function actualizarSemaforosAspecto() {
  if (!S.sigMarkers.length) return;
  const waitSet = new Set();
  for (const t of S.trains) if (t.esperando && t.esperaDesde && t.esperaHacia) waitSet.add(t.esperaDesde + ">" + t.esperaHacia);
  for (const s of S.sigMarkers) {
    const asp = aspectoBloque(s.from, s.to, s.sb, s.m, s.line);
    const manual = (S.cantonMode.get(s.k) || "AUTO") !== "AUTO";
    const wait = waitSet.has(s.from + ">" + s.to);
    const cls = asp + (manual ? " manual" : "") + (wait ? " wait" : "");
    if (cls === s.cls) continue;                 // sin cambios → no toca el DOM
    s.cls = cls;
    const el = s.mk.getElement(); if (!el) continue;
    el.classList.remove("green", "amber", "red", "manual", "wait");
    el.classList.add(asp);
    if (manual) el.classList.add("manual");
    if (wait) el.classList.add("wait");
  }
}
map.on("moveend zoomend", construirSemaforos);

// ---- Panel del semáforo de tramo (dock derecho, exclusivo con estación/incidencias) --------
function abrirSemaforo(sig) { S.selSig = sig; cerrarDocks("signal"); $("signalBox").hidden = false; buildSignalPanel(); }
const MODO_LABEL = { AUTO: "Automático", MV: "Manual verde", MR: "Manual rojo" };
function buildSignalPanel() {
  const g = S.selSig; if (!g) return;
  $("sigTitle").textContent = nombre(g.from) + " → " + nombre(g.to);
  $("sigMeta").innerHTML = `<span class="st-logos">${g.lines.map((l) => lineImg(l, "line-logo sm")).join("")}</span>` +
    `<span>${g.unica ? "vía única · " : ""}bloque ${(g.sb || 0) + 1} de ${g.m || 1}</span>`;
  const cur = S.cantonMode.get(g.k) || "AUTO";
  $("sigModes").innerHTML = ["AUTO", "MV", "MR"].map((mo) =>
    `<button class="sigmode ${mo === cur ? "active" : ""} ${mo.toLowerCase()}" data-mode="${mo}">${MODO_LABEL[mo]}</button>`).join("");
  $("sigModes").querySelectorAll("button").forEach((btn) => btn.onclick = () => {
    S.cantonMode.set(g.k, btn.dataset.mode); buildSignalPanel(); actualizarSemaforosAspecto();
  });
  actualizarSemaforoDinamico();
}
// Refresca solo lo que cambia con el tiempo: aspecto actual y el tren detenido (uno como máx).
function actualizarSemaforoDinamico() {
  const g = S.selSig; if (!g || $("signalBox").hidden) return;
  const asp = aspectoBloque(g.from, g.to, g.sb || 0, g.m || 1, g.lines[0]);
  $("sigAspect").className = "sig-aspect " + asp;
  $("sigAspect").textContent = asp === "red" ? "● ROJO — bloque ocupado" : asp === "amber" ? "● ÁMBAR — próxima señal en rojo" : "● VERDE — vía libre";
  const det = trenesEsperandoEn(g.from, g.to)[0];   // por semáforo solo puede haber UN tren detenido
  if (det) {
    $("sigHeld").innerHTML = `<div class="held-card">
      ${lineImg(det.line, "line-logo big") || `<span class="badge" style="background:${det.color}">${det.line}</span>`}
      <div class="held-info"><b>Tren ${det.id}</b><span>→ ${nombre(det.pts[det.pts.length - 1].id)}</span>
      <span class="held-motivo">${ICON_PAUSE} ${MOTIVO_LABEL[det.esperaMotivo] || det.esperaMotivo} · +${Math.round(det.shift / 60)} min</span></div></div>`;
  } else {
    $("sigHeld").innerHTML = `<div class="held-empty">Sin tren detenido tras este semáforo.</div>`;
  }
}

// ---- Panel de enclavamiento de estación ---------------------------------------------------
// Los tres paneles (estación, semáforo e incidencias) comparten el MISMO rincón derecho y
// NUNCA se solapan: abrir uno cierra los otros dos. `keep` = el que se está abriendo.
function cerrarDocks(keep) {
  if (keep !== "station") { const s = $("stationBox"); if (s) { s.hidden = true; S.selSt = null; } }
  if (keep !== "signal") { const g = $("signalBox"); if (g) { g.hidden = true; S.selSig = null; } }
  if (keep !== "inc") { const inc = $("incPanel"); if (inc) { inc.classList.remove("open"); inc.setAttribute("aria-hidden", "true"); } }
}
function abrirEstacion(s) { S.selSt = s; cerrarDocks("station"); $("stationBox").hidden = false; buildStationPanel(); }

// Construye el panel UNA vez (o al cambiar un ajuste): crea el DOM y engancha los manejadores.
// No debe llamarse cada fotograma (destruiría los botones justo al pulsarlos).
function buildStationPanel() {
  const s = S.selSt; if (!s) return;
  const il = interlock(s.id);
  $("stName").textContent = s.name;
  const nDep = sidingsDe(s.id);
  $("stMeta").innerHTML = `<span class="st-logos">${s.lines.map((l) => lineImg(l, "line-logo sm")).join("")}</span><span>${il.tracks} vía(s)${il.corr.length > 1 ? " · " + il.corr.length + " corr." : ""} · <b id="parkCount">0</b>/${nDep} plazas depósito</span>`;
  // Semáforos INTERNOS de salida: uno por vía y sentido (A/B). Verde por defecto (automáticos);
  // se ponen en rojo solos cuando retienen a un tren (bloque/destino ocupado) y se pueden tocar
  // (clic = forzar/soltar). No se dibujan en el mapa: el tren esperando se queda DENTRO. En
  // estaciones con corredores, cada corredor (grupo de líneas) tiene su propia vía principal por
  // lado → varias vías principales por dirección (dos líneas que no se estorban). Clic en la vía:
  // reasigna la principal de su corredor (1º lado A, 2º lado B).
  let html = "";
  for (let c = 0; c < il.corr.length; c++) {
    const g = il.corr[c];
    if (il.corr.length > 1) {
      const et = g.lines.length ? g.lines.map((l) => lineImg(l, "line-logo sm")).join("") : "Corredor " + (c + 1);
      html += `<div class="corr-head">${et}<span class="corr-tag">${g.terminal ? "cabecera · vías no principales" : "vía ppal. propia"}</span></div>`;
    }
    for (const v of g.vias) {
      const esMain = g.mainA === v || g.mainB === v;
      // Dirección de la vía (en vez de A/B): A = sentido Norte, B = sentido Sur (geográfico, ver ladoEdge).
      // La interfaz se muestra VOLTEADA horizontalmente (Sur a la izquierda, Norte a la derecha) para
      // que las direcciones casen con la orientación del mapa (como las flechitas blancas). Es SOLO
      // presentación: `data-lado` (A/B) y toda la lógica de señalización quedan intactos.
      const tags = esMain ? [g.mainB === v ? "◀ dir. Sur" : "", g.mainA === v ? "dir. Norte ▶" : ""].filter(Boolean).join(" ") : '<span class="dep-tag">' + ICON_PARK + ' depósito</span>';
      html += `<div class="trackrow${esMain ? "" : " siding"}">
        <button class="sig" data-v="${v}" data-lado="B" title="Señal sentido Sur">S</button>
        <div class="viac ${esMain ? "main" : "dep"}" data-v="${v}" data-corr="${c}">Vía ${v + 1} ${tags}<span class="occ-label"></span></div>
        <button class="sig" data-v="${v}" data-lado="A" title="Señal sentido Norte">N</button>
      </div>`;
    }
  }
  const box = $("stTracks"); box.innerHTML = html;
  // Clic en la señal interna: fuerza/suelta el manual de esa vía y sentido.
  box.querySelectorAll(".sig").forEach((b) => b.onclick = () => { const v = +b.dataset.v, l = b.dataset.lado; il.green[v][l] = !il.green[v][l]; actualizarEstacionDinamico(); });
  // Clic en la vía: reasigna la principal de su corredor (1er clic → lado A, 2º → lado B).
  box.querySelectorAll(".viac").forEach((cell) => cell.onclick = () => {
    const v = +cell.dataset.v, g = il.corr[+cell.dataset.corr];
    if (g.terminal) return;   // cabecera: no tiene vía principal que reasignar
    if (g.mainA !== v) g.mainA = v; else if (g.mainB !== v) g.mainB = v;
    buildStationPanel();
  });

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
  const esperaSalida = new Set();       // "corredor|lado": un servicio espera salir por ahí (vía ppal.)
  const dormidoSalida = new Set();      // "v|lado": un durmiente saldrá por ahí (señal roja hasta que salga)
  const etiqueta = new Map();           // v -> texto de ocupación
  // Servicios EN CURSO parados aquí → en su vía principal (corredor + lado). Se toma la posición
  // REAL del reproductor (trace), no el horario: un tren está andanado en `s` si el trace lo sitúa
  // parado (frac≈1) con `s` como destino del cantón que ocupa. Así el panel coincide con el mapa.
  for (const t of S.trains) {
    const r = t.repro;
    if (!r || r.frac < 0.995 || r.b !== s.id) continue;
    const c = corrIdx(il, t.line);
    let via = viaPrincipal(il, t.side, t.line);
    // Principal ocupada (o corredor de cabecera sin principal) → otra vía libre de su corredor.
    if (via == null || il.occ[via] != null) { const libre = il.corr[c].vias.find((v) => il.occ[v] == null); if (libre != null) via = libre; }
    if (via == null) continue;
    il.occ[via] = t.id; etiqueta.set(via, t.line + " → " + nombre(t.pts[t.pts.length - 1].id));
    // Un tren que solo PARA (recoge pasajeros) NO pone su señal de salida en rojo; solo la pone
    // roja si está realmente RETENIDO por el bloque de delante (t.esperando). El tiempo de parada
    // reglamentario se cumple igual: no saldrá antes aunque la señal esté verde.
    if (t.esperando) esperaSalida.add(c + "|" + t.side);
  }
  // Flota DURMIENDO aquí → aparcada en las PLAZAS DE DEPÓSITO (vías no principales) libres, con
  // su id y su próximo itinerario; su semáforo de salida en rojo hasta su hora.
  const durm = durmientesEn(s.id).sort((a, b) => (a.h.svc ? a.h.svc.pts[0].d : DIA_FIN) - (b.h.svc ? b.h.svc.pts[0].d : DIA_FIN));
  const sidings = sidingTracksDe(s.id);
  let park = 0;
  for (const d of durm) {
    const via = sidings.find((v) => il.occ[v] == null);
    if (via == null) break;   // depósito lleno (la dispersión lo evita salvo desborde)
    il.occ[via] = d.unitId; park++;
    const svc = d.h.svc;
    const itin = svc ? "→ " + nombre(svc.pts[svc.pts.length - 1].id) + " · sale " + hhmm(svc.pts[0].d) : "pernocta";
    etiqueta.set(via, `${ICON_PARK} ${d.unitId} · ${d.line} ${itin}${d.h.depositoDe ? " · reubicado de " + nombre(d.h.depositoDe) : ""}`);
    if (svc) dormidoSalida.add(via + "|" + svc.side);
  }
  const pc = $("parkCount"); if (pc) pc.textContent = park;
  let prox = null, mejor = Infinity;
  for (const t of S.trains) { const te = S.simT - t.shift, seg = segmentoDe(t, te); if (!seg) continue; for (let j = Math.max(seg.i, 0); j < t.pts.length; j++) { if (t.pts[j].id === s.id) { const eta = t.pts[j].a - te; if (eta >= 0 && eta < mejor) { mejor = eta; prox = t; } break; } } }
  $("stNext").innerHTML = prox ? `<span class="stNextline">Próximo: <b>${prox.line}</b> → ${nombre(prox.pts[prox.pts.length - 1].id)} en ~${Math.round(mejor / 60)} min</span>` : `<span class="muted">Sin trenes próximos.</span>`;
  pintarProximos(s);
  // Aspecto EFECTIVO de cada señal interna (misma regla que las de tramo): ROJO si está forzada
  // a mano, si el primer bloque de vía de salida de ese lado está ocupado, si retiene a un
  // servicio que espera salir o a un durmiente en su plaza; ÁMBAR si su bloque está libre pero
  // la señal SIGUIENTE (aguas abajo) está en rojo; VERDE en otro caso.
  document.querySelectorAll("#stTracks .sig").forEach((btn) => {
    const v = +btn.dataset.v, l = btn.dataset.lado, c = il.corrDeVia[v], g = il.corr[c];
    const manualRojo = !il.green[v][l];
    const esMain = (l === "A" ? g.mainA : g.mainB) === v;
    const sal = salidaBloqueada(s.id, l);
    const autoRojo = (esMain && (esperaSalida.has(c + "|" + l) || sal.rojo)) || dormidoSalida.has(v + "|" + l);
    const rojo = manualRojo || autoRojo;
    const ambar = !rojo && esMain && sal.ambar;
    btn.classList.toggle("red", rojo); btn.classList.toggle("amber", ambar); btn.classList.toggle("green", !rojo && !ambar);
    btn.classList.toggle("auto", autoRojo && !manualRojo);
    btn.title = manualRojo ? "Rojo (forzado a mano) — clic para soltar" : autoRojo ? "Rojo automático — bloque de salida ocupado — clic para forzar" : ambar ? "Ámbar — próxima señal en rojo" : "Verde — clic para cerrar a mano";
  });
  document.querySelectorAll("#stTracks .viac").forEach((cell) => {
    const v = +cell.dataset.v, occ = il.occ[v];
    cell.classList.toggle("occ", !!occ);
    const lbl = cell.querySelector(".occ-label"); const h = occ ? " · " + (etiqueta.get(v) || occ) : "";
    if (lbl && lbl._h !== h) { lbl._h = h; lbl.innerHTML = h; }   // innerHTML: la etiqueta de depósito lleva el icono SVG; solo si cambia
  });
}

// Índice de parada ALCANZADA por un tren según el reproductor (trace): i si va por el cantón
// pts[i]→pts[i+1], i+1 si está parado en pts[i+1]. -1 si aún no ha salido / ya finalizó / no hay trace.
function reproIndex(t) {
  const r = t.repro;
  if (!r) return -1;
  for (let i = 0; i < t.pts.length - 1; i++) {
    if (t.pts[i].id === r.a && t.pts[i + 1].id === r.b) return r.frac >= 0.995 ? i + 1 : i;
  }
  return -1;
}
// Próximos trenes que POR HORARIO deben pasar por la estación `s`: los siguientes 15 aún no llegados,
// ordenados por hora de paso. Si su hora ya pasó y el tren no ha llegado, se marca el RETRASO (los
// minutos transcurridos desde esa hora). El horario NO manda sobre los semáforos; esto es informativo.
function pintarProximos(s) {
  const box = $("stNextList"); if (!box) return;
  const now = S.simT;
  const rows = [];
  for (const t of S.trains) {
    let j = -1;
    for (let k = 0; k < t.pts.length; k++) { if (t.pts[k].id === s.id && !t.pts[k].pass) { j = k; break; } }
    if (j < 0) continue;
    const ta = t.pts[j].a;
    const ri = reproIndex(t);   // posición REAL del tren en el trace (−1 si no está en la ventana)
    let lateMin = 0;
    if (ri >= 0) {
      // En ruta: si ya alcanzó/pasó la estación, no es "próximo". Si va con retraso, se muestra el
      // retraso REAL del tren (del trace del motor), no el derivado de comparar con su horario.
      if (ri >= j) continue;
      lateMin = Math.round((t.reproDelay || 0) / 60);
    } else {
      // Sin posición en el trace: o ya circuló (jornada acabada) o aún no ha salido (futuro).
      if (t.pts[t.pts.length - 1].a < now) continue;   // ya terminó su servicio → no es próximo
      if (ta < now) continue;                           // sin trace y hora pasada → no inventar retraso
    }
    if (ta > now + 3 * 3600) continue;                  // solo las próximas ~3 h
    rows.push({ ta, line: t.line, dest: nombre(t.pts[t.pts.length - 1].id), lateMin });
  }
  rows.sort((a, b) => a.ta - b.ta);
  const top = rows.slice(0, 15);
  box.innerHTML = top.length
    ? top.map((r) => {
        const when = r.lateMin > 0
          ? `<span class="np-time late">${hhmm(r.ta)} <b>+${r.lateMin}′</b></span>`
          : `<span class="np-time">${hhmm(r.ta)}</span>`;
        return `<div class="np-row${r.lateMin > 0 ? " is-late" : ""}">${lineImg(r.line, "line-logo xs") || ""}<span class="np-line">${r.line}</span><span class="np-dest">→ ${r.dest}</span>${when}</div>`;
      }).join("")
    : `<div class="muted" style="padding:6px">Sin trenes programados próximamente.</div>`;
}

// ---- Actualización del GTFS (descarga oficial o subida manual) ----------------------------
// Se eligen en la pantalla previa: "GTFS nuevo" (descarga oficial) o "Importar horarios" (ZIP
// propio). `ejecutarJobGtfs(start)` arranca el trabajo del servidor y resuelve true/false al acabar
// (polling de /api/gtfs/status); si va bien, recarga red + horarios antes de generar el día.
async function ejecutarJobGtfs(start) {
  const st = $("gtfsStatus");
  let r;
  try { r = await (await start()).json(); } catch (e) { r = { started: false, reason: String(e) }; }
  if (!r.started) { st.textContent = r.reason || "No se pudo iniciar."; return false; }
  for (;;) {
    await new Promise((res) => setTimeout(res, 900));
    let j; try { j = await (await fetch("/api/gtfs/status")).json(); } catch (e) { j = {}; }
    if (j.error) { st.textContent = "Error: " + j.error; return false; }
    st.textContent = j.step || "Actualizando…";
    if (j.done) {
      const s = j.stats;
      if (s) st.textContent = `Actualizado: ${s.n_routes} líneas · ${s.n_trips} viajes · ${s.n_stops} estaciones.`;
      await cargarRed(); await cargarHorarios(); construirSemaforos();
      fijarHoraInicioDefecto();   // el GTFS nuevo puede empezar a otra hora
      await pintarInfoGtfs();
      return true;
    }
  }
}
// "GTFS guardado del …": fecha del conjunto cargado ahora (opción por defecto del selector).
async function pintarInfoGtfs() {
  const el = $("gtfsSavedInfo"); if (!el) return;
  try {
    const j = await (await fetch("/api/gtfs/info")).json();
    const [y, m, d] = (j.date || "").split("-");
    el.textContent = (j.date ? `Del ${d}/${m}/${y}` : "Fecha desconocida") + ` · ${j.n_lines} líneas · ${j.n_trips.toLocaleString((window.LOCALE?LOCALE():"es-ES"))} viajes (por defecto)`;
  } catch (e) { el.textContent = "El que está cargado ahora (por defecto)"; }
}

// ---- Generador de incidencias -------------------------------------------------------------
const KIND = { averia_puertas: "stop", caida_catenaria: "slow", averia_mecanica: "block" };
const INCID = {
  averia_puertas: { nombre: "Avería de puertas", dur: 5 },
  caida_catenaria: { nombre: "Falta de tensión", dur: 30 },
  averia_mecanica: { nombre: "Avería mecánica grave", dur: 90 },
};
function poblarSelectores() {
  // Líneas (leyenda + filtros). El antiguo generador de incidencias PROGRAMADAS se sustituyó por
  // "Lanzar incidencia ahora" (ver el módulo `incidentTemplates`), así que aquí ya no se puebla.
  const ln = S.net.lines.map((l) => `<option value="${l.line}">${l.line}</option>`).join("");
  $("tFiltLine").innerHTML = '<option value="">Todas</option>' + ln;
}
function rellenarDonde() {
  const scope = $("incScope").value, sel = $("incWhere");
  if (scope === "station") {
    sel.innerHTML = [...S.stationById.values()].filter((s) => !s.junction).sort((a, b) => a.name.localeCompare(b.name)).map((s) => `<option value="${s.id}">${s.name}</option>`).join("");
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
  $("incList").innerHTML = S.incidentes.length
    ? S.incidentes.slice().reverse().map((i) =>
        `<li class="${i.estado === "finalizada" ? "done" : ""}">${hhmm(i.whenSec)} · ${i.nombre} · ${i.label} <span class="muted">[${i.estado}]</span></li>`).join("")
    : '<li class="empty">Sin incidencias programadas.</li>';
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
    `<div class="t${f.t.esperando ? " esperando" : ""}">${lineImg(f.t.line, "line-logo") || `<span class="badge" style="background:${f.t.color}">${f.t.line}</span>`}
      <span>${f.t.esperando ? ICON_PAUSE + " " : ""}${f.ubic} → ${nombre(f.t.pts[f.t.pts.length - 1].id)}${f.t.esperando ? ` <span class="muted">(${MOTIVO_LABEL[f.t.esperaMotivo] || f.t.esperaMotivo})</span>` : ""}</span>
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
  // Las llegadas tarde aún no sumadas (el último paso, o un salto con "Terminar día") entran ya.
  while (S.arrPtr < S.arrivals.length && S.arrivals[S.arrPtr].t <= S.simT) { S.kpi += S.arrivals[S.arrPtr].delay / 60; S.arrPtr++; }
  const r = resumenJornada();
  const loc = window.LOCALE ? LOCALE() : "es-ES", n0 = (x) => Math.round(x).toLocaleString(loc);
  $("finStats").innerHTML = `
    <div class="stat"><span>Minutos de retraso acumulados</span><b>${n0(r.kpi_min)} min</b></div>
    <div class="stat"><span>Trenes con retraso</span><b>${r.late_trains}</b></div>
    <div class="stat"><span>Pico de retraso vivo</span><b>${r.peak_live} min</b></div>
    <div class="stat" title="Tren·minutos esperando ante señales en rojo (todas las retenciones del día). No depende de ir adelantado o atrasado respecto al horario: sirve para comparar modos de horario."><span>Tiempo retenido ante señales</span><b>${n0(r.held_min)} min · ${n0(r.held_events)} ret.</b></div>` +
    (r.stuck ? `<div class="stat warn" title="Trenes que a las 24:00 seguían sin llegar a su destino pese a que su llegada prevista era anterior (atascados: no cuentan como llegada tarde)."><span>Trenes atascados al cierre</span><b>${r.stuck}</b></div>` : "") +
    `<div class="stat"><span>Incidencias del día</span><b>${r.incidents}</b></div>
    <div class="stat"><span>Horarios</span><b>${escHtml(r.sched)}</b></div>`;
  $("finDia").hidden = false;
  $("finReopen").hidden = true;
  guardarEnHistorial(r);
}
// Resultados finales de la jornada (fin del día + historial de runs).
function resumenJornada() {
  const tr = S.trace || {};
  // Atascados: su último registro es al cierre, sin estar en su destino, y debían llegar antes.
  const fin = new Map(S.trains.map((t) => [t.id, t.pts[t.pts.length - 1]]));
  let stuck = 0;
  for (const rt of tr.trains || []) {
    const last = fin.get(rt.id), n = rt.a.length;
    if (last && rt.t0 + (n - 1) * tr.dt >= tr.end - 10 && tr.ids[rt.b[n - 1]] !== last.id && last.a < tr.end) stuck++;
  }
  return {
    t: Date.now(), seed: S.traceSeed || 0,
    sched: S.cadId ? "Cadenciado · " + (S.cadName || "") : tr.free_run ? "Sin horarios (modo caos)" : "GTFS",
    vvar: (/[?&]vvar=([\d.]+)/.exec(S.traceSel || "") || [])[1] || null,
    kpi_min: S.kpi, late_trains: new Set(S.arrivals.map((a) => a.train)).size,
    peak_live: S.delayTL ? Math.round(S.delayTL.max) : 0,
    held_min: (tr.held_total_s || 0) / 60, held_events: tr.held_total_events || 0, stuck,
    incidents: (S.dayIncidents || []).length, ended_early: !!S.endedEarly,
    top: (tr.hotspots || []).slice(0, 3).map((h) => ({ n: h.from_name + " → " + h.to_name, min: Math.round(h.held_s / 60) })),
  };
}

// ---- Historial de runs visuales (últimas 10, localStorage "rod-visual-history") -----------
const HIST_KEY = "rod-visual-history", HIST_MAX = 10;
function historial() { try { const v = JSON.parse(localStorage.getItem(HIST_KEY)); return Array.isArray(v) ? v : []; } catch (e) { return []; } }
function guardarEnHistorial(r) {
  if (S.histSaved) return;   // una entrada por jornada (el balance se puede reabrir)
  S.histSaved = true;
  try { localStorage.setItem(HIST_KEY, JSON.stringify([r, ...historial()].slice(0, HIST_MAX))); } catch (e) { /* sin storage */ }
  pintarBotonHistorial();
}
function pintarBotonHistorial() { const b = $("preHist"); if (b) { const n = historial().length; b.hidden = !n; b.textContent = `Historial de runs visuales (${n})`; } }
function abrirHistorial() {
  const h = historial(), loc = window.LOCALE ? LOCALE() : "es-ES", n0 = (x) => Math.round(x).toLocaleString(loc);
  $("histTbl").innerHTML = h.length ? `<thead><tr><th>Fecha</th><th>Semilla</th><th>Horarios</th><th>Incid.</th><th>Min retraso</th><th>Trenes tarde</th><th>Pico vivo</th><th>Retenido</th><th>Atascados</th><th>Tramos más problemáticos</th></tr></thead><tbody>` +
    h.map((r) => `<tr><td>${new Date(r.t).toLocaleString(loc, { dateStyle: "short", timeStyle: "short" })}${r.ended_early ? '<span class="tag">terminado</span>' : ""}</td>
      <td class="num">${r.seed}</td><td>${escHtml(r.sched)}${r.vvar ? `<span class="tag">vvar ${r.vvar} %</span>` : ""}</td>
      <td class="num">${r.incidents}</td><td class="num">${n0(r.kpi_min)}</td><td class="num">${r.late_trains}</td><td class="num">${r.peak_live} min</td>
      <td class="num">${n0(r.held_min)} min</td><td class="num">${r.stuck || 0}</td>
      <td class="top">${(r.top || []).map((x) => `${escHtml(x.n)} · ${x.min} min`).join("<br>") || "–"}</td></tr>`).join("") + "</tbody>"
    : `<tr><td class="hist-empty">Todavía no hay runs visuales terminadas.</td></tr>`;
  $("histDlg").hidden = false;
}
$("finHist").addEventListener("click", abrirHistorial);
$("preHist").addEventListener("click", abrirHistorial);
$("histClose").addEventListener("click", () => $("histDlg").hidden = true);
$("histDlg").addEventListener("click", (e) => { if (e.target === $("histDlg")) $("histDlg").hidden = true; });
$("histClear").addEventListener("click", async () => {
  if (!(await confirmar({ title: "¿Borrar el historial de runs visuales?", html: "<p>Se borran los resultados guardados en este navegador.</p>", yes: "Borrar" }))) return;
  try { localStorage.removeItem(HIST_KEY); } catch (e) { /* sin storage */ }
  pintarBotonHistorial(); abrirHistorial();
});
pintarBotonHistorial();

// ---- "Terminar día": salta al final de la jornada (la grabación del motor ya cubre todo el día) --
$("btnFinDia").addEventListener("click", async () => {
  if (!S.trace || S.finished) return;
  const sp = S.speedKey; setSpeed("PAUSA");
  const ok = await confirmar({ title: "¿Terminar el día?", html: `<p>Se salta de las <b>${hhmm(S.simT)}</b> al final de la jornada y se muestra el balance. La simulación ya está calculada entera, así que los resultados son los mismos que viéndola hasta el final.</p>`, yes: "Terminar día" });
  if (!ok) { setSpeed(sp); return; }
  S.endedEarly = true;
  S.simT = DIA_FIN; pasoRepro(0);
});

// ---- Ranking de tramos más problemáticos (fin del día) ------------------------------------
// El motor mide, SOLO en la run visual, cuánto tiempo han esperado los trenes ante la señal de
// entrada de cada tramo (cantón dirigido) y manda los 15 peores en el trace (`hotspots`).
function escHtml(x) { return String(x).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c])); }
function abrirTramos() {
  const hs = (S.trace && S.trace.hotspots) || [];
  const fmtMin = (sec) => (sec / 60).toLocaleString(window.LOCALE ? LOCALE() : "es-ES", { maximumFractionDigits: sec < 600 ? 1 : 0 });
  const max = hs.length ? hs[0].held_s : 1;
  $("hotList").innerHTML = hs.length ? hs.map((h, i) => {
    const lines = (h.lines || []).slice(0, 5).map(([l, sec]) =>
      `<span class="ln">${lineImg(l, "line-logo xs") || l} ${Math.round(100 * sec / Math.max(1, h.held_s))} %</span>`).join("");
    return `<li data-i="${i}" title="Ver el tramo en el mapa">
      <span class="rk">${i + 1}</span>
      <span class="nm">${escHtml(h.from_name)} → ${escHtml(h.to_name)}</span>
      <span class="tot">${fmtMin(h.held_s)} min</span>
      <span class="bar"><i style="width:${Math.max(2, 100 * h.held_s / max)}%"></i></span>
      <span class="meta"><span>${h.events} retenciones</span><span>máx. ${fmtMin(h.max_s)} min seguidos</span>${lines}</span>
    </li>`;
  }).join("") : `<li class="hot-empty">Ningún tren ha quedado retenido ante una señal en esta jornada.</li>`;
  $("hotList").querySelectorAll("li[data-i]").forEach((li) => li.onclick = () => verTramoEnMapa(hs[+li.dataset.i]));
  $("hotDlg").hidden = false;
}
// Resalta el tramo en el mapa y deja un botón para volver al balance del día.
let hotLayer = null;
function verTramoEnMapa(h) {
  const pts = viaPts(h.from, h.to);
  if (!pts.length) return;
  if (!hotLayer) hotLayer = L.layerGroup().addTo(map);
  hotLayer.clearLayers();
  L.polyline(pts, { color: "#ff3b3b", weight: 9, opacity: 0.85, lineCap: "round" }).addTo(hotLayer)
    .bindTooltip(`${escHtml(h.from_name)} → ${escHtml(h.to_name)} · ${Math.round(h.held_s / 60)} min retenidos`, { sticky: true });
  map.fitBounds(L.latLngBounds(pts).pad(1.2), { maxZoom: 14 });
  $("hotDlg").hidden = true; $("finDia").hidden = true; $("finReopen").hidden = false;
}
$("finTramos").addEventListener("click", abrirTramos);
$("hotClose").addEventListener("click", () => $("hotDlg").hidden = true);
$("hotDlg").addEventListener("click", (e) => { if (e.target === $("hotDlg")) $("hotDlg").hidden = true; });
$("finReopen").addEventListener("click", () => {
  if (hotLayer) hotLayer.clearLayers();
  $("finReopen").hidden = true; $("finDia").hidden = false;
});

// ---- KPI, leyenda -------------------------------------------------------------------------
function actualizarKpi() {
  $("kDelay").textContent = Math.round(S.kpi).toLocaleString((window.LOCALE?LOCALE():"es-ES"));
  // Retraso VIVO de la red: minutos de retraso sumados de todos los trenes en marcha AHORA (sube y
  // baja, a diferencia del acumulado monótono de al lado). `S.delayNow` se recalcula cada fotograma.
  $("kLive").textContent = Math.round(S.delayNow || 0).toLocaleString((window.LOCALE?LOCALE():"es-ES"));
}
// Orden de la leyenda: por REGIÓN (R rodalies/regionals, luego RG, RL, RT como bloques separados)
// y, dentro de cada bloque, por número (con el sufijo N/S detrás). Solo estético.
function lineSortKey(name) {
  const m = /^(R[GLT]?)(\d*)([NS]?)/i.exec(name) || [];
  const prefix = (m[1] || "R").toUpperCase();
  const groupOrder = { R: 0, RG: 1, RL: 2, RT: 3 }[prefix];
  return { group: groupOrder === undefined ? 9 : groupOrder, prefix, num: parseInt(m[2] || "0", 10) || 0, suf: m[3] || "" };
}
function pintarLeyenda() {
  const lines = S.net.lines.slice().sort((a, b) => {
    const ka = lineSortKey(a.line), kb = lineSortKey(b.line);
    return ka.group - kb.group || ka.num - kb.num || ka.suf.localeCompare(kb.suf) || a.line.localeCompare(b.line);
  });
  let html = "", prevGroup = null;
  for (const l of lines) {
    const g = lineSortKey(l.line).group;
    if (prevGroup !== null && g !== prevGroup) html += `<div class="legend-sep" aria-hidden="true"></div>`;
    prevGroup = g;
    html += `<div class="row" data-line="${l.line}" title="Clic: aislar ${l.line} en el mapa">${lineImg(l.line, "line-logo") || `<span class="sw" style="background:${l.color}"></span>`}` +
      `<span class="ln-name">${l.line}</span><span class="ln-bar"><i style="--c:${l.color}"></i></span><span class="ln-val">–</span></div>`;
  }
  $("legend").innerHTML = html;
  // Referencias a las celdas vivas (retraso por línea) para actualizarlas sin recrear el DOM.
  S.legendRows = new Map();
  $("legend").querySelectorAll(".row").forEach((r) => S.legendRows.set(r.dataset.line, { bar: r.querySelector(".ln-bar > i"), val: r.querySelector(".ln-val"), last: null }));
}

// ---- Vista "mapa por velocidad máxima (Vmax)" ---------------------------------------------
// Colorea cada cantón por su Vmax de vía (del perfil ADIF, servida en el JSON de red). Los cantones
// con una LTV (limitación temporal) vigente se marcan con un color BRILLANTE distinto. Es solo una
// capa de color alternativa: no toca la simulación (los trenes siguen reproduciendo el trace).
function vmaxColor(z) {
  if (!z || !z.v) return "#5b6472";                  // sin cobertura ADIF
  if (z.ltv) return "#ff33d6";                        // LTV: color brillante que resalta
  const v = z.v;
  return v >= 140 ? "#ef4444" : v >= 120 ? "#f59e0b" : v >= 100 ? "#22c55e" : v >= 70 ? "#06b6d4" : "#2563eb";
}
const VMAX_LEYENDA = [
  ["#2563eb", "< 70"], ["#06b6d4", "70–99"], ["#22c55e", "100–119"],
  ["#f59e0b", "120–139"], ["#ef4444", "≥ 140"], ["#ff33d6", "LTV (temporal)"], ["#5b6472", "sin dato"],
];
function pintarVmaxLegend() {
  const el = $("vmaxLegend"); if (!el) return;
  el.hidden = !S.vmaxView;
  if (S.vmaxView) {
    el.innerHTML = `<div class="vmax-title">Vmax de vía (km/h)</div>` +
      VMAX_LEYENDA.map(([c, t]) => `<div class="vmax-item"><span class="vmax-sw" style="background:${c}"></span>${t}</div>`).join("");
  }
}
function aplicarVistaVmax(on) {
  S.vmaxView = on;
  dibujarLineas();       // redibuja con el color por Vmax (dibujarLineas ya lee S.vmaxView)
  pintarVmaxLegend();
}

// ---- Controles ----------------------------------------------------------------------------
function setSpeed(k) { S.speedKey = k; S.speed = VELOCIDADES[k]; document.querySelectorAll("#speeds button").forEach((b) => b.classList.toggle("active", b.dataset.speed === k)); }
$("speeds").addEventListener("click", (e) => { const b = e.target.closest("button"); if (b) setSpeed(b.dataset.speed); });
$("showSignals").addEventListener("change", construirSemaforos);
$("showVmax").addEventListener("change", (e) => aplicarVistaVmax(e.target.checked));
$("btnTrenes").addEventListener("click", () => { const p = $("trenesPanel"); p.hidden = !p.hidden; refrescarTrenes(); });
$("trenesClose").addEventListener("click", () => $("trenesPanel").hidden = true);
["tFiltLine", "tOnlyLate", "tSort"].forEach((id) => $(id).addEventListener("change", refrescarTrenes));
$("finRepetir").addEventListener("click", () => location.reload());
// Seguimiento de retrasos de la jornada que se acaba de ver (series + análisis del motor, que
// vienen en el trace; si se lanzó una incidencia "ahora", es el trace re-simulado con ella).
$("finSeguimiento").addEventListener("click", () => {
  const tr = S.trace; if (!tr || !window.DelayTrack) return;
  DelayTrack.open({ track: tr.track, analysis: tr.analysis, incidents: S.dayIncidents || [] },
    { title: "Simulación visual · semilla " + (S.traceSeed || 0) });
});

// Cajón de incidencias del día.
$("btnIncidDia").addEventListener("click", () => { const p = $("incidDiaPanel"); p.hidden = !p.hidden; pintarIncidenciasDia(); });
$("incidDiaClose").addEventListener("click", () => $("incidDiaPanel").hidden = true);

// Pantalla PREVIA — selector de horarios: el campo de fichero solo aparece con "Importar".
const schedMode = () => (document.querySelector('input[name="sched"]:checked') || {}).value || "saved";
// "Sin horarios" pide confirmación (explica qué va a pasar); si se cancela, vuelve a la opción previa.
let schedPrev = "saved";
$("schedOpts").addEventListener("change", async () => {
  if (schedMode() === "free" && schedPrev !== "free" && !(await confirmarCaos())) {
    const r = [...document.querySelectorAll('input[name="sched"]')].find((x) => x.value === schedPrev);
    if (r) r.checked = true;
  }
  schedPrev = schedMode();
  $("gtfsFileWrap").hidden = schedMode() !== "import"; $("gtfsStatus").textContent = "";
});
function confirmarCaos() {
  return confirmar({
    kicker: '<span class="chaos-tag">modo caos</span> Experimento',
    title: "¿Simular sin horarios?",
    html: `<ul class="chaos-list">
      <li>Cada tren conserva <b>solo su hora de salida</b> de la estación de cabecera.</li>
      <li>En el resto de paradas sale en cuanto cumple la <b>parada mínima reglamentaria</b>, aunque vaya adelantado.</li>
      <li>Las señales solo miran si hay un tren delante: la vía única ya no reserva paso a los regionales según su horario.</li>
      <li>El retraso se sigue midiendo contra el horario GTFS, así que casi nadie irá «tarde»: fíjate en las retenciones y en los <b>tramos más problemáticos</b> al final del día.</li>
    </ul>`,
    yes: "Activar modo caos",
  });
}
// Diálogo de confirmación genérico (Andana) → Promise<boolean>. Esc / clic fuera = cancelar.
function confirmar({ kicker = "", title, html = "", yes = "Confirmar" }) {
  return new Promise((res) => {
    const dlg = $("confirmDlg");
    $("confirmKicker").innerHTML = kicker; $("confirmKicker").hidden = !kicker;
    $("confirmTitle").textContent = title; $("confirmBody").innerHTML = html; $("confirmYes").textContent = yes;
    const fin = (v) => { dlg.hidden = true; document.removeEventListener("keydown", esc); res(v); };
    const esc = (e) => { if (e.key === "Escape") fin(false); };
    $("confirmYes").onclick = () => fin(true);
    $("confirmNo").onclick = () => fin(false);
    dlg.onclick = (e) => { if (e.target === dlg) fin(false); };
    document.addEventListener("keydown", esc);
    dlg.hidden = false;
    $("confirmYes").focus();
  });
}
// Horarios CADENCIADOS guardados por el creador (/horarios) en este navegador. localStorage
// "rod-cadences" = [{ id, name, updated, spec: { name, markers: [{ line, pat, min, every, alt }] } }]
// (el formato lo fija game/web/cadencia.js). Se envían al servidor al confirmar (`cargarCadencia`).
const CAD_KEY = "rod-cadences";
function cadenciasGuardadas() {
  try { const v = JSON.parse(localStorage.getItem(CAD_KEY)); return Array.isArray(v) ? v : []; } catch (e) { return []; }
}
function pintarCadencias() {
  const box = $("schedCads"); if (!box) return;
  const loc = window.LOCALE ? LOCALE() : "es-ES";
  box.innerHTML = cadenciasGuardadas().map((c) => {
    const n = ((c.spec && c.spec.markers) || []).length + ((c.spec && c.spec.groups) || []).length;
    const f = c.updated ? new Date(c.updated).toLocaleDateString(loc) : "";
    return `<label class="sched-opt"><input type="radio" name="sched" value="cad:${escHtml(c.id)}">
      <span><b>${escHtml(c.name || "Sin nombre")} <span class="cad-tag">cadenciado</span></b>
      <small>${n} marcador${n === 1 ? "" : "es"}${f ? " · guardado el " + f : ""}</small></span></label>`;
  }).join("");
  // Enlace "Ver en simulador" de /numeros con un horario cadenciado: lo deja elegido.
  const want = new URLSearchParams(location.search).get("cad");
  const r = want && [...box.querySelectorAll("input")].find((x) => x.value === "cad:" + want);
  if (r) { r.checked = true; schedPrev = r.value; }
}
pintarCadencias();
// Carga en el servidor la malla del horario cadenciado `localId` → id de servidor (`cad=`).
async function cargarCadencia(localId) {
  const c = cadenciasGuardadas().find((x) => x.id === localId);
  if (!c) throw new Error("El horario cadenciado ya no está guardado en este navegador.");
  const r = await (await fetch("/api/cadence/load", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ name: c.name, markers: c.spec.markers || [], groups: c.spec.groups || [] }) })).json();
  if (!r.ok) throw new Error(r.error || "No se pudo generar el horario cadenciado.");
  return { id: r.id, name: c.name, warnings: r.warnings || [], n: r.n_trains };
}
$("gtfsFile").addEventListener("change", () => { const f = $("gtfsFile").files[0]; $("gtfsFileName").textContent = f ? f.name : "Elegir fichero .zip…"; });
// "Empezar a ver desde" (HH:MM) → segundos de jornada. La jornada se simula ENTERA (es una
// grabación del motor); esto solo sitúa el cabezal de reproducción.
// Hora de inicio POR DEFECTO = salida del PRIMER tren del horario cargado (el motor arranca la
// jornada ahí, no a las 05:00 fijas). Si el usuario ya tocó el campo, se respeta su hora. También
// ajusta el reloj/rótulo mientras no hay jornada generada.
function fijarHoraInicioDefecto() {
  let first = Infinity;
  for (const t of S.trains) if (t.pts[0].d < first) first = t.pts[0].d;
  if (!Number.isFinite(first)) return;
  const inp = $("preStart");
  if (inp && !inp.dataset.touched) inp.value = hhmm(first);
  if (!S.trace) {
    S.simT = first;
    const sub = document.querySelector(".clock-sub");
    if (sub) sub.textContent = `Jornada · ${hhmm(first)} → 00:00`;
  }
}
$("preStart").addEventListener("input", () => { $("preStart").dataset.touched = "1"; });

// ── Variabilidad de velocidad (conducción real): interruptor + factor p (%). El motor hace circular
// cada tren, en cada tramo entre señales, a Vmax × f con f gaussiana de media 1−p truncada a [1−2p, 1]
// (p definido a 120 km/h; el motor lo escala con la Vmax del punto). Se guarda
// por visor (localStorage); un enlace "Ver en simulador" de /numeros (`&vvar=`) manda en esa visita.
const VVAR_KEY = "rod-vvar-visual";
function vvarPct() {
  const p = parseFloat($("preVvar").value);
  return Number.isFinite(p) ? Math.min(25, Math.max(0.5, p)) : 3;
}
// Parámetro de query para el motor (vacío = apagado → todos a Vmax).
function vvarQuery() { return $("preVvarOn").checked ? "&vvar=" + vvarPct() : ""; }
function pintarVvar() {
  const on = $("preVvarOn").checked, p = vvarPct();
  const fmt = (x) => x.toLocaleString(window.LOCALE ? LOCALE() : "es-ES", { maximumFractionDigits: 2 });
  $("preVvarRow").hidden = !on; $("preVvarHint").hidden = !on;
  if (on) $("preVvarHint").textContent = `A 120 km/h: media Vmax − ${fmt(p)} % · entre Vmax − ${fmt(2 * p)} % y Vmax. Escala con la Vmax (a 60 km/h, la mitad) y se sortea en cada tramo entre señales. Súbelo en días de lluvia.`;
}
function guardarVvar() {
  try { localStorage.setItem(VVAR_KEY, JSON.stringify({ on: $("preVvarOn").checked, p: vvarPct() })); } catch (e) { /* sin storage */ }
}
(function iniciarVvar() {
  try {
    const v = JSON.parse(localStorage.getItem(VVAR_KEY));
    if (v) { $("preVvarOn").checked = !!v.on; if (Number.isFinite(v.p)) $("preVvar").value = v.p; }
  } catch (e) { /* sin storage */ }
  const q = new URLSearchParams(location.search);
  if (q.get("from") === "numeros") {
    const p = parseFloat(q.get("vvar"));
    $("preVvarOn").checked = Number.isFinite(p) && p > 0;
    if ($("preVvarOn").checked) $("preVvar").value = p;
  }
  pintarVvar();
  $("preVvarOn").addEventListener("change", () => { pintarVvar(); guardarVvar(); });
  $("preVvar").addEventListener("input", () => { pintarVvar(); guardarVvar(); });
})();

function horaInicioSeg() {
  const [h, m] = ($("preStart").value || "05:00").split(":").map(Number);
  return (h || 0) * 3600 + (m || 0) * 60;
}
// Al confirmar: (1) si se pidió, descarga/importa el GTFS y recarga red+horarios; (2) genera el día
// con la semilla y la selección de incidencias del modo visual; (3) salta a la hora de inicio.
$("preConfirm").addEventListener("click", async () => {
  const seed = +$("preSeed").value || 0, mode = schedMode();
  const btn = $("preConfirm"), st = $("preStatus");
  btn.disabled = true;
  try {
    if (mode === "new") {
      st.textContent = "Descargando el GTFS de hoy (puede tardar un minuto)…";
      if (!(await ejecutarJobGtfs(() => fetch("/api/gtfs/update")))) { st.textContent = "No se pudo actualizar el GTFS (ver arriba)."; return; }
    } else if (mode === "import") {
      const f = $("gtfsFile").files[0];
      if (!f) { st.textContent = "Elige el fichero .zip a importar."; return; }
      st.textContent = "Importando " + f.name + "…";
      if (!(await ejecutarJobGtfs(() => fetch("/api/gtfs/upload", { method: "POST", body: f })))) { st.textContent = "No se pudo importar el GTFS (ver arriba)."; return; }
    }
    // Horario cadenciado: se genera en el servidor y se recargan los horarios del visor (los ids de
    // tren del trace deben coincidir). Volver a GTFS tras uno cadenciado también recarga.
    let cad = null;
    if (mode.startsWith("cad:")) {
      st.textContent = "Generando el horario cadenciado…";
      try { cad = await cargarCadencia(mode.slice(4)); } catch (e) { st.textContent = String(e.message || e); return; }
    }
    if ((cad ? cad.id : null) !== (S.cadId || null)) {
      S.cadId = cad ? cad.id : null; S.cadName = cad ? cad.name : "";
      await cargarHorarios(); construirSemaforos(); fijarHoraInicioDefecto();
    }
    st.textContent = "Generando la jornada con el motor (unos segundos)…";
    // La variabilidad de velocidad, el horario cadenciado y el modo sin horarios viajan con la
    // selección: "lanzar incidencia ahora" re-simula exactamente el mismo día.
    const extra = (S.cadId ? "&cad=" + S.cadId : "") + (mode === "free" ? "&free=1" : "");
    const ok = await generarDia(seed, (S.tplSelQuery ? S.tplSelQuery() : "") + vvarQuery() + extra, horaInicioSeg());
    if (ok && cad && cad.warnings.length) console.warn("Horario cadenciado:", cad.warnings.join(" "));
    if (!ok) { st.textContent = "No se pudo generar la jornada (ver Estado de la red)."; return; }
    $("preDay").classList.add("hidden");
    st.textContent = "";
  } finally {
    btn.disabled = false;
  }
});
window.addEventListener("resize", () => map.invalidateSize());

// ---- Bucle --------------------------------------------------------------------------------
const PASO_MAX_S = 6;   // s de juego por sub-paso: acota el salto por fotograma (señalización exacta a ×25)
let ultimo = performance.now(), acumUI = 0;
function bucle(now) {
  const dt = Math.min(0.1, (now - ultimo) / 1000); ultimo = now;
  if (S.speed > 0 && !S.finished) {
    let rem = dt * FACTOR_BASE * S.speed;              // segundos de juego a avanzar este fotograma
    // Motor visual ÚNICO: siempre el reproductor del trace del motor Rust (no hay motor JS).
    while (rem > 1e-6 && !S.finished) { const st = Math.min(PASO_MAX_S, rem); pasoRepro(st); rem -= st; }
  }
  render();
  actualizarSemaforosAspecto();      // color de las señales del mapa: CADA fotograma (fluido a ×25)
  actualizarSemaforoDinamico();      // aspecto del panel de señal abierto: también por fotograma
  actualizarIncidenciasDia();        // cajón de incidencias en rojo si hay alguna en curso
  acumUI += dt;
  if (acumUI > 0.5) {
    acumUI = 0; actualizarKpi(); refrescarTrenes(); actualizarEstacionDinamico(); pintarIncidenciasDia();
    actualizarRetrasoLineas(); pintarGraficoRetraso();
  }
  requestAnimationFrame(bucle);
}

// ---- Arranque -----------------------------------------------------------------------------
async function iniciar() {
  // Fuente del horario: GTFS; `?source=optimized` (enlace directo) carga la malla optimizada.
  S.source = new URLSearchParams(location.search).get("source") === "optimized" ? "optimized" : "gtfs";
  pintarInfoGtfs();
  try {
    await cargarRed(); await cargarHorarios(); construirSemaforos();
    fijarHoraInicioDefecto();
    setTimeout(() => map.invalidateSize(), 120);
    // NO se genera el día directamente: primero la PANTALLA PREVIA de incidencias. El día (trace)
    // se genera al pulsar "Confirmar" (con o sin incidencias). Ver `generarDia`.
    const seedUrl = +(new URLSearchParams(location.search).get("seed"));
    if (Number.isFinite(seedUrl) && $("preSeed")) $("preSeed").value = seedUrl;
    rodLoad(false);   // red+horarios OK: oculta el overlay de carga
  } catch (e) { $("netInfo").textContent = "Error cargando datos: " + e; rodLoad(false); }
  requestAnimationFrame(bucle);
}
// Genera el día: pide el TRACE físico al motor con la semilla elegida y reproduce la física real.
// Es el ÚNICO motor visual (no hay respaldo JS). Se llama al confirmar la pantalla previa.
// `selQ` = selección de plantillas del modo visual (`&sel=…`); `startSec` = hora desde la que se
// empieza a VER (la grabación cubre toda la jornada). Devuelve true si el trace cargó.
async function generarDia(seed, selQ, startSec) {
  rodLoad(true, "Conectando con el motor", "pantografo");
  $("netInfo").textContent = $("netInfo").textContent.replace(/ · .*$/, "") + " · Calculando la simulación física (motor Rust)…";
  try {
    await cargarTrace(seed, selQ);
    // Salta el cabezal a la hora pedida (acotada a la jornada del trace). Un paso de 0 s sitúa los
    // trenes y pone al día el contador de retraso con las llegadas tarde anteriores a esa hora.
    if (startSec > S.trace.start) S.simT = Math.min(startSec, S.trace.end - 60);
    pasoRepro(0);
    pintarIncidenciasDia();
    $("btnFinDia").disabled = false;
    $("netInfo").textContent = $("netInfo").textContent.replace(" · Calculando la simulación física (motor Rust)…", " · Física del motor Rust (reproductor).");
    return true;
  } catch (e) {
    $("netInfo").textContent = "No se pudo cargar la simulación del motor Rust (trace): " + e;
    return false;
  } finally {
    rodLoad(false);
  }
}
iniciar();

// ══════════════════════════════════════════════════════════════════════════
//  CHROME DE UI (rediseño CTC) — SOLO PRESENTACIÓN.
//  Este bloque no altera la física de la simulación: únicamente gestiona el
//  layout flotante (paneles colapsables, HUD de estación, cajón inferior con
//  pestañas), el sparkline/alerta del KPI y el aislado visual de líneas. Lee el
//  estado `S` ya calculado por el motor; nunca lo modifica.
// ══════════════════════════════════════════════════════════════════════════
(function chromeUI() {
  const el = (id) => document.getElementById(id);

  // ── Paneles laterales colapsables ────────────────────────────────────────
  const leftPanel = el("leftPanel"), incPanel = el("incPanel");
  const openPanel = (p, on) => { p.classList.toggle("open", on); p.setAttribute("aria-hidden", on ? "false" : "true"); };
  const toggle = (p) => openPanel(p, !p.classList.contains("open"));
  el("leftTab").addEventListener("click", () => toggle(leftPanel));
  // Estación, semáforo e incidencias comparten el dock derecho → nunca solapan (cerrarDocks).
  el("incTab").addEventListener("click", () => { const abriendo = !incPanel.classList.contains("open"); if (abriendo) cerrarDocks("inc"); toggle(incPanel); });
  el("leftClose").addEventListener("click", () => openPanel(leftPanel, false));
  el("incClose").addEventListener("click", () => openPanel(incPanel, false));

  // ── Botones de cierre del HUD de estación y del panel de semáforo ────────
  el("stClose").addEventListener("click", () => { el("stationBox").hidden = true; S.selSt = null; });
  el("sigClose").addEventListener("click", () => { el("signalBox").hidden = true; S.selSig = null; });

  // ── Aislado visual de línea desde la leyenda ─────────────────────────────
  // Atenúa los cantones que no son de la línea elegida (las líneas pintadas no
  // las repinta el bucle, así que el efecto persiste) y sincroniza el filtro
  // del cajón de trenes con esa línea, reutilizando el cableado existente.
  let isoLine = null;
  function aplicarAislado() {
    const legend = el("legend");
    legend.classList.toggle("has-iso", !!isoLine);
    legend.querySelectorAll(".row").forEach((r) => r.classList.toggle("iso-active", r.dataset.line === isoLine));
    el("legendReset").hidden = !isoLine;
    for (const { lines, unica, color, casing } of linePolys) {
      const on = !isoLine || lines.includes(isoLine);
      for (const poly of color) poly.setStyle({ opacity: on ? 0.95 : 0.12, weight: on ? (unica ? 2.5 : 3) : 1 });
      for (const cas of casing) cas.setStyle({ opacity: on ? 0.5 : 0.06 });
    }
  }
  el("legend").addEventListener("click", (e) => {
    const row = e.target.closest(".row"); if (!row) return;
    isoLine = (isoLine === row.dataset.line) ? null : row.dataset.line;
    aplicarAislado();
    const f = el("tFiltLine"); f.value = isoLine || ""; f.dispatchEvent(new Event("change"));
  });
  el("legendReset").addEventListener("click", () => {
    isoLine = null; aplicarAislado();
    const f = el("tFiltLine"); f.value = ""; f.dispatchEvent(new Event("change"));
  });

  // ── Cajón inferior: pestañas Trenes / Registro CTC ───────────────────────
  const tabTrains = () => el("drawerTabs").querySelector('[data-tab="trains"]').classList.contains("active");
  el("drawerTabs").addEventListener("click", (e) => {
    const b = e.target.closest(".dtab"); if (!b) return;
    el("drawerTabs").querySelectorAll(".dtab").forEach((x) => x.classList.toggle("active", x === b));
    const trains = b.dataset.tab === "trains";
    el("trenesList").hidden = !trains; el("ctcLog").hidden = trains;
    el("trainFilters").style.visibility = trains ? "visible" : "hidden";
    if (!trains) pintarRegistro();
  });

  // Registro CTC: derivado en vivo del estado (incidencias + retenciones actuales + llegadas tarde).
  const TAG_LABEL = { inc: "INCID", hold: "RETEN", late: "TARDE" };
  function pintarRegistro() {
    const filas = [];
    for (const i of S.incidentes) {
      filas.push({ ts: i.whenSec, tag: "inc", cls: "inc", txt: `${i.nombre} · ${i.label} <span class="muted">[${i.estado}]</span>` });
    }
    for (const t of S.trains) {
      if (!t.esperando) continue;
      const st = S.stationById.get(t.esperaDesde);
      filas.push({ ts: S.simT, tag: "hold", cls: "hold", txt: `<b>${t.line}</b> tren ${t.id} retenido en ${st ? st.name : t.esperaDesde} <span class="muted">(${MOTIVO_LABEL[t.esperaMotivo] || t.esperaMotivo})</span>` });
    }
    // Llegadas tarde a estación ya ocurridas (las que el puntero `arrPtr` ha dejado atrás). Se toman
    // las ~80 más recientes (el array va en orden temporal, así que están justo antes del puntero).
    const arr = S.arrivals, hasta = S.arrPtr;
    for (let k = Math.max(0, hasta - 80); k < hasta; k++) {
      const a = arr[k]; const st = S.stationById.get(a.stop);
      filas.push({ ts: a.t, tag: "late", cls: "late", txt: `<b>${a.line}</b> tren ${a.train} llegó a ${st ? st.name : a.stop} <span class="muted">+${Math.round(a.delay / 60)}′</span>` });
    }
    filas.sort((a, b) => b.ts - a.ts);
    el("ctcLog").innerHTML = filas.length
      ? filas.slice(0, 80).map((f) => `<div class="logline"><span class="ts">${hhmm(f.ts)}</span><span class="tagx ${f.cls}">${TAG_LABEL[f.tag] || f.tag}</span><span>${f.txt}</span></div>`).join("")
      : `<div class="empty">Sin eventos. La red circula según horario.</div>`;
  }

  // ── Sparkline + estado de alerta del KPI de retraso ──────────────────────
  const spark = el("kpiSparkLine"), chip = el("kpiChip"), trend = el("kpiTrend");
  const sparkL = el("kpiSparkLineLive"), chipL = el("kpiChipLive"), trendL = el("kpiTrendLive");
  const hist = [];
  function pintarSpark() {
    hist.push(S.delayNow || 0);   // retraso vivo de la red (min): traza sube/baja, no el total monótono
    if (hist.length > 48) hist.shift();
    if (hist.length < 2) return;
    const min = Math.min(...hist), max = Math.max(...hist), span = (max - min) || 1;
    const pts = hist.map((v, i) => `${(i / (hist.length - 1) * 96).toFixed(1)},${(32 - (v - min) / span * 28).toFixed(1)}`).join(" ");
    spark.setAttribute("points", pts);
    if (sparkL) sparkL.setAttribute("points", pts); // el recuadro VIVO traza el mismo retraso de red
    // Color de alerta por el retraso VIVO de la red (min sumados de los trenes en marcha AHORA):
    // independiente de la velocidad de reproducción. Normal ~40–70 min, punta ~90; incidencia →
    // dispara. La flecha indica si ese retraso vivo sube o baja respecto a la muestra anterior.
    const live = S.delayNow || 0, prev = pintarSpark._prevLive || 0; pintarSpark._prevLive = live;
    const warn = live > 70 && live <= 110, crit = live > 110;
    const arrow = live - prev > 4 ? "▲" : (live - prev < -4 ? "▼" : "");
    chip.classList.toggle("warn", warn);
    chip.classList.toggle("crit", crit);
    trend.textContent = arrow;
    // El recuadro del retraso VIVO comparte alerta, tendencia y número (por si la cadencia de
    // `actualizarKpi` no ha refrescado aún: la alerta es del retraso de red, su hogar natural).
    if (chipL) { chipL.classList.toggle("warn", warn); chipL.classList.toggle("crit", crit); }
    if (trendL) trendL.textContent = arrow;
    if (el("kLive")) el("kLive").textContent = Math.round(live).toLocaleString((window.LOCALE?LOCALE():"es-ES"));
  }

  // ── Contador del asa del cajón (trenes retenidos ahora) ──────────────────
  function refrescarAsa() {
    let hold = 0; for (const t of S.trains) if (t.esperando) hold++;
    const h = el("handleCount"); h.textContent = hold;
    h.style.background = hold ? "var(--rojo)" : "rgba(255,255,255,.14)";
    h.style.color = hold ? "#fff" : "var(--muted)";
  }

  // Tic presentacional propio (independiente del bucle de simulación).
  setInterval(() => {
    pintarSpark();
    refrescarAsa();
    aplicarAislado();
    if (!el("trenesPanel").hidden && !tabTrains()) pintarRegistro();
  }, 620);

  // Abrir el panel de red por defecto para orientar al usuario en el primer arranque.
  setTimeout(() => openPanel(leftPanel, true), 400);
})();

// ═══════════════ Selección de incidencias de la jornada + "lanzar incidencia ahora" ═══════════════
// Módulo AISLADO del motor del juego: solo habla con la API (/api/incidents/templates y
// /api/game/trace) y con su propio DOM. Las plantillas (compartidas con /numeros) se crean y
// editan en el apartado /incidencias del menú; aquí solo se ELIGEN para la jornada visual.
(function incidentTemplates() {
  const $ = (id) => document.getElementById(id);
  const EFF = { red_one: "Señal roja (1 sentido)", red_both: "Señal roja (2 sentidos)", single: "Tramo → vía única" };
  let tpls = [];

  // SELECCIÓN por modo: las plantillas son COMPARTIDAS (servidor; se crean/editan en el apartado
  // /incidencias del menú), pero qué plantillas entran en la jornada visual se guarda aparte
  // (localStorage "rod-sel-visual", independiente de /numeros) por ID ESTABLE de plantilla: editar
  // una plantilla no la desmarca y borrar/añadir desde otro sitio no desplaza la selección.
  const SELKEY = "rod-sel-visual";
  let sel = null;   // Set de ids seleccionados
  const rateTxt = (t) => `${Math.round(t.prob * 100)} % cada ${(Math.round(t.every_h * 100) / 100).toLocaleString((window.LOCALE?LOCALE():"es-ES"))} h`;
  function loadSel() {
    try {
      const v = JSON.parse(localStorage.getItem(SELKEY));
      if (Array.isArray(v)) {
        // Migración: versiones previas guardaban la firma "motivo|efecto|dur|prob" en vez del id.
        const old = new Set(v.filter((x) => typeof x === "string" && x.includes("|")));
        sel = new Set(v.filter((x) => typeof x === "string" && !x.includes("|")));
        for (const t of tpls) if (old.has([t.motivo, t.effect, t.dur_min, t.prob].join("|"))) sel.add(t.id);
      }
    } catch (e) { /* sin storage */ }
    // Primera vez: se parte de las marcadas como activas en el servidor.
    if (!sel) sel = new Set(tpls.filter((t) => t.enabled).map((t) => t.id));
  }
  function saveSel() { try { localStorage.setItem(SELKEY, JSON.stringify([...sel])); } catch (e) { /* sin storage */ } }
  // Parámetro de query para el motor con la selección actual (vacía → "&sel=" = ninguna).
  S.tplSelQuery = () => "&sel=" + tpls.filter((t) => sel && sel.has(t.id)).map((t) => encodeURIComponent(t.id)).join(",");

  async function load() {
    try { tpls = await (await fetch("/api/incidents/templates")).json(); } catch (e) { tpls = []; }
    loadSel();
    aplicarParamsDeRuns();
    render();
  }
  // "Ver en simulador" desde /numeros: `?seed=…&sel=id1,id2&from=numeros&gtfs=AAAA-MM-DD`. La
  // selección de la URL manda SOLO en esta visita (no se guarda salvo que el usuario la toque), así
  // la selección propia del modo visual no se pisa. Misma semilla + mismas plantillas + mismo GTFS
  // ⇒ el motor reproduce exactamente la misma jornada (el roll de incidencias es determinista).
  function aplicarParamsDeRuns() {
    const q = new URLSearchParams(location.search);
    if (q.get("from") !== "numeros" || !q.has("sel")) return;
    const want = (q.get("sel") || "").split(",").filter(Boolean);
    const known = new Set(tpls.map((t) => t.id));
    sel = new Set(want.filter((id) => known.has(id)));
    const lost = want.length - sel.size;
    const box = $("preFrom"); if (!box) return;
    box.innerHTML = `Parámetros de <b>Varias runs</b>: semilla <b>${+q.get("seed") || 0}</b> · <b>${sel.size}</b> plantilla${sel.size === 1 ? "" : "s"} de incidencia. Pulsa <b>Confirmar y generar el día</b> para ver exactamente esa jornada.` +
      (+q.get("vvar") > 0 ? ` Variabilidad de velocidad: <b>${+q.get("vvar")} %</b>.` : "") +
      (lost ? `<span class="warn">${lost} plantilla${lost === 1 ? " ya no existe" : "s ya no existen"} (se borraron después de la run): el día puede diferir.</span>` : "");
    box.hidden = false;
    const gt = q.get("gtfs");
    if (gt && /^\d{4}-\d\d-\d\d$/.test(gt)) fetch("/api/gtfs/info").then((r) => r.json()).then((j) => {
      if (j.date && j.date !== gt) box.insertAdjacentHTML("beforeend", `<span class="warn">Los horarios cargados ahora (${j.date}) no son los de la run (${gt}): el día puede diferir.</span>`);
    }).catch(() => {});
  }
  function render() {
    const ul = $("tplList"); if (!ul) return;
    const all = $("tplAll"), nOn = tpls.filter((t) => sel.has(t.id)).length;
    if (all) { all.checked = tpls.length > 0 && nOn === tpls.length; all.indeterminate = nOn > 0 && nOn < tpls.length; all.disabled = !tpls.length; }
    if (!tpls.length) { ul.innerHTML = '<li class="empty">Sin plantillas. Créalas en el apartado Incidencias del menú.</li>'; return; }
    ul.innerHTML = "";
    tpls.forEach((t, i) => {
      const on = sel.has(t.id);
      const li = document.createElement("li");
      if (!on) li.className = "off";
      li.innerHTML =
        `<label class="chk"><input type="checkbox" ${on ? "checked" : ""} data-i="${i}" class="tplTgl" aria-label="Incluir en esta jornada"></label>` +
        `<span class="tpl-main"><b>${t.motivo || "(sin motivo)"}</b>` +
        `<small>${EFF[t.effect] || t.effect} · ${t.dur_min} min · ${rateTxt(t)}</small></span>`;
      ul.appendChild(li);
    });
    ul.querySelectorAll(".tplTgl").forEach((c) => (c.onchange = () => {
      const id = tpls[+c.dataset.i].id;
      if (c.checked) sel.add(id); else sel.delete(id);
      saveSel(); render();
    }));
  }
  // Casilla maestra: todas / ninguna de golpe.
  const allBox = $("tplAll");
  if (allBox) allBox.onchange = () => { sel = allBox.checked ? new Set(tpls.map((t) => t.id)) : new Set(); saveSel(); render(); };

  // ── "Lanzar incidencia AHORA": elige plantilla + tramo → re-simula la jornada con la
  // incidencia inyectada desde la hora actual (así el mapa visual la refleja de verdad). ──
  const tramoMap = new Map();   // texto visible "Nombre A → Nombre B" -> "from>to"
  function poblarLive() {
    const selTpl = $("liveTpl"), dl = $("tramoList");
    if (selTpl) selTpl.innerHTML = tpls.map((t, i) => `<option value="${i}">${t.motivo} · ${EFF[t.effect] || t.effect}</option>`).join("")
      || '<option value="">(crea plantillas arriba)</option>';
    // Buscador de TRAMOS: datalist con el nombre COMPLETO (se busca escribiéndolo). El value del
    // <input> es el texto visible; se resuelve a "from>to" con tramoMap al lanzar.
    const nEdges = (S.net && S.net.edges || []).length;
    if (dl && nEdges && dl.dataset.n !== String(nEdges)) {
      const eds = S.net.edges.slice().sort((a, b) => nombreCorto(a.from).localeCompare(nombreCorto(b.from)));
      tramoMap.clear();
      dl.innerHTML = eds.map((e) => {
        const txt = `${nombreCorto(e.from)} → ${nombreCorto(e.to)}`;
        tramoMap.set(txt, e.from + ">" + e.to);
        return `<option value="${txt}"></option>`;
      }).join("");
      dl.dataset.n = String(nEdges);
    }
  }
  const launchBtn = $("liveLaunch");
  if (launchBtn) launchBtn.onclick = async () => {
    const ti = +$("liveTpl").value, tpl = tpls[ti];
    const txt = $("liveTramo").value.trim(), st = $("liveStatus");
    // El input acepta el texto visible del tramo; se resuelve a "from>to" con el mapa del datalist.
    const tramo = tramoMap.get(txt);
    if (!tpl || !tramo) { st.textContent = "Escribe/elige un tramo válido de la lista (nombre exacto)."; return; }
    const [from, to] = tramo.split(">");
    const t0 = Math.floor(S.simT);
    st.textContent = "Recalculando la jornada con la incidencia…";
    launchBtn.disabled = true;
    try {
      // Misma semilla y MISMA selección de plantillas que la jornada en curso → el día re-simulado
      // es idéntico hasta 'ahora' (mismas incidencias rodadas) + la incidencia lanzada.
      const url = `/api/game/trace?seed=${S.traceSeed || 0}${S.traceSel || ""}&eff=${tpl.effect}&from=${encodeURIComponent(from)}&to=${encodeURIComponent(to)}&t0=${t0}&dur=${tpl.dur_min}&motivo=${encodeURIComponent(tpl.motivo)}`;
      const tr = await (await fetch(url)).json();
      if (!tr || !tr.trains) throw new Error("trace vacío");
      tr.byId = new Map();
      for (const t of tr.trains) tr.byId.set(t.id, t);
      S.trace = tr; // se conserva S.simT: el trace hasta 'ahora' es idéntico (misma semilla)
      construirTimelineRetraso(tr);   // el gráfico de retraso incorpora el efecto de la incidencia
      // Reinicia el contador de retraso: el bucle re-suma las llegadas hasta 'ahora' (idénticas) y
      // sigue con las nuevas que provoque la incidencia inyectada.
      S.arrivals = tr.arrivals || []; S.arrPtr = 0; S.kpi = 0;
      // El trace re-generado ya incluye TODAS las incidencias del día (incluida la lanzada).
      S.dayIncidents = (tr.incidents || []).map((i) => ({ ...i, launched: i.t0 >= t0 - 1 }));
      pintarIncidenciasDia();
      st.textContent = `Incidencia "${tpl.motivo}" activa desde ${hhmm(t0)} en ${nombreCorto(from)} → ${nombreCorto(to)}.`;
    } catch (e) {
      st.textContent = "Error al recalcular la jornada.";
    } finally {
      launchBtn.disabled = false;
    }
  };

  // Rellena los selectores al cargar y reintenta hasta que la red esté disponible.
  let tries = 0;
  const _poll = setInterval(() => { poblarLive(); if ((S.net && S.net.edges && S.net.edges.length) || ++tries > 40) clearInterval(_poll); }, 400);

  load().then(poblarLive);
})();
