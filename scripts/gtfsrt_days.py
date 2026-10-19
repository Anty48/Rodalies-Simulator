#!/usr/bin/env python3
"""Resumen por DÍA de los datos reales grabados (PC + nube): cobertura, puntualidad y valoración.

Lee raw/gtfsrt/YYYY-MM-DD.jsonl[.gz] (PC) y raw/gtfsrt/cloud/ (nube); un mismo sondeo (mismo `t`)
grabado por los dos se cuenta una vez. Escribe processed/gtfsrt/days.json para la página /datos.

Por día:
  coverage   % de los minutos del horario de servicio (05:00–24:00, hora de Madrid) con ≥1 sondeo
  punct5     % de muestras con retraso < 5 min      over15  % de muestras con retraso ≥ 15 min
  mean_min   retraso medio (min)                    canceled  viajes cancelados distintos
  alerts     avisos de incidencia distintos
  verdict    normal · alterado · excepcional (dato feo) · incompleto, y el motivo

La valoración usa umbrales fijos y, con ≥5 días completos, también lo habitual de TUS días (mediana y
desviación absoluta mediana): un día muy por encima de lo normal es «excepcional» aunque el umbral fijo
no salte. Cache por fichero (tamaño + fecha): solo se recalculan los días nuevos o que han cambiado.

Uso: python scripts/gtfsrt_days.py
"""
import glob
import gzip
import json
import os
import statistics
import sys
from datetime import date, datetime, timedelta, timezone

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
BASE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
RAW = os.path.join(BASE, "raw", "gtfsrt")
OUT_DIR = os.path.join(BASE, "processed", "gtfsrt")
OUT = os.path.join(OUT_DIR, "days.json")
CACHE = os.path.join(OUT_DIR, "days_cache.json")
SERVICE_FROM_MIN, SERVICE_TO_MIN = 5 * 60, 24 * 60  # horario de servicio de Rodalies (aprox.)
MIN_COVERAGE = 50.0  # por debajo, el día es «incompleto»
DOW = ["lunes", "martes", "miércoles", "jueves", "viernes", "sábado", "domingo"]


def last_sunday(y, m):
    d = date(y, m + 1, 1) - timedelta(days=1) if m < 12 else date(y, 12, 31)
    return d - timedelta(days=(d.weekday() + 1) % 7)


def madrid_minute(ts):
    """Minuto del día en hora de Madrid (CET/CEST: cambio el último domingo de marzo y de octubre
    a las 01:00 UTC). Sin depender de la base de zonas horarias (en Windows no viene con Python)."""
    u = datetime.fromtimestamp(ts, tz=timezone.utc)
    start = datetime.combine(last_sunday(u.year, 3), datetime.min.time(), tzinfo=timezone.utc) + timedelta(hours=1)
    end = datetime.combine(last_sunday(u.year, 10), datetime.min.time(), tzinfo=timezone.utc) + timedelta(hours=1)
    off = 2 if start <= u < end else 1
    loc = u + timedelta(hours=off)
    return loc.hour * 60 + loc.minute


def open_any(path):
    return gzip.open(path, "rt", encoding="utf-8") if path.endswith(".gz") else open(path, encoding="utf-8")


def day_files():
    """{día: {"polls": [rutas], "alerts": [rutas]}} de PC y nube."""
    out = {}
    for d in (RAW, os.path.join(RAW, "cloud")):
        for ext in (".jsonl", ".jsonl.gz"):
            for p in glob.glob(os.path.join(d, "*" + ext)):
                name = os.path.basename(p)[: -len(ext)]
                kind = "alerts" if name.startswith("alerts_") else "polls"
                day = name.replace("alerts_", "")
                if len(day) == 10 and day[4] == "-":
                    out.setdefault(day, {"polls": [], "alerts": []})[kind].append(p)
    return out


def signature(paths):
    return [[os.path.relpath(p, BASE), os.path.getsize(p), int(os.path.getmtime(p))] for p in sorted(paths)]


def summarize(day, polls, alerts):
    seen_t = set()
    minutes = set()
    sources = set()
    delays = []
    canceled = set()
    for p in polls:
        src = "nube" if os.sep + "cloud" + os.sep in p or "/cloud/" in p.replace("\\", "/") else "PC"
        with open_any(p) as f:
            for ln in f:
                try:
                    rec = json.loads(ln)
                except ValueError:
                    continue
                t = rec.get("t")
                if t is None or t in seen_t:
                    continue
                seen_t.add(t)
                sources.add(src)
                m = madrid_minute(t)
                if SERVICE_FROM_MIN <= m < SERVICE_TO_MIN:
                    minutes.add(m)
                for tid in rec.get("c", []):
                    canceled.add(tid)
                for row in rec.get("v", []):
                    dl = row[7] if len(row) > 7 else None
                    if dl is not None:
                        delays.append(max(0, dl))
    al = set()
    for p in alerts:
        with open_any(p) as f:
            for ln in f:
                try:
                    a = json.loads(ln)
                except ValueError:
                    continue
                al.add((a.get("id"), a.get("text")))
    n = len(delays)
    d = date.fromisoformat(day)
    return {
        "day": day,
        "dow": DOW[d.weekday()],
        "polls": len(seen_t),
        "sources": sorted(sources),
        "coverage": round(100.0 * len(minutes) / (SERVICE_TO_MIN - SERVICE_FROM_MIN), 1),
        "samples": n,
        "punct5": round(100.0 * sum(1 for x in delays if x < 300) / n, 1) if n else None,
        "over15": round(100.0 * sum(1 for x in delays if x >= 900) / n, 1) if n else None,
        "mean_min": round(statistics.fmean(delays) / 60, 1) if n else None,
        "canceled": len(canceled),
        "alerts": len(al),
    }


def fixed_level(s):
    """Umbrales fijos (Rodalies): % de muestras con ≥15 min de retraso y viajes cancelados."""
    o, c = s["over15"] or 0, s["canceled"]
    if o >= 25 or c >= 60:
        return 2, f"{o:.0f} % de trenes con ≥15 min de retraso" if o >= 25 else f"{c} viajes cancelados"
    if o >= 10 or c >= 20:
        return 1, f"{o:.0f} % de trenes con ≥15 min de retraso" if o >= 10 else f"{c} viajes cancelados"
    return 0, ""


def classify(days):
    complete = [s for s in days if s["coverage"] >= MIN_COVERAGE and s["samples"]]
    base = None
    if len(complete) >= 5:
        def robust(key):
            xs = [s[key] or 0 for s in complete]
            med = statistics.median(xs)
            mad = statistics.median([abs(x - med) for x in xs]) * 1.4826
            return med, max(mad, 1.0)
        base = {"over15": robust("over15"), "canceled": robust("canceled")}
    for s in days:
        if not s["samples"]:
            s["verdict"], s["lean"], s["why"] = "incompleto", None, "sin datos de retraso"
            continue
        lvl, why = fixed_level(s)
        if base:
            for key, label in (("over15", "retrasos ≥15 min"), ("canceled", "cancelaciones")):
                med, mad = base[key]
                z = ((s[key] or 0) - med) / mad
                zl = 2 if z >= 3 else 1 if z >= 1.5 else 0
                if zl > lvl:
                    lvl, why = zl, f"{label} muy por encima de lo habitual (z = {z:.1f})"
        level = ["normal", "alterado", "excepcional"][lvl]
        if s["coverage"] < MIN_COVERAGE:
            # Poco día grabado: no se puede juzgar del todo, pero se da la tendencia de lo que hay.
            s["verdict"], s["lean"] = "incompleto", level
            s["why"] = f"solo {s['coverage']:.0f} % del día grabado; lo grabado pinta {level}" + (f" ({why})" if why else "")
        else:
            s["verdict"], s["lean"] = level, level
            s["why"] = why or "dentro de lo habitual"
    return base is not None


def main():
    os.makedirs(OUT_DIR, exist_ok=True)
    try:
        cache = json.load(open(CACHE, encoding="utf-8"))
    except (OSError, ValueError):
        cache = {}
    files = day_files()
    days, new_cache = [], {}
    recalc = 0
    for day in sorted(files):
        sig = signature(files[day]["polls"] + files[day]["alerts"])
        c = cache.get(day)
        if c and c.get("sig") == sig:
            s = c["summary"]
        elif files[day]["polls"]:
            s = summarize(day, files[day]["polls"], files[day]["alerts"])
            recalc += 1
        else:
            continue
        new_cache[day] = {"sig": sig, "summary": s}
        days.append(dict(s))
    relative = classify(days)
    json.dump(new_cache, open(CACHE, "w", encoding="utf-8"), ensure_ascii=False)
    out = {"generated": datetime.now().isoformat(timespec="seconds"), "relative": relative, "days": days}
    json.dump(out, open(OUT, "w", encoding="utf-8"), ensure_ascii=False, indent=1)
    print(f"OK {len(days)} días ({recalc} recalculados)")
    for s in days:
        print(f"  {s['day']} {s['dow']:9} cobertura {s['coverage']:5.1f} %  ≥15' {s['over15']}%  cancel {s['canceled']:3}  → {s['verdict']} ({s['why']})")


if __name__ == "__main__":
    main()
