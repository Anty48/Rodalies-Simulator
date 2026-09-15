#!/usr/bin/env python3
"""Ingesta de velocidades máximas de vía desde OpenStreetMap / OpenRailwayMap.

La CVM de ADIF (processed/adif/rfig_speed.json) mezcla TODAS las vías (incluida la alta velocidad)
y no distingue tipos, así que el emparejamiento por cercanía se pega a las vías paralelas de AVE
(valores de 300 km/h irreales para Rodalies) y le faltan restricciones. OSM tiene `maxspeed` por vía
y permite EXCLUIR la alta velocidad (`highspeed=yes`), lo que arregla ambos problemas.

Este script consulta Overpass por las vías `railway=rail` de Catalunya con `maxspeed`, excluyendo la
alta velocidad, y las escribe en el MISMO esquema que espera `AdifNet::load`
(`[{"speed_kmh": f, "coords": [[lat,lon],...]}]`) en `processed/adif/osm_speed.json`.

Uso:  python scripts/fetch_osm_maxspeed.py
El binario prefiere `processed/adif/osm_speed.json` si existe (ver src/main.rs / src/server.rs).
"""
import json
import os
import re
import sys
import urllib.request

OVERPASS = "https://overpass-api.de/api/interpreter"
OUT = os.path.join("processed", "adif", "osm_speed.json")

# Vías de tren convencionales de Catalunya con maxspeed, EXCLUYENDO la alta velocidad (LAV/AVE).
QUERY = r"""
[out:json][timeout:300];
area["ISO3166-2"="ES-CT"][admin_level=4]->.cat;
(
  way(area.cat)["railway"="rail"]["maxspeed"][!"highspeed"];
  way(area.cat)["railway"="rail"]["maxspeed"]["highspeed"!="yes"];
);
out geom;
"""


def parse_maxspeed(v):
    """maxspeed OSM -> km/h (float). Admite '140', '160;100', 'RO:...'. Toma el mayor número."""
    nums = [float(x) for x in re.findall(r"\d+(?:\.\d+)?", str(v))]
    return max(nums) if nums else 0.0


def main():
    try:
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
        sys.stderr.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass
    print("-> Consultando Overpass (railway=rail con maxspeed en Catalunya, sin alta velocidad)…")
    req = urllib.request.Request(OVERPASS, data=("data=" + QUERY).encode("utf-8"),
                                 headers={"User-Agent": "rodalies-sim/1.0 (maxspeed ingest)"})
    try:
        with urllib.request.urlopen(req, timeout=320) as r:
            data = json.load(r)
    except Exception as e:
        print(f"ERROR Error consultando Overpass: {e}", file=sys.stderr)
        print("  (Overpass puede estar saturado; reintenta más tarde o usa otra instancia.)", file=sys.stderr)
        sys.exit(1)

    feats = []
    for el in data.get("elements", []):
        if el.get("type") != "way":
            continue
        sp = parse_maxspeed(el.get("tags", {}).get("maxspeed"))
        if sp <= 0:
            continue
        geom = el.get("geometry") or []
        coords = [[g["lat"], g["lon"]] for g in geom if "lat" in g and "lon" in g]
        if len(coords) >= 2:
            feats.append({"speed_kmh": sp, "coords": coords})

    if not feats:
        print("ERROR Overpass no devolvió vías con maxspeed. Revisa la consulta/área.", file=sys.stderr)
        sys.exit(1)

    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w", encoding="utf-8") as f:
        json.dump(feats, f)
    total_km = sum(len(x["coords"]) for x in feats)
    print(f"OK {len(feats)} vías con maxspeed escritas en {OUT} ({total_km} vértices).")
    print("  Reinicia el binario: preferirá osm_speed.json sobre rfig_speed.json.")


if __name__ == "__main__":
    main()
