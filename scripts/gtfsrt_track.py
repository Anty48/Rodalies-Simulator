#!/usr/bin/env python3
"""Seguimiento de retrasos de un DÍA REAL (GTFS-RT grabado), con las MISMAS magnitudes que el motor.

Lee los sondeos de raw/gtfsrt/ (PC) y raw/gtfsrt/cloud/ (nube) de ese día en hora de Madrid (un sondeo
grabado por los dos se cuenta una vez) y escribe processed/gtfsrt/track_<día>.json:

  t0, dt=60, total[], active[]  retraso vivo de la red por minuto: Σ max(0, retraso) de los trenes que
                                circulan y nº de trenes con retraso conocido (= lo que el motor llama
                                «retraso acumulado» y «trenes en servicio»; el medio = total/active).
                                Cada minuto usa el ÚLTIMO sondeo de ese minuto.
  lines{L: {total[], active[]}} lo mismo por línea.
  gaps[[s0, s1]]                minutos de servicio sin datos (recolector parado o feed caído): ahí la
                                serie vale 0 y la interfaz los sombrea.
  trips[{key, trip, line, stops[[stop, t, retraso]]}]
                                retraso al LLEGAR a cada parada (primera observación «parado en» esa
                                parada), en s; `key` = nº de tren + línea (p. ej. «15004R15»), común con
                                los viajes del GTFS que simula el motor (cambia solo el prefijo del día).
  canceled{L: n}                viajes cancelados distintos por línea.
  notices[{t, source, lines, text}]
                                avisos de incidencia PUBLICADOS ese día (Gencat + Renfe, ver
                                scripts/incidents.py) con su hora de publicación; la duración real no
                                se publica.

Además escribe processed/gtfsrt/pos_<día>.json con las POSICIONES de cada tren (para reproducir el
día en el simulador visual): {trips: [{trip, key, line, obs: [[t, lat, lon, estado, parada, retraso]]}]},
una observación por cambio (estado S = parado en, I = llegando a, T = en camino a `parada`).

El retraso es el campo `delay` de trip_updates de Renfe (s; negativo = adelantado). La resolución es el
refresco del feed (~20-30 s). Hora = segundos desde la medianoche de Madrid (como el motor).

Si el resultado ya existe y es más nuevo que los datos del día, no se recalcula.

Uso: python scripts/gtfsrt_track.py --day AAAA-MM-DD
"""
import argparse
import gzip
import json
import os
import re
import sys
from datetime import date, datetime, timedelta, timezone

BASE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
RAW = os.path.join(BASE, "raw", "gtfsrt")
OUT_DIR = os.path.join(BASE, "processed", "gtfsrt")
SERVICE_FROM_S, SERVICE_TO_S = 5 * 3600, 24 * 3600  # horario de servicio (para detectar huecos)
GAP_MIN = 3  # minutos seguidos sin datos que cuentan como hueco
INC_DIR = os.path.join(BASE, "raw", "incidents")
KEY_RE = re.compile(r"^\d{4}[A-Z](\d+R[A-Z0-9]*)$")  # «5190L77803R7» → «77803R7»


def last_sunday(y, m):
    d = date(y, m + 1, 1) - timedelta(days=1) if m < 12 else date(y, 12, 31)
    return d - timedelta(days=(d.weekday() + 1) % 7)


def madrid(ts):
    """(día AAAA-MM-DD, segundos desde la medianoche) en hora de Madrid (CET/CEST), sin tzdata."""
    u = datetime.fromtimestamp(ts, tz=timezone.utc)
    start = datetime.combine(last_sunday(u.year, 3), datetime.min.time(), tzinfo=timezone.utc) + timedelta(hours=1)
    end = datetime.combine(last_sunday(u.year, 10), datetime.min.time(), tzinfo=timezone.utc) + timedelta(hours=1)
    loc = u + timedelta(hours=2 if start <= u < end else 1)
    return loc.strftime("%Y-%m-%d"), loc.hour * 3600 + loc.minute * 60 + loc.second


def open_any(path):
    return gzip.open(path, "rt", encoding="utf-8") if path.endswith(".gz") else open(path, encoding="utf-8")


def source_files(day):
    """Ficheros de sondeos del día y de sus vecinos (un fichero se nombra por la fecha del equipo que
    graba; con el cambio de hora o la nube en UTC, un sondeo del día puede caer en el de al lado)."""
    d = date.fromisoformat(day)
    names = {(d + timedelta(days=k)).isoformat() for k in (-1, 0, 1)}
    out = []
    for folder in (RAW, os.path.join(RAW, "cloud")):
        for n in names:
            for ext in (".jsonl", ".jsonl.gz"):
                p = os.path.join(folder, n + ext)
                if os.path.exists(p):
                    out.append(p)
    return out


def main():
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    ap = argparse.ArgumentParser()
    ap.add_argument("--day", required=True, help="día a analizar (AAAA-MM-DD, hora de Madrid)")
    a = ap.parse_args()
    if not re.fullmatch(r"\d{4}-\d{2}-\d{2}", a.day):
        sys.exit("--day debe ser AAAA-MM-DD")
    os.makedirs(OUT_DIR, exist_ok=True)
    dst = os.path.join(OUT_DIR, f"track_{a.day}.json")
    pos_dst = os.path.join(OUT_DIR, f"pos_{a.day}.json")
    sources = source_files(a.day)
    if not sources:
        sys.exit(f"No hay datos grabados del {a.day}")
    inc_file = os.path.join(INC_DIR, a.day + ".jsonl")
    deps = sources + ([inc_file] if os.path.exists(inc_file) else [])
    newest = max(os.path.getmtime(p) for p in deps)
    if all(os.path.exists(x) and os.path.getmtime(x) >= newest for x in (dst, pos_dst)):
        print(f"OK {dst} (ya calculado)")
        return

    # Sondeos del día (dedupe por `t`), ordenados.
    polls = {}
    for path in sources:
        with open_any(path) as f:
            for ln in f:
                try:
                    rec = json.loads(ln)
                except ValueError:
                    continue
                t = rec.get("t")
                if not t or t in polls:
                    continue
                d, sec = madrid(t)
                if d == a.day:
                    polls[t] = (sec, rec)
    if not polls:
        sys.exit(f"No hay sondeos del {a.day} (hora de Madrid)")

    # Último sondeo de cada minuto → retraso vivo de la red y por línea.
    last_of_min = {}
    for t in sorted(polls):
        sec, rec = polls[t]
        last_of_min[sec // 60] = rec
    per_min = {}  # minuto → (total_s, n, {línea: [total_s, n]})
    for m, rec in last_of_min.items():
        tot, n, by = 0.0, 0, {}
        for row in rec.get("v", []):
            tid, line, _stop, _st, _lat, _lon, _ts, dl = row[:8]
            if dl is None or not line.startswith("R"):
                continue  # sin retraso conocido no cuenta (como un tren que el motor no ve)
            d = max(0.0, float(dl))
            tot += d
            n += 1
            acc = by.setdefault(line, [0.0, 0])
            acc[0] += d
            acc[1] += 1
        if n:
            per_min[m] = (tot, n, by)
    if not per_min:
        sys.exit(f"El {a.day} no trae retrasos (trip_updates vacío)")
    m0, m1 = min(per_min), max(per_min)
    lines = sorted({ln for v in per_min.values() for ln in v[2]})
    total, active = [], []
    lt = {ln: {"total": [], "active": []} for ln in lines}
    for m in range(m0, m1 + 1):
        tot, n, by = per_min.get(m, (0.0, 0, {}))
        total.append(round(tot, 1))
        active.append(n)
        for ln in lines:
            v = by.get(ln, (0.0, 0))
            lt[ln]["total"].append(round(v[0], 1))
            lt[ln]["active"].append(v[1])
    # Huecos: minutos de servicio sin dato.
    gaps, run = [], None
    for m in range(max(m0, SERVICE_FROM_S // 60), min(m1, SERVICE_TO_S // 60) + 1):
        if m not in per_min:
            run = run or [m, m]
            run[1] = m
        else:
            if run and run[1] - run[0] + 1 >= GAP_MIN:
                gaps.append([run[0] * 60, run[1] * 60 + 59])
            run = None
    if run and run[1] - run[0] + 1 >= GAP_MIN:
        gaps.append([run[0] * 60, run[1] * 60 + 59])

    # Llegadas: primera observación «parado en» cada parada, con su retraso. Posiciones: cada cambio.
    trips = {}
    canceled = {}
    pos = {}
    for t in sorted(polls):
        sec, rec = polls[t]
        for row in rec.get("v", []):
            tid, line, stop, st, lat, lon, ts_v, dl = row[:8]
            if not tid or not line.startswith("R") or not lat:
                continue
            when = madrid(ts_v)[1] if ts_v else sec
            pt = pos.setdefault(tid, {"trip": tid, "line": line, "obs": []})
            ob = [when, round(lat, 5), round(lon, 5), st, stop or "", dl]
            obs = pt["obs"]
            if obs and when <= obs[-1][0]:
                continue  # observación repetida o vieja (el feed repite posiciones antiguas)
            # Parado (misma posición y estado): basta la primera y la ÚLTIMA observación de la racha,
            # para saber cuándo vuelve a moverse sin guardar cada sondeo.
            if len(obs) >= 2 and obs[-1][1:5] == ob[1:5] and obs[-2][1:5] == ob[1:5]:
                obs[-1] = ob
            else:
                obs.append(ob)
        for tid in rec.get("c", []):
            m = re.search(r"\d(R[A-Z0-9]*)$", tid)
            if m:
                canceled.setdefault(m.group(1), set()).add(tid)
        for row in rec.get("v", []):
            tid, line, stop, st, _lat, _lon, ts_v, dl = row[:8]
            if not tid or not stop or st != "S" or dl is None or not line.startswith("R"):
                continue
            tr = trips.setdefault(tid, {"trip": tid, "line": line, "stops": []})
            if tr["stops"] and tr["stops"][-1][0] == stop:
                continue  # sigue parado en la misma
            when = madrid(ts_v)[1] if ts_v else sec
            tr["stops"].append([stop, when, int(dl)])
    out_trips = []
    for tr in trips.values():
        m = KEY_RE.match(tr["trip"])
        tr["key"] = m.group(1) if m else tr["trip"]
        out_trips.append(tr)
    out_trips.sort(key=lambda x: (x["stops"][0][1] if x["stops"] else 0, x["trip"]))

    # Avisos de incidencia publicados ese día (no los arrastrados de días anteriores).
    notices = []
    try:
        sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
        import incidents  # noqa: E402  (script hermano: misma clave y fuentes que /datos)
        for it in incidents.list_day(a.day):
            if it.get("before_day"):
                continue
            d, sec = madrid(it["pub"])
            if d == a.day:
                notices.append({"t": sec, "source": it.get("source", ""), "lines": it.get("lines", []), "text": it.get("text", "")})
    except Exception as e:  # sin avisos no se pierde el resto del análisis
        print(f"   (sin avisos de incidencia: {e})")

    res = {
        "day": a.day,
        "source": "Renfe GTFS-RT (vehicle_positions + trip_updates), grabado por scripts/collect_gtfsrt.py",
        "polls": len(polls),
        "t0": m0 * 60,
        "dt": 60,
        "total": total,
        "active": active,
        "lines": lt,
        "gaps": gaps,
        "trips": out_trips,
        "canceled": {k: len(v) for k, v in sorted(canceled.items())},
        "notices": notices,
    }
    with open(dst, "w", encoding="utf-8") as f:
        json.dump(res, f, ensure_ascii=False, separators=(",", ":"))
    pos_out = []
    for tid, pt in sorted(pos.items()):
        m = KEY_RE.match(tid)
        pt["key"] = m.group(1) if m else tid
        if len(pt["obs"]) >= 2:
            pos_out.append(pt)
    with open(pos_dst, "w", encoding="utf-8") as f:
        json.dump({"day": a.day, "trips": pos_out}, f, ensure_ascii=False, separators=(",", ":"))
    print(f"OK {dst}: {len(polls)} sondeos, {len(out_trips)} viajes, {len(gaps)} huecos")


if __name__ == "__main__":
    main()
