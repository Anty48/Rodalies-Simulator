#!/usr/bin/env python3
"""Longitud y trazado REALES de vía por cantón, encaminando sobre la red ferroviaria de OSM.

El motor medía cada cantón (par de paradas consecutivas del GTFS) en LÍNEA RECTA entre estaciones.
Donde la vía da un rodeo —p. ej. Estació de França → Passeig de Gràcia, que sale hacia el Clot y da
media vuelta por el túnel de Aragó— la recta se queda en una fracción del recorrido real: el tren
"llega" en 2 min a un tramo que el horario (y la realidad) recorre en 10.

Este script:
  1. Descarga de Overpass las vías `railway=rail` del área de Rodalies (sin playas de vías,
     apartaderos industriales ni ramales turísticos/militares), con ids de nodo → grafo. La alta
     velocidad se excluye SALVO la de ancho ibérico (1668 mm): la variante de Vandellòs (corredor
     litoral Tarragona–Ulldecona) está etiquetada `highspeed=yes` y por ella circula la R16.
  2. Lee los cantones del GTFS filtrado (`data/gtfs`): pares únicos de paradas consecutivas.
  3. Encamina cada cantón con A* sobre el grafo con RESTRICCIÓN DE GIRO: en un nodo el tren no puede
     desviarse más de `MAX_TURN_DEG` (una aguja diverge pocos grados; >60° es una marcha atrás o un
     cruce en ángulo, imposibles sin invertir la marcha).
  4. Recorta cada trazado al punto de VÍA más cercano a cada estación (sin el enganche recto desde
     la coordenada GTFS, que suele caer en el edificio y dibujaba un gancho junto a la estación).
  5. Detecta BIFURCACIONES fuera de estación: desde cada estación, dos cantones que van por la
     MISMA vía (≤ `SHARE_TOL_M`) más de `MIN_SHARED_M` y luego se separan comparten un tramo común
     hasta el punto de divergencia (El Prat → Aeroport / Viladecans, França → Passeig de Gràcia por
     la Bif. Aragó, Arc de Triomf → Clot por Glòries…). Dos líneas en vías PARALELAS distintas
     (4 vías, p. ej. Clot → Sant Adrià) quedan a más de `SHARE_TOL_M` y no se juntan.
  6. Escribe `processed/adif/track_geometry.json`:
     {"source": ..., "junctions": {"J1": {"lat", "lon", "near"}},
      "cantons": {"<from_id>><to_id>": {"len_m": m, "coords": [[lat,lon],...], "via": [["J1", m]]}}}
     (`via` = bifurcaciones por las que pasa el cantón y a qué distancia de su origen).

Si un cantón no se puede encaminar (estación lejos de la vía, hueco en OSM…) se omite y el motor
usa la línea recta para él (el informe final lista cuántos y cuáles).

Uso:  python scripts/fetch_osm_track.py [--refresh]
      (--refresh vuelve a descargar de Overpass; si no, reutiliza raw/osm_rail.json)
"""
import csv
import heapq
import json
import math
import os
import sys
import time
import urllib.request

# Instancias públicas de Overpass (se prueban en orden: la principal suele dar 504 si va saturada).
OVERPASS = [
    "https://overpass-api.de/api/interpreter",
    "https://overpass.kumi.systems/api/interpreter",
    "https://overpass.private.coffee/api/interpreter",
]
RAW = os.path.join("raw", "osm_rail.json")
OUT = os.path.join("processed", "adif", "track_geometry.json")
GTFS = os.path.join("data", "gtfs")

# Giro máximo permitido en un nodo (grados). Las agujas divergen ~5-15°; la geometría OSM de curvas
# cerradas no pasa de ~30° entre nodos consecutivos. Un giro mayor = invertir la marcha.
MAX_TURN_DEG = 60.0
# Radio de búsqueda de la vía alrededor de la coordenada GTFS de la estación (m).
SNAP_M = 350.0
# Nº de nodos candidatos por estación: TODAS las vías de andén de una estación grande (Sants tiene
# 14), para que el A* pueda salir por las de SU línea aunque la coordenada GTFS caiga en otras.
SNAP_K = 60
# Peso del enganche estación↔vía en el coste: con 1 daría igual engancharse a 340 m por la vía que a
# 10 m + 330 m de vía, y el trazado podría empezar/acabar lejos de la estación (y quedarse corto al
# recortarlo). Con 3 se engancha a la vía más cercana salvo que la de SU línea compense.
SNAP_COST = 3.0
# Un camino más largo que esto × recta (+2 km) se descarta: seguramente cruza por donde no toca.
MAX_DETOUR = 4.0
# Bifurcaciones: dos trazados comparten vía si distan ≤ SHARE_TOL_M (vía doble: ejes a ~4 m; dos
# líneas en vías paralelas distintas quedan más lejos) durante ≥ MIN_SHARED_M, y a cada ramal le
# quedan ≥ MIN_BRANCH_M tras separarse. Los primeros GRACE_M son la garganta de la propia estación
# (pueden salir de andenes distintos). Puntos de divergencia a ≤ MERGE_M son la misma bifurcación.
SHARE_TOL_M = 6.0
MIN_SHARED_M = 300.0
MIN_BRANCH_M = 200.0
GRACE_M = 150.0
MERGE_M = 80.0
MIN_CONFIRMED_M = 150.0


def haversine(a, b):
    R = 6371000.0
    la1, lo1, la2, lo2 = map(math.radians, (a[0], a[1], b[0], b[1]))
    h = math.sin((la2 - la1) / 2) ** 2 + math.cos(la1) * math.cos(la2) * math.sin((lo2 - lo1) / 2) ** 2
    return 2 * R * math.asin(min(1.0, math.sqrt(h)))


def bearing_vec(a, b):
    """Vector (este, norte) aproximado en metros de a→b (plano local; basta para ángulos)."""
    k = math.cos(math.radians((a[0] + b[0]) / 2))
    return ((b[1] - a[1]) * k, b[0] - a[0])


def turn_ok(p, n, q):
    """¿Se puede pasar p→n→q sin girar más de MAX_TURN_DEG?"""
    u, v = bearing_vec(p, n), bearing_vec(n, q)
    nu, nv = math.hypot(*u), math.hypot(*v)
    if nu < 1e-12 or nv < 1e-12:
        return True
    c = (u[0] * v[0] + u[1] * v[1]) / (nu * nv)
    return c >= math.cos(math.radians(MAX_TURN_DEG))


# Vía sin trenes de viajeros en el GTFS vigente (autobús por obras) que usan los MERCANCÍAS: se traza
# igual que un cantón. COPIA de `FREIGHT_ONLY_TRACK` en src/topology.rs (si cambias una, cambia la otra).
FREIGHT_ONLY_TRACK = [
    # La Plana-Picamoixons – Les Borges Blanques por Montblanc (Lleida → Valls → Sant Vicenç).
    ["73100", "73010", "73009", "73008", "73007", "73006", "73005", "73004", "73003"],
]


def load_stops_and_cantons():
    stops = {}
    with open(os.path.join(GTFS, "stops.txt"), encoding="utf-8-sig") as f:
        for r in csv.DictReader(f):
            stops[r["stop_id"].strip()] = (float(r["stop_lat"]), float(r["stop_lon"]))
    bus_routes = set()
    with open(os.path.join(GTFS, "routes.txt"), encoding="utf-8-sig") as f:
        for r in csv.DictReader(f):
            if (r.get("route_type") or "").strip() == "3":
                bus_routes.add(r["route_id"].strip())
    short = {}
    with open(os.path.join(GTFS, "routes.txt"), encoding="utf-8-sig") as f:
        for r in csv.DictReader(f):
            short[r["route_id"].strip()] = (r.get("route_short_name") or "").strip()
    bus_trips, trip_line = set(), {}
    with open(os.path.join(GTFS, "trips.txt"), encoding="utf-8-sig") as f:
        for r in csv.DictReader(f):
            if r["route_id"].strip() in bus_routes:
                bus_trips.add(r["trip_id"].strip())
            trip_line[r["trip_id"].strip()] = short.get(r["route_id"].strip(), "")
    seqs = {}
    with open(os.path.join(GTFS, "stop_times.txt"), encoding="utf-8-sig") as f:
        for r in csv.DictReader(f):
            t = r["trip_id"].strip()
            if t in bus_trips:
                continue
            seqs.setdefault(t, []).append((int(r["stop_sequence"]), r["stop_id"].strip()))
    pairs, lines = set(), {}
    for t, v in seqs.items():
        v.sort()
        for (_, a), (_, b) in zip(v, v[1:]):
            if a != b and a in stops and b in stops:
                pairs.add((a, b))
                lines.setdefault((a, b), set()).add(trip_line.get(t, ""))
    for chain in FREIGHT_ONLY_TRACK:
        for a, b in zip(chain, chain[1:]):
            if a in stops and b in stops:
                pairs.add((a, b))
                pairs.add((b, a))
    return stops, sorted(pairs), lines


def fetch_osm(stops, refresh):
    if os.path.exists(RAW) and not refresh:
        print(f"-> Reutilizando {RAW} (usa --refresh para volver a descargar)")
        with open(RAW, encoding="utf-8") as f:
            return json.load(f)
    la = [p[0] for p in stops.values()]
    lo = [p[1] for p in stops.values()]
    bbox = f"{min(la) - 0.05},{min(lo) - 0.05},{max(la) + 0.05},{max(lo) + 0.05}"
    q = f"""
[out:json][timeout:600][bbox:{bbox}];
(
  way["railway"="rail"]["highspeed"!="yes"]["service"!~"^(yard|spur)$"]["usage"!~"^(industrial|military|tourism)$"];
  way["railway"="rail"]["highspeed"="yes"]["gauge"~"1668"];
);
out body geom qt;
"""
    print(f"-> Consultando Overpass (railway=rail, bbox {bbox})… puede tardar un par de minutos")
    t0, data, err = time.time(), None, None
    for attempt in range(2):
        for url in OVERPASS:
            try:
                req = urllib.request.Request(url, data=("data=" + q).encode("utf-8"),
                                             headers={"User-Agent": "rodalies-sim/1.0 (track geometry ingest)"})
                with urllib.request.urlopen(req, timeout=660) as r:
                    data = json.load(r)
                break
            except Exception as e:  # 429/504 = saturado → siguiente instancia
                err = e
                print(f"   {url}: {e} → probando otra instancia")
        if data is not None:
            break
        time.sleep(20)
    if data is None:
        raise SystemExit(f"ERROR Overpass no responde ({err}); reintenta más tarde.")
    os.makedirs(os.path.dirname(RAW), exist_ok=True)
    with open(RAW, "w", encoding="utf-8") as f:
        json.dump(data, f)
    print(f"   {len(data.get('elements', []))} vías en {time.time() - t0:.0f} s → {RAW}")
    return data


REFS_OUT = []  # (nodos de la vía, ref) — refs de línea para la preferencia del encaminamiento


def build_graph(data):
    """Grafo de vías + identidad de línea de cada tramo (`ref` de OSM, p. ej. "R1;R3;R4;RG1", o
    `name`): distingue la vía de una línea de la vía CONTIGUA de otra (4 vías, túneles paralelos)."""
    pos, adj = {}, {}
    ident = {}
    for w in data.get("elements", []):
        if w.get("type") != "way":
            continue
        ids, geom = w.get("nodes") or [], w.get("geometry") or []
        if len(ids) != len(geom) or len(ids) < 2:
            continue
        for nid, g in zip(ids, geom):
            pos[nid] = (g["lat"], g["lon"])
        tags = w.get("tags") or {}
        who = tags.get("ref") or tags.get("name")
        if tags.get("ref"):
            REFS_OUT.append((ids, tags["ref"]))
        for a, b in zip(ids, ids[1:]):
            if a == b:
                continue
            d = haversine(pos[a], pos[b])
            adj.setdefault(a, {})[b] = d
            adj.setdefault(b, {})[a] = d
            if who:
                ident[(min(a, b), max(a, b))] = who
    return pos, adj, ident


class Grid:
    """Rejilla de ~250 m para buscar nodos cercanos a una estación sin recorrer todo el grafo."""

    def __init__(self, pos, cell=0.0025):
        self.cell, self.pos, self.g = cell, pos, {}
        for nid, (la, lo) in pos.items():
            self.g.setdefault((int(la / cell), int(lo / cell)), []).append(nid)

    def near(self, p, radius_m, k):
        r = int(radius_m / (self.cell * 111000)) + 1
        ci, cj = int(p[0] / self.cell), int(p[1] / self.cell)
        out = []
        for i in range(ci - r, ci + r + 1):
            for j in range(cj - r, cj + r + 1):
                for nid in self.g.get((i, j), ()):
                    d = haversine(p, self.pos[nid])
                    if d <= radius_m:
                        out.append((d, nid))
        out.sort()
        return out[:k]


# Coste relativo de circular por una vía etiquetada en OSM para OTRAS líneas (`ref` sin ninguna de
# las del cantón): en tramos de 4 vías o túneles paralelos (Sants → L'Hospitalet por las vías de la
# R1/R3/R4, no por las de la R2) el tren va por las SUYAS aunque las de al lado sean algo más cortas.
FOREIGN_TRACK_COST = 1.5


def norm_ref(ref):
    """`ref` de OSM → conjunto de líneas con los nombres del GTFS ("R2Nord" → "R2N")."""
    out = set()
    for tok in (ref or "").replace(",", ";").split(";"):
        tok = tok.strip()
        if not tok:
            continue
        out.add(tok.replace("Nord", "N").replace("Sud", "S"))
    return out


def route(pos, adj, grid, a, b, lines=None, ident=None):
    """A* con restricción de giro. Estado = (nodo, nodo previo). Devuelve [nodos] o None. Con
    `lines`, las vías etiquetadas para otras líneas cuestan FOREIGN_TRACK_COST× (preferencia)."""
    src = grid.near(a, SNAP_M, SNAP_K)
    dst = {nid: d for d, nid in grid.near(b, SNAP_M, SNAP_K)}
    if not src or not dst:
        return None
    straight = haversine(a, b)
    limit = MAX_DETOUR * straight + 2000.0
    # Heurística admisible: lo que falta hasta el destino menos el mayor enganche posible.
    h = lambda n: max(0.0, haversine(pos[n], b) - SNAP_M)
    pq, best, parent = [], {}, {}
    GOAL = ("__goal__", None)
    for d, nid in src:
        st = (nid, None)
        best[st] = SNAP_COST * d  # el enganche estación→vía cuenta (y se penaliza: ver SNAP_COST)
        parent[st] = None
        heapq.heappush(pq, (best[st] + h(nid), best[st], nid, None))
    while pq:
        f, g, n, p = heapq.heappop(pq)
        if (n, p) == GOAL:
            # Destino alcanzado por el mejor enganche vía→estación (no el primer nodo del radio).
            path, st = [], parent[GOAL]
            while st is not None:
                path.append(st[0])
                st = parent[st]
            return path[::-1]
        if g > best.get((n, p), math.inf) or g > limit:
            continue
        if n in dst:
            # Candidato a llegada: el coste total incluye el enganche vía→estación. Se encola un
            # objetivo virtual; el A* sigue por si otro nodo del andén da un total menor.
            gg = g + SNAP_COST * dst[n]
            if gg < best.get(GOAL, math.inf):
                best[GOAL] = gg
                parent[GOAL] = (n, p)
                heapq.heappush(pq, (gg, gg, GOAL[0], None))
        for q, w in adj.get(n, {}).items():
            if q == p or (p is not None and not turn_ok(pos[p], pos[n], pos[q])):
                continue
            if lines and ident is not None:
                ref = ident.get((min(n, q), max(n, q)))
                if ref is not None:
                    toks = norm_ref(ref)
                    if toks and not (toks & lines):
                        w *= FOREIGN_TRACK_COST
            ng = g + w
            st = (q, n)
            if ng < best.get(st, math.inf):
                best[st] = ng
                parent[st] = (n, p)
                heapq.heappush(pq, (ng + h(q), ng, q, n))
    return None


def simplify(pts, tol_m=2.0):
    """Douglas-Peucker en metros (plano local): conserva la forma de la vía con muchos menos puntos."""
    if len(pts) < 3:
        return pts
    k = math.cos(math.radians(pts[0][0]))
    xy = [((p[1]) * 111320 * k, p[0] * 110540) for p in pts]
    keep = [False] * len(pts)
    keep[0] = keep[-1] = True
    stack = [(0, len(pts) - 1)]
    while stack:
        i, j = stack.pop()
        (x1, y1), (x2, y2) = xy[i], xy[j]
        dx, dy = x2 - x1, y2 - y1
        L = math.hypot(dx, dy) or 1e-9
        dmax, idx = 0.0, -1
        for m in range(i + 1, j):
            d = abs(dy * (xy[m][0] - x1) - dx * (xy[m][1] - y1)) / L
            if d > dmax:
                dmax, idx = d, m
        if dmax > tol_m:
            keep[idx] = True
            stack += [(i, idx), (idx, j)]
    return [p for p, k2 in zip(pts, keep) if k2]


def trim_to_stations(path_pts, a, b):
    """Recorta la polilínea al punto de VÍA más cercano a cada estación (proyección sobre el
    segmento más cercano dentro del primer/último tramo): sin gancho estación→vía."""
    def proj(p, q, x):
        k = math.cos(math.radians(x[0]))
        px, py = (p[1] - x[1]) * k, p[0] - x[0]
        qx, qy = (q[1] - x[1]) * k, q[0] - x[0]
        dx, dy = qx - px, qy - py
        L = dx * dx + dy * dy
        t = 0.0 if L == 0 else max(0.0, min(1.0, -(px * dx + py * dy) / L))
        pt = (p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t)
        return haversine(pt, x), pt, t
    n = len(path_pts)
    if n < 2:
        return path_pts, (0, max(0, n - 2))
    lim = max(2, n // 3)
    best = min(((proj(path_pts[i], path_pts[i + 1], a), i) for i in range(0, min(lim, n - 1))), key=lambda z: z[0][0])
    (_, pa, _), ia = best
    best = min(((proj(path_pts[i], path_pts[i + 1], b), i) for i in range(max(0, n - 1 - lim), n - 1)), key=lambda z: z[0][0])
    (_, pb, _), ib = best
    if ib < ia:
        return path_pts, (ia, ib)
    return [pa] + path_pts[ia + 1: ib + 1] + [pb], (ia, ib)


def to_xy(pts, ref):
    k = math.cos(math.radians(ref[0]))
    return [((q[1] - ref[1]) * 111320 * k, (q[0] - ref[0]) * 110540) for q in pts]


def resample(xy, step=10.0):
    """Puntos cada `step` m a lo largo de la polilínea (en metros locales): (punto, distancia,
    índice de segmento) + longitud total."""
    out, acc = [(xy[0], 0.0, 0)], 0.0
    nxt = step
    for k, ((x1, y1), (x2, y2)) in enumerate(zip(xy, xy[1:])):
        L = math.hypot(x2 - x1, y2 - y1)
        while L > 0 and acc + L >= nxt:
            t = (nxt - acc) / L
            out.append(((x1 + (x2 - x1) * t, y1 + (y2 - y1) * t), nxt, k))
            nxt += step
        acc += L
    return out, acc


def dist_to_poly(p, xy):
    """Distancia del punto a la polilínea y segmento más cercano."""
    best, seg = math.inf, 0
    for k, ((x1, y1), (x2, y2)) in enumerate(zip(xy, xy[1:])):
        dx, dy = x2 - x1, y2 - y1
        L = dx * dx + dy * dy
        t = 0.0 if L == 0 else max(0.0, min(1.0, ((p[0] - x1) * dx + (p[1] - y1) * dy) / L))
        d = math.hypot(x1 + t * dx - p[0], y1 + t * dy - p[1])
        if d < best:
            best, seg = d, k
    return best, seg


def same_track(id_i, id_j):
    """Misma vía si las dos llevan la misma identidad de línea OSM (o alguna no la lleva: agujas,
    escapes). Dos vías contiguas de líneas distintas tienen `ref` distintos."""
    return id_i is None or id_j is None or id_i == id_j


def shared_prefix(si, ids_i, xj, ids_j):
    """Distancia (m) hasta la que el trazado remuestreado `si` va por la MISMA vía que `xj`: primer
    punto (pasada la garganta) a partir del cual, durante 3 muestras, se aleja > SHARE_TOL_M o está
    sobre una vía de OTRA línea."""
    far, confirmed = 0, 0.0
    step = si[1][1] - si[0][1] if len(si) > 1 else 10.0
    for k, (pt, d, seg) in enumerate(si):
        if d < GRACE_M:
            continue
        dd, sj = dist_to_poly(pt, xj)
        a_id, b_id = ids_i[min(seg, len(ids_i) - 1)], ids_j[min(sj, len(ids_j) - 1)]
        if dd > SHARE_TOL_M or not same_track(a_id, b_id):
            far += 1
            if far >= 3:
                return si[k - 2][1], confirmed
        else:
            far = 0
            if a_id is not None and a_id == b_id:
                confirmed += step
    return si[-1][1], confirmed


def atomic_pairs(out):
    """Cantones ATÓMICOS (estación a estación sin pasar por otra): los exprés S→X que recorren
    S→Y→X los descompone el motor, así que no cuentan para buscar bifurcaciones."""
    L = {tuple(k.split(">")): v["len_m"] for k, v in out.items()}
    nb = {}
    for a, b in L:
        nb.setdefault(a, set()).add(b)
    atomic = set()
    for (a, x), lx in L.items():
        # ¿Hay un camino a→…→x por OTRAS estaciones de longitud ≈ la del cantón? (Dijkstra acotado)
        best = {a: 0.0}
        pq = [(0.0, a)]
        express = False
        while pq and not express:
            d, u = heapq.heappop(pq)
            if d > best.get(u, math.inf) or d > 1.15 * lx:
                continue
            for v in nb.get(u, ()):
                if u == a and v == x:
                    continue  # el propio cantón no cuenta
                nd = d + L[(u, v)]
                if v == x:
                    if nd <= 1.15 * lx:
                        express = True
                    continue
                if nd < best.get(v, math.inf) and nd <= 1.15 * lx:
                    best[v] = nd
                    heapq.heappush(pq, (nd, v))
        if not express:
            atomic.add((a, x))
    return atomic


def raw_of(raw, a, b):
    """Polilínea sin simplificar + identidades del cantón a→b (invirtiendo si solo existe b→a)."""
    if (a, b) in raw:
        return raw[(a, b)]
    pts, ids = raw[(b, a)]
    return pts[::-1], ids[::-1]


def find_junctions(stops, out, raw):
    """Bifurcaciones fuera de estación → (junctions, via por cantón)."""
    atomic = atomic_pairs(out)
    by_station = {}
    for key in out:
        a, b = key.split(">")
        if (a, b) in atomic:
            by_station.setdefault(a, []).append(b)
    cands = []  # (lat, lon, estación, cantón, distancia)
    for st, nbrs in by_station.items():
        if len(nbrs) < 2:
            continue
        ref = stops[st]
        geo = {}
        for b in nbrs:
            pts, ids = raw_of(raw, st, b)
            xy = to_xy(pts, ref)
            geo[b] = (xy, ids, *resample(xy))
        for i in nbrs:
            for j in nbrs:
                if i == j:
                    continue
                xi, idi, si, Li = geo[i]
                xj, idj, sj, Lj = geo[j]
                d, conf = shared_prefix(si, idi, xj, idj)
                d_rev, _ = shared_prefix(sj, idj, xi, idi)
                # Tramo común con ≥ MIN_CONFIRMED_M sobre vía de la MISMA línea etiquetada en las dos
                # (las agujas/escapes sin etiqueta de una garganta no bastan para unir dos túneles).
                if min(d, d_rev) < MIN_SHARED_M or conf < MIN_CONFIRMED_M or Li - d < MIN_BRANCH_M or Lj - d_rev < MIN_BRANCH_M:
                    continue
                # Punto de divergencia sobre el trazado i (en lat/lon).
                k = min(range(len(si)), key=lambda q: abs(si[q][1] - d))
                x, y = si[k][0]
                kk = math.cos(math.radians(ref[0]))
                lat, lon = ref[0] + y / 110540, ref[1] + x / (111320 * kk)
                cands.append((lat, lon, st, f"{st}>{i}", d))
    # Agrupar divergencias cercanas en una misma bifurcación.
    junctions, cid = [], []
    for lat, lon, st, key, d in cands:
        for n, (jl, jo, near, _) in enumerate(junctions):
            if haversine((lat, lon), (jl, jo)) <= MERGE_M:
                cid.append(n)
                break
        else:
            junctions.append((lat, lon, st, key))
            cid.append(len(junctions) - 1)
    via = {}
    for (lat, lon, st, key, d), n in zip(cands, cid):
        a, b = key.split(">")
        L = out[key]["len_m"]
        via.setdefault(key, {})[n] = d
        via.setdefault(f"{b}>{a}", {})[n] = L - d
    jid = {n: f"J{n + 1}" for n in range(len(junctions))}
    res_j = {jid[n]: {"lat": round(la, 6), "lon": round(lo, 6), "near": st} for n, (la, lo, st, _) in enumerate(junctions)}
    res_via = {k: sorted(([jid[n], round(dd, 1)] for n, dd in v.items()), key=lambda z: z[1]) for k, v in via.items()}
    return res_j, res_via


def main():
    try:
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass
    stops, pairs, pair_lines = load_stops_and_cantons()
    print(f"-> {len(pairs)} cantones dirigidos en el GTFS")
    data = fetch_osm(stops, "--refresh" in sys.argv)
    pos, adj, ident = build_graph(data)
    ident_ref = {}
    for ids, ref in REFS_OUT:
        for p_, q_ in zip(ids, ids[1:]):
            ident_ref[(min(p_, q_), max(p_, q_))] = ref
    print(f"-> Grafo OSM: {len(pos)} nodos, {sum(len(v) for v in adj.values()) // 2} tramos")
    grid = Grid(pos)
    out, failed, ratios = {}, [], []
    done = {}
    raw = {}  # (a, b) → (polilínea recortada SIN simplificar, identidad de línea por segmento)
    t0 = time.time()
    for k, (a, b) in enumerate(pairs):
        # El sentido contrario comparte vía: se reutiliza invertido si ya salió.
        if (b, a) in done:
            rev = done[(b, a)]
            out[f"{a}>{b}"] = {"len_m": rev["len_m"], "coords": rev["coords"][::-1]}
            continue
        path = route(pos, adj, grid, stops[a], stops[b], pair_lines.get((a, b), set()) | pair_lines.get((b, a), set()), ident_ref)
        if not path:
            failed.append((a, b))
            continue
        coords, (ia, ib) = trim_to_stations([pos[n] for n in path], stops[a], stops[b])
        seg_ids = [ident.get((min(p, q), max(p, q))) for p, q in zip(path, path[1:])]
        raw[(a, b)] = (coords, seg_ids[ia: ib + 1] if ib >= ia else seg_ids)
        length = sum(haversine(p, q) for p, q in zip(coords, coords[1:]))
        coords = simplify(coords)
        rec = {"len_m": round(length, 1), "coords": [[round(la, 6), round(lo, 6)] for la, lo in coords]}
        out[f"{a}>{b}"] = rec
        done[(a, b)] = rec
        ratios.append((length / max(1.0, haversine(stops[a], stops[b])), a, b, length))
        if (k + 1) % 100 == 0:
            print(f"   {k + 1}/{len(pairs)} ({time.time() - t0:.0f} s)")
    junctions, via = find_junctions(stops, out, raw)
    for key, v in via.items():
        if key in out:
            out[key]["via"] = v
    names = {}
    with open(os.path.join(GTFS, "stops.txt"), encoding="utf-8-sig") as f:
        for r in csv.DictReader(f):
            names[r["stop_id"].strip()] = r["stop_name"].strip()
    print(f"-> {len(junctions)} bifurcaciones fuera de estación:")
    for j, v in junctions.items():
        users = sorted({k for k, vv in via.items() if any(x[0] == j for x in vv)})
        print(f"   {j} cerca de {names.get(v['near'], v['near'])}: " + ", ".join(f"{names.get(k.split('>')[0])}→{names.get(k.split('>')[1])} ({dict(via[k])[j]:.0f} m)" for k in users if k.split('>')[0] == v['near']))
    res = {
        "source": "OpenStreetMap (Overpass API, railway=rail) · © OpenStreetMap contributors, ODbL",
        "generated": time.strftime("%Y-%m-%d"),
        "max_turn_deg": MAX_TURN_DEG,
        "junctions": junctions,
        "cantons": out,
    }
    with open(OUT, "w", encoding="utf-8") as f:
        json.dump(res, f, separators=(",", ":"))
    print(f"OK {len(out)}/{len(pairs)} cantones encaminados → {OUT} ({os.path.getsize(OUT) // 1024} KB)")
    if failed:
        print(f"   {len(failed)} sin ruta (el motor usará la recta): " + ", ".join(f"{a}>{b}" for a, b in failed[:25]))
    ratios.sort(reverse=True)
    print("   Mayores rodeos (vía real / línea recta):")
    for r, a, b, L in ratios[:12]:
        print(f"     {a}>{b}: ×{r:.2f} ({L / 1000:.2f} km)")


if __name__ == "__main__":
    main()
