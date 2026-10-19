#!/usr/bin/env python3
"""Deduce los tiempos REALES de tramo y de parada de Rodalies a partir de lo grabado por
scripts/collect_gtfsrt.py (raw/gtfsrt/*.jsonl[.gz] del PC y raw/gtfsrt/cloud/ descargado del
servidor de la nube; un sondeo repetido en ambos se cuenta una vez) y escribe
processed/gtfsrt/observed.json.

Método (resolución = el refresco del feed, ~20-30 s):
  * Por viaje (tripId) se ordenan las observaciones (timestamp del vehículo, parada, estado).
  * LLEGADA a X  = punto medio entre la última observación que aún NO estaba parada en X y la primera
                   STOPPED_AT X.  SALIDA de X = punto medio entre la última STOPPED_AT X y la siguiente.
    (el punto medio quita el sesgo de muestreo: el evento real cae en algún sitio del intervalo).
  * TRAMO X→Y = llegada(Y) − salida(X) entre dos paradas CONSECUTIVAS OBSERVADAS (puede haber
    paradas intermedias cortas que el feed no llegó a ver: el tramo las incluye).
  * PARADA en X = salida(X) − llegada(X).
  * Si una parada dura menos que el refresco puede no verse STOPPED_AT: el tramo abarca entonces
    varias paradas (se cuenta en `missed_stops`).
  * Retraso: el campo `delay` de trip_updates (s) por línea → distribución real de retrasos.

Con --day AAAA-MM-DD analiza solo ese día y escribe processed/gtfsrt/observed_<día>.json (la página
/datos lo usa para «Realidad por línea»; observed.json, el de todos los días, no se toca). Si el
resultado ya existe y es más nuevo que los datos del día, no se recalcula.

Uso: python scripts/analyze_gtfsrt.py [--min-n 3] [--day AAAA-MM-DD]
"""
import argparse
import gzip
import sys
import glob
import re
import json
import os
import statistics
from collections import defaultdict

BASE = os.path.join(os.path.dirname(__file__), "..")
RAW = os.path.join(BASE, "raw", "gtfsrt")
OUT_DIR = os.path.join(BASE, "processed", "gtfsrt")


def data_files(pattern):
    """Ficheros de datos del PC (raw/gtfsrt) y descargados de la nube (raw/gtfsrt/cloud), sin
    comprimir o .gz, ordenados por día. Devuelve (ruta, día)."""
    out = []
    for d in (RAW, os.path.join(RAW, "cloud")):
        for ext in (".jsonl", ".jsonl.gz"):
            for path in glob.glob(os.path.join(d, pattern + ext)):
                name = os.path.basename(path)
                day = name[: -len(ext)].replace("alerts_", "")
                out.append((path, day))
    return sorted(out, key=lambda x: (x[1], x[0]))


def open_any(path):
    return gzip.open(path, "rt", encoding="utf-8") if path.endswith(".gz") else open(path, encoding="utf-8")


def pct(v, p):
    if not v:
        return None
    v = sorted(v)
    k = (len(v) - 1) * p
    lo, hi = int(k), min(int(k) + 1, len(v) - 1)
    return v[lo] + (v[hi] - v[lo]) * (k - lo)


def summary(v):
    return {
        "n": len(v),
        "median_s": round(statistics.median(v), 1),
        "mean_s": round(statistics.fmean(v), 1),
        "p25_s": round(pct(v, 0.25), 1),
        "p75_s": round(pct(v, 0.75), 1),
        "p90_s": round(pct(v, 0.90), 1),
    }


def main():
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    ap = argparse.ArgumentParser()
    ap.add_argument("--min-n", type=int, default=3)
    ap.add_argument("--day", default=None, help="analizar solo este día (AAAA-MM-DD)")
    a = ap.parse_args()
    if a.day and not re.fullmatch(r"\d{4}-\d{2}-\d{2}", a.day):
        sys.exit("--day debe ser AAAA-MM-DD")
    os.makedirs(OUT_DIR, exist_ok=True)
    dst = os.path.join(OUT_DIR, f"observed_{a.day}.json" if a.day else "observed.json")
    sources = [p for p, _d in data_files(a.day or "*")]
    if a.day:
        if not sources:
            sys.exit(f"No hay datos grabados del {a.day}")
        if os.path.exists(dst) and os.path.getmtime(dst) >= max(os.path.getmtime(p) for p in sources):
            print(f"OK {dst} (ya calculado)")
            return

    obs = defaultdict(list)  # (día, trip) -> [(ts, stop, estado)]
    line_of = {}
    delays = defaultdict(list)  # línea -> [retraso s] (una muestra por viaje y sondeo)
    days = []
    polls = 0
    canceled = defaultdict(set)  # línea -> viajes cancelados (únicos por día)
    seen_t = set()  # el mismo sondeo puede venir del PC y de la nube: se cuenta una vez
    for path, day in data_files(a.day or "[0-9]*"):
        if day not in days:
            days.append(day)
        with open_any(path) as f:
            for ln in f:
                try:
                    rec = json.loads(ln)
                except ValueError:
                    continue
                if rec.get("t") in seen_t:
                    continue
                seen_t.add(rec.get("t"))
                polls += 1
                for tid in rec.get("c", []):
                    m = re.search(r"\d(R[A-Z0-9]*)$", tid)
                    if m:
                        canceled[m.group(1)].add((day, tid))
                for row in rec["v"]:
                    tid, line, stop, st, _lat, _lon, ts, dl = row[:8]
                    if not tid or not stop or not line.startswith("R"):
                        continue
                    key = (day, tid)
                    obs[key].append((ts or rec["t"], stop, st))
                    line_of[key] = line
                    if dl is not None:
                        delays[line].append(dl)

    seg = defaultdict(list)
    dwell = defaultdict(list)
    seg_line = defaultdict(set)
    missed = 0
    for key, rows in obs.items():
        rows.sort()
        # Colapsar duplicados exactos (mismo ts) y construir los intervalos parado-en-X.
        dedup = []
        for r in rows:
            if not dedup or r[0] != dedup[-1][0]:
                dedup.append(r)
        visits = []  # [stop, t_arr, t_dep]
        i = 0
        n = len(dedup)
        while i < n:
            ts, stop, st = dedup[i]
            if st != "S":
                i += 1
                continue
            j = i
            while j + 1 < n and dedup[j + 1][2] == "S" and dedup[j + 1][1] == stop:
                j += 1
            prev_t = dedup[i - 1][0] if i > 0 else None
            next_t = dedup[j + 1][0] if j + 1 < n else None
            t_arr = (prev_t + dedup[i][0]) / 2 if prev_t is not None else None
            t_dep = (dedup[j][0] + next_t) / 2 if next_t is not None else None
            visits.append((stop, t_arr, t_dep, i, j))
            i = j + 1
        for k, (stop, ta, td, _i, _j) in enumerate(visits):
            if ta is not None and td is not None and td > ta:
                dwell[stop].append(td - ta)
            if k + 1 < len(visits):
                nstop, nta, _ntd, ni, _nj = visits[k + 1]
                if td is None or nta is None or nstop == stop:
                    continue
                # Si alguna parada intermedia no se vio como STOPPED_AT (parada más corta que el
                # refresco del feed), el tramo cubre varias paradas: se guarda igual (A→C, con las
                # paradas intermedias dentro) y el comparador lo mide igual en la simulación.
                between = {dedup[x][1] for x in range(visits[k][4] + 1, ni)}
                between.discard(nstop)
                if between:
                    missed += 1
                if nta > td:
                    seg[(stop, nstop)].append(nta - td)
                    seg_line[(stop, nstop)].add(line_of[key])

    out = {
        "source": "Renfe GTFS-RT (vehicle_positions + trip_updates), recolectado con scripts/collect_gtfsrt.py",
        "days": days,
        "polls": polls,
        "trips": len(obs),
        "missed_stops": missed,
        "segments": {},
        "dwells": {},
        "delays_by_line": {},
    }
    out["segments"] = {
        f"{x}|{y}": {**summary(v), "lines": sorted(seg_line[(x, y)])}
        for (x, y), v in seg.items()
        if len(v) >= a.min_n
    }
    out["dwells"] = {s: summary(v) for s, v in dwell.items() if len(v) >= a.min_n}
    for line, v in delays.items():
        if len(v) >= a.min_n:
            d = summary(v)
            d["punct_3min"] = round(sum(1 for x in v if x < 180) / len(v), 3)
            d["punct_5min"] = round(sum(1 for x in v if x < 300) / len(v), 3)
            d["over_15min"] = round(sum(1 for x in v if x >= 900) / len(v), 3)
            out["delays_by_line"][line] = d
    out["canceled_by_line"] = {l: len(v) for l, v in sorted(canceled.items())}
    alerts = []
    seen_al = set()
    for path, _day in data_files(f"alerts_{a.day}" if a.day else "alerts_*"):
        with open_any(path) as f:
            for ln in f:
                try:
                    a = json.loads(ln)
                except ValueError:
                    continue
                if (a.get("id"), a.get("text")) not in seen_al:
                    seen_al.add((a.get("id"), a.get("text")))
                    alerts.append(a)
    alerts.sort(key=lambda a: a.get("seen") or 0)
    out["alerts"] = len(alerts)
    out["alerts_last"] = [{"seen": a.get("seen"), "text": a.get("text", "")[:300]} for a in alerts[-20:]]
    with open(dst, "w", encoding="utf-8") as f:
        json.dump(out, f, ensure_ascii=False, indent=1)
    print(f"OK {dst}: {len(out['segments'])} tramos, {len(out['dwells'])} paradas, "
          f"{polls} sondeos, {len(obs)} viajes, {missed} tramos con paradas intermedias no vistas")
    if out["canceled_by_line"]:
        print("  cancelados: " + ", ".join(f"{l} {n}" for l, n in out["canceled_by_line"].items()))
    print(f"  avisos de incidencia guardados: {len(alerts)}")
    for line, d in sorted(out["delays_by_line"].items()):
        print(f"  {line:5} retraso mediano {d['median_s']/60:5.1f} min · p90 {d['p90_s']/60:5.1f} min · "
              f"puntual<5' {d['punct_5min']*100:4.0f}% · >15' {d['over_15min']*100:4.1f}%")


if __name__ == "__main__":
    main()
