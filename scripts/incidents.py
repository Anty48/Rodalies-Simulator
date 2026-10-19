#!/usr/bin/env python3
"""Incidencias de Rodalies con su HORA DE PUBLICACIÓN, asociadas a cada día (para saber después qué
causó los retrasos de un día grabado).

Fuentes oficiales que se pueden leer de forma automática y gratuita:
  · Generalitat — RSS «Incidències Rodalies» (https://www.gencat.cat/rodalies/incidencies_rodalies_rss_ca_ES.xml):
    cada incidencia con su hora de publicación, línea(s) y texto. Solo trae las VIGENTES: por eso se
    captura a menudo (la nube cada ~5 min, el PC cada 5 min cuando graba él, y el botón al momento).
  · Renfe — avisos GTFS-RT (alerts.json): avisos de servicio con su hora de inicio.
  (Twitter/X y WhatsApp no se pueden leer gratis de forma automática: X cobra su API y la parte
  pública devuelve tuits aleatorios de otros años; el canal de WhatsApp no tiene web ni API.)

Se guardan en raw/incidents/AAAA-MM-DD.jsonl (día = día de publicación, hora de Madrid), sin repetir.

  python scripts/incidents.py capture          captura ahora y guarda las nuevas → JSON
  python scripts/incidents.py list --day D     incidencias del día D (capturadas + avisos grabados) → JSON
"""
import glob
import gzip
import hashlib
import json
import os
import re
import sys
import time
import urllib.request
from datetime import date, datetime, timedelta, timezone
from email.utils import parsedate_to_datetime
from html import unescape

BASE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
INC_DIR = os.path.join(BASE, "raw", "incidents")
GTFSRT = os.path.join(BASE, "raw", "gtfsrt")
RSS_URL = "https://www.gencat.cat/rodalies/incidencies_rodalies_rss_ca_ES.xml"
AL_URL = "https://gtfsrt.renfe.com/alerts.json"
UA = {"User-Agent": "Mozilla/5.0 (rod4lia-sim/1.0)"}
RODALIES_ROUTE = re.compile(r"^5\dT")
LINE = re.compile(r"\d(R[A-Z0-9]*)$")


def _last_sunday(y, m):
    d = date(y, m + 1, 1) - timedelta(days=1) if m < 12 else date(y, 12, 31)
    return d - timedelta(days=(d.weekday() + 1) % 7)


def madrid_day(ts):
    """Día (AAAA-MM-DD) en hora de Madrid, sin depender de la base de zonas horarias."""
    u = datetime.fromtimestamp(ts, tz=timezone.utc)
    start = datetime.combine(_last_sunday(u.year, 3), datetime.min.time(), tzinfo=timezone.utc) + timedelta(hours=1)
    end = datetime.combine(_last_sunday(u.year, 10), datetime.min.time(), tzinfo=timezone.utc) + timedelta(hours=1)
    return (u + timedelta(hours=2 if start <= u < end else 1)).strftime("%Y-%m-%d")


def _get(url):
    with urllib.request.urlopen(urllib.request.Request(url, headers=UA), timeout=25) as r:
        return r.read().decode("utf-8", "replace")


def key_of(source, pub, text):
    return hashlib.sha1(f"{source}|{pub}|{text}".encode("utf-8")).hexdigest()[:16]


def fetch_gencat():
    """Incidencias vigentes del RSS oficial de la Generalitat."""
    xml = _get(RSS_URL)
    out = []
    for item in re.findall(r"<item>(.*?)</item>", xml, re.S):
        def tag(t):
            m = re.search(rf"<{t}>(.*?)</{t}>", item, re.S)
            return unescape(re.sub(r"<!\[CDATA\[|\]\]>", "", m.group(1))).strip() if m else ""
        title, desc, pub = tag("title"), tag("description"), tag("pubDate")
        try:
            ts = int(parsedate_to_datetime(pub).timestamp())
        except (TypeError, ValueError):
            continue
        m = re.match(r"L[íi]nia\s+([^.]+)\.", title)
        lines = [x.strip() for x in m.group(1).split(",")] if m else []
        text = desc or title
        out.append({"key": key_of("gencat", ts, text), "source": "gencat", "pub": ts, "lines": lines, "text": text})
    return out


def fetch_renfe():
    """Avisos de servicio de Rodalies vigentes en el GTFS-RT de Renfe (hora = inicio del aviso)."""
    data = json.loads(_get(AL_URL))
    out = []
    for e in data.get("entity", []):
        a = e.get("alert", {})
        routes = [x.get("routeId", "") for x in a.get("informedEntity", [])]
        if not any(RODALIES_ROUTE.match(r or "") for r in routes):
            continue
        tr = ((a.get("descriptionText") or a.get("headerText") or {}).get("translation") or [{}])[0]
        try:
            ts = int(a["activePeriod"][0]["start"])
        except (KeyError, IndexError, TypeError, ValueError):
            ts = int(time.time())
        lines = sorted({m.group(1) for r in routes if (m := LINE.search(r or ""))}, key=lambda l: (len(l), l))
        text = tr.get("text", "")
        out.append({"key": key_of("renfe", ts, text), "source": "renfe", "pub": ts, "lines": lines, "text": text})
    return out


def _read_keys(path):
    keys = set()
    if os.path.exists(path):
        with open(path, encoding="utf-8") as f:
            for ln in f:
                try:
                    keys.add(json.loads(ln)["key"])
                except (ValueError, KeyError):
                    pass
    return keys


def store(items, captured_at=None):
    """Guarda las incidencias nuevas en raw/incidents/<día de publicación>.jsonl. Devuelve cuántas."""
    os.makedirs(INC_DIR, exist_ok=True)
    now = int(captured_at or time.time())
    by_day = {}
    for it in items:
        by_day.setdefault(madrid_day(it["pub"]), []).append(it)
    added = 0
    for day, its in by_day.items():
        path = os.path.join(INC_DIR, f"{day}.jsonl")
        have = _read_keys(path)
        new = [dict(it, captured=it.get("captured", now)) for it in its if it["key"] not in have]
        if new:
            with open(path, "a", encoding="utf-8") as f:
                for it in new:
                    f.write(json.dumps(it, ensure_ascii=False) + "\n")
            added += len(new)
    return added


def capture():
    """Captura ahora de todas las fuentes disponibles. Devuelve {fuente: n o error}."""
    res, items = {}, []
    for name, fn in (("gencat", fetch_gencat), ("renfe", fetch_renfe)):
        try:
            got = fn()
            items += got
            res[name] = len(got)
        except Exception as e:
            res[name] = f"error: {e}"
    res["added"] = store(items)
    return res


def _open(p):
    return gzip.open(p, "rt", encoding="utf-8") if p.endswith(".gz") else open(p, encoding="utf-8")


def list_day(day):
    """Incidencias de un día: las capturadas (publicadas ese día) + los avisos de Renfe grabados ese día
    por el recolector (vigentes ese día aunque empezaran antes). Sin repetir."""
    items = {}
    path = os.path.join(INC_DIR, f"{day}.jsonl")
    if os.path.exists(path):
        with open(path, encoding="utf-8") as f:
            for ln in f:
                try:
                    it = json.loads(ln)
                except ValueError:
                    continue
                items[it["key"]] = it
    texts = {(i["source"], i["text"]) for i in items.values()}
    for d in (GTFSRT, os.path.join(GTFSRT, "cloud")):
        for p in glob.glob(os.path.join(d, f"alerts_{day}.jsonl")) + glob.glob(os.path.join(d, f"alerts_{day}.jsonl.gz")):
            with _open(p) as f:
                for ln in f:
                    try:
                        a = json.loads(ln)
                    except ValueError:
                        continue
                    try:
                        ts = int(a["period"][0]["start"])
                    except (KeyError, IndexError, TypeError, ValueError):
                        ts = a.get("seen") or 0
                    text = a.get("text", "")
                    if ("renfe", text) in texts:
                        continue
                    k = key_of("renfe", ts, text)
                    lines = sorted({m.group(1) for r in a.get("routes") or [] if (m := LINE.search(r or ""))},
                                   key=lambda l: (len(l), l))
                    items.setdefault(k, {"key": k, "source": "renfe", "pub": ts, "lines": lines, "text": text})
    out = sorted(items.values(), key=lambda i: i["pub"])
    for it in out:
        it["before_day"] = madrid_day(it["pub"]) < day  # aviso que ya venía de días anteriores
    return out


def main():
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    cmd = sys.argv[1] if len(sys.argv) > 1 else "capture"
    if cmd == "capture":
        print(json.dumps(capture(), ensure_ascii=False))
    elif cmd == "list":
        day = sys.argv[sys.argv.index("--day") + 1] if "--day" in sys.argv else datetime.now().strftime("%Y-%m-%d")
        with_inc = sorted(os.path.basename(p)[:10] for p in glob.glob(os.path.join(INC_DIR, "*.jsonl")))
        print(json.dumps({"day": day, "items": list_day(day), "days": with_inc}, ensure_ascii=False))
    else:
        sys.exit("uso: incidents.py capture | list --day AAAA-MM-DD")


if __name__ == "__main__":
    main()
