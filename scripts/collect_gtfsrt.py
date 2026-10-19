#!/usr/bin/env python3
"""Recolector del GTFS-RT de Renfe (Cercanías) filtrado a Rodalies de Catalunya.

Cada `--every` segundos (20 por defecto, el refresco de la fuente) descarga
  https://gtfsrt.renfe.com/vehicle_positions.json   (posición + estado + parada de cada tren)
  https://gtfsrt.renfe.com/trip_updates.json        (retraso, cancelaciones y paradas suprimidas)
  https://gtfsrt.renfe.com/alerts.json              (avisos de incidencia, texto libre; cada 2 min)
y guarda SOLO lo de Rodalies en raw/gtfsrt/:
  YYYY-MM-DD.jsonl         una línea por sondeo:
      {"t":ts, "v":[[viaje,línea,stop,estado,lat,lon,ts_v,retraso,vía], ...],
       "c":[viajes cancelados], "k":{viaje:[paradas suprimidas]}}
  alerts_YYYY-MM-DD.jsonl  cada aviso de Rodalies la primera vez que aparece (o cuando cambia)
  collector.log            arranques, errores y un resumen cada ~10 min

Una sola instancia: el recolector escucha en 127.0.0.1:47231 (si ya hay uno, el segundo sale). Por
ese puerto el simulador le pregunta el estado («ping»), le pide que pare («stop») o que vuelva a mirar
YA si la nube graba («recheck»). Mientras la nube (Cloudflare) graba, este se queda EN ESPERA.

De esa serie se deducen después (scripts/analyze_gtfsrt.py) los tiempos REALES de cada tramo y de cada
parada: la transición STOPPED_AT(A) → IN_TRANSIT/INCOMING(B) marca la salida de A y la llegada a B.

Uso:  python scripts/collect_gtfsrt.py [--every 20] [--hours 0 (=infinito)]
      (sin consola: pythonw scripts/collect_gtfsrt.py — así lo arranca la tarea programada)
"""
import argparse
import json
import os
import re
import socket
import sys
import threading
import time
import urllib.request
from datetime import datetime

VP_URL = "https://gtfsrt.renfe.com/vehicle_positions.json"
TU_URL = "https://gtfsrt.renfe.com/trip_updates.json"
AL_URL = "https://gtfsrt.renfe.com/alerts.json"
OUT_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "raw", "gtfsrt")
CTRL_PORT = 47231
# route_id de Rodalies en el GTFS de Renfe (los avisos se refieren a rutas, no a trenes).
RODALIES_ROUTE = re.compile(r"^5\dT")

STATUS = {"STOPPED_AT": "S", "IN_TRANSIT_TO": "T", "INCOMING_AT": "I"}
PLATF = re.compile(r"PLATF\.\((\d+)\)")

# saved_at = hora local (epoch) del último sondeo GUARDADO; next_at = cuándo se hará el siguiente.
state = {"polls": 0, "last": 0, "stop": False, "started": time.time(), "errors": 0, "saved_at": 0.0, "next_at": 0.0,
         "standby": False}

# ---- Solo UN recolector a la vez: si la nube (Cloudflare, ver src/cloud.rs) está grabando, este se
# queda EN ESPERA y solo comprueba de vez en cuando. Si la nube deja de dar señales de vida (o se
# para), toma el relevo él. Así siempre graba exactamente uno.
CLOUD_CFG = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "report", "cloudflare.json")
CLOUD_CHECK_S = 120     # cada cuánto se pregunta a la nube
CLOUD_ALIVE_S = 150     # la nube cuenta como viva si su última alarma fue hace menos de esto


def cloud_recording():
    """True si la nube está desplegada, activada y con señal de vida reciente; False si no;
    None si no se puede saber (sin red): entonces se mantiene lo que se estaba haciendo."""
    try:
        with open(CLOUD_CFG, encoding="utf-8") as f:
            cfg = json.load(f)
    except (OSError, ValueError):
        return False
    if not cfg.get("deployed") or not cfg.get("url") or not cfg.get("token"):
        return False
    try:
        req = urllib.request.Request(cfg["url"].rstrip("/") + "/status",
                                     headers={"Authorization": "Bearer " + cfg["token"], "User-Agent": "rod4lia-sim/1.0"})
        with urllib.request.urlopen(req, timeout=15) as r:
            st = json.loads(r.read().decode("utf-8"))
    except Exception:
        return None
    run = st.get("running") or {}
    alive = run.get("alive_at") or run.get("saved_at") or 0
    return bool(st.get("enabled")) and (st.get("now", time.time()) - alive) < CLOUD_ALIVE_S


def log(msg):
    line = f"[{datetime.now():%Y-%m-%d %H:%M:%S}] {msg}"
    try:
        with open(os.path.join(OUT_DIR, "collector.log"), "a", encoding="utf-8") as f:
            f.write(line + "\n")
    except OSError:
        pass
    if sys.stdout is not None:
        print(line, flush=True)


def fetch(url):
    req = urllib.request.Request(url, headers={"User-Agent": "rod4lia-sim/1.0"})
    with urllib.request.urlopen(req, timeout=25) as r:
        return json.loads(r.read().decode("utf-8"))


def line_of(label):
    return label.split("-")[0]


def is_rodalies_trip(trip_id):
    # «5189D77436R4»: termina en la línea R*. Los de otros núcleos acaban en C*, T*…
    m = re.search(r"\d(R[A-Z0-9]*)$", trip_id or "")
    return m is not None


def poll():
    vp = fetch(VP_URL)
    try:
        tu = fetch(TU_URL)
    except Exception:
        tu = {"entity": []}
    delays, canceled, skipped = {}, [], {}
    for e in tu.get("entity", []):
        u = e.get("tripUpdate", {})
        tid = u.get("trip", {}).get("tripId")
        if not tid or not is_rodalies_trip(tid):
            continue
        if u.get("trip", {}).get("scheduleRelationship") == "CANCELED":
            canceled.append(tid)
        if "delay" in u:
            delays[tid] = u["delay"]
        sk = [s.get("stopId") for s in u.get("stopTimeUpdate", []) if s.get("scheduleRelationship") == "SKIPPED"]
        if sk:
            skipped[tid] = sk
    rows = []
    for e in vp.get("entity", []):
        v = e.get("vehicle", {})
        label = v.get("vehicle", {}).get("label", "") or ""
        line = line_of(label)
        if not (line[:1] == "R" and len(line) >= 2):
            continue
        tid = v.get("trip", {}).get("tripId", "")
        pos = v.get("position", {})
        m = PLATF.search(label)
        rows.append([
            tid,
            line,
            v.get("stopId", "") or "",
            STATUS.get(v.get("currentStatus", ""), "?"),
            round(pos.get("latitude", 0.0), 6),
            round(pos.get("longitude", 0.0), 6),
            int(v.get("timestamp", 0) or 0),
            delays.get(tid),
            int(m.group(1)) if m else None,  # vía de andén anunciada
        ])
    ts = int(vp.get("header", {}).get("timestamp", time.time()))
    return ts, {"t": ts, "v": rows, "c": canceled, "k": skipped}


def poll_alerts(seen):
    """Guarda los avisos de Rodalies nuevos o cambiados (texto libre de Renfe)."""
    al = fetch(AL_URL)
    out = []
    for e in al.get("entity", []):
        a = e.get("alert", {})
        routes = [x.get("routeId", "") for x in a.get("informedEntity", [])]
        if not any(RODALIES_ROUTE.match(r or "") for r in routes):
            continue
        texts = (a.get("descriptionText") or a.get("headerText") or {}).get("translation", [])
        text = texts[0].get("text", "") if texts else ""
        key = (e.get("id"), text)
        if key in seen:
            continue
        seen.add(key)
        out.append({"id": e.get("id"), "seen": int(time.time()), "period": a.get("activePeriod"),
                    "routes": routes, "stops": [x.get("stopId") for x in a.get("informedEntity", []) if x.get("stopId")],
                    "text": text})
    if out:
        day = datetime.now().strftime("%Y-%m-%d")
        with open(os.path.join(OUT_DIR, f"alerts_{day}.jsonl"), "a", encoding="utf-8") as f:
            for x in out:
                f.write(json.dumps(x, ensure_ascii=False) + "\n")
    return len(out)


def control_server(sock):
    """Responde a «ping» (estado) y a «stop» (parar) del simulador."""
    sock.listen(4)
    while not state["stop"]:
        try:
            conn, _ = sock.accept()
        except OSError:
            break
        with conn:
            try:
                cmd = conn.recv(64).decode("utf-8", "replace").strip()
            except OSError:
                continue
            if cmd == "stop":
                state["stop"] = True
                conn.sendall(b"stopping\n")
            elif cmd == "recheck":
                # El simulador acaba de desplegar/arrancar/parar la nube: comprobarla YA (no esperar 2 min).
                state["recheck"] = True
                conn.sendall(b"ok\n")
            else:
                info = {"pid": os.getpid(), "polls": state["polls"], "last": state["last"],
                        "started": int(state["started"]), "errors": state["errors"],
                        "saved_at": state["saved_at"], "next_at": state["next_at"], "now": time.time(),
                        "standby": state["standby"]}
                conn.sendall((json.dumps(info) + "\n").encode())


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--every", type=float, default=20.0)
    ap.add_argument("--hours", type=float, default=0.0, help="0 = sin límite")
    a = ap.parse_args()
    os.makedirs(OUT_DIR, exist_ok=True)
    # Instancia única: si el puerto de control ya está tomado, ya hay un recolector grabando.
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    try:
        sock.bind(("127.0.0.1", CTRL_PORT))
    except OSError:
        if sys.stdout is not None:
            print("Ya hay un recolector en marcha (puerto 47231): no se abre otro.", flush=True)
        return
    threading.Thread(target=control_server, args=(sock,), daemon=True).start()
    log(f"recolector en marcha (pid {os.getpid()}, cada {a.every:.0f} s)")

    t_end = time.time() + a.hours * 3600 if a.hours > 0 else float("inf")
    last_ts = None
    seen_alerts = set()
    next_alerts = 0.0
    next_inc = 0.0
    next_cloud = 0.0
    while time.time() < t_end and not state["stop"]:
        t0 = time.time()
        if t0 >= next_cloud or state.pop("recheck", False):
            next_cloud = t0 + CLOUD_CHECK_S
            cloud = cloud_recording()
            if cloud is not None and cloud != state["standby"]:
                state["standby"] = cloud
                log("la nube está grabando: el recolector del PC queda EN ESPERA" if cloud
                    else "la nube no está grabando: el recolector del PC toma el relevo")
        if state["standby"]:
            state["next_at"] = 0.0
            while not state["stop"] and not state.get("recheck") and time.time() - t0 < 10:
                time.sleep(0.5)
            continue
        try:
            ts, rec = poll()
            if ts != last_ts:  # el feed no siempre se refresca entre sondeos: no duplicar
                last_ts = ts
                day = datetime.fromtimestamp(ts).strftime("%Y-%m-%d")
                with open(os.path.join(OUT_DIR, f"{day}.jsonl"), "a", encoding="utf-8") as f:
                    f.write(json.dumps(rec, separators=(",", ":")) + "\n")
                state["polls"] += 1
                state["last"] = ts
                state["saved_at"] = time.time()
                if state["polls"] % 30 == 1:
                    log(f"sondeo {state['polls']}: {len(rec['v'])} trens Rodalies, {len(rec['c'])} cancel·lats")
            if t0 >= next_alerts:
                next_alerts = t0 + 120
                n = poll_alerts(seen_alerts)
                if n:
                    log(f"{n} avisos nous")
            if t0 >= next_inc:
                # Incidencias del RSS de la Generalitat (solo trae las vigentes: capturar a menudo).
                next_inc = t0 + 300
                try:
                    import incidents
                    added = incidents.store(incidents.fetch_gencat())
                    if added:
                        log(f"{added} incidències noves (Generalitat)")
                except Exception as ex:
                    log(f"incidències: {ex}")
        except Exception as ex:  # red caída: seguir intentándolo
            state["errors"] += 1
            if state["errors"] % 20 == 1:
                log(f"error: {ex}")
        # Espera troceada: así un «stop» se atiende enseguida.
        state["next_at"] = t0 + max(1.0, a.every)
        while not state["stop"] and not state.get("recheck") and time.time() - t0 < max(1.0, a.every):
            time.sleep(0.5)
    log("recolector aturat")
    sock.close()


if __name__ == "__main__":
    main()
