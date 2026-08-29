#!/usr/bin/env python3
"""Importador de infraestructura ADIF (Fase C, calculador fase 3).

Descarga del WFS INSPIRE de ADIF (IDEADIF) las dos capas necesarias y las une:

  * tn-ra:RailwayLink   -> geometría real de la vía (LineString, EPSG:4258 lat/lon)
  * tn-ra:DesignSpeed   -> velocidad máxima de diseño (km/h) por enlace

Se unen por el código numérico del localId (RailwayLink_XXXXX <-> DesignSpeed_XXXXX)
y se escribe processed/adif/rfig_speed.json:

  [{ "code": "011020010", "speed_kmh": 130.0,
     "start":"...", "end":"...",
     "coords": [[lat,lon],...] }, ...]

Fuente: ADIF — IDEADIF, WFS INSPIRE "Red de Transporte Ferroviario de Adif"
        (datos.gob.es e0dat0002). versionId del dataset: ver 'versionId' en el GML.
Uso: python scripts/fetch_adif_cvm.py
"""
import re, json, sys, urllib.request, os

WFS = "https://ideadif.adif.es/services/wfs"
OUT = os.path.join("processed", "adif", "rfig_speed.json")

def get(typename):
    url = (f"{WFS}?service=WFS&version=2.0.0&request=GetFeature"
           f"&typeNames={typename}&count=5000")
    print(f"  descargando {typename} …", flush=True)
    with urllib.request.urlopen(url, timeout=180) as r:
        return r.read().decode("utf-8", "replace")

def code_of(localid):
    m = re.search(r"_(\d+)$", localid)
    return m.group(1) if m else localid

def parse_speeds(xml):
    speeds = {}
    for m in re.finditer(r"<tn-ra:DesignSpeed\b.*?</tn-ra:DesignSpeed>", xml, re.S):
        blk = m.group(0)
        lid = re.search(r"<base:localId>([^<]+)</base:localId>", blk)
        sp  = re.search(r'<tn-ra:speed[^>]*>([\d.]+)</tn-ra:speed>', blk)
        if lid and sp:
            speeds[code_of(lid.group(1))] = float(sp.group(1))
    return speeds

def parse_links(xml):
    links = {}
    for m in re.finditer(r"<tn-ra:RailwayLink\b.*?</tn-ra:RailwayLink>", xml, re.S):
        blk = m.group(0)
        lid = re.search(r"<base:localId>([^<]+)</base:localId>", blk)
        pos = re.search(r"<gml:posList[^>]*>([^<]+)</gml:posList>", blk)
        if not (lid and pos):
            continue
        nums = [float(v) for v in pos.group(1).split()]
        coords = [[round(nums[i], 6), round(nums[i+1], 6)] for i in range(0, len(nums) - 1, 2)]
        # quitar puntos consecutivos idénticos
        clean = [coords[0]] if coords else []
        for c in coords[1:]:
            if c != clean[-1]:
                clean.append(c)
        sn = re.search(r'net:startNode[^>]*xlink:href="[^"]*?_(\w+)"', blk)
        en = re.search(r'net:endNode[^>]*xlink:href="[^"]*?_(\w+)"', blk)
        links[code_of(lid.group(1))] = {
            "coords": clean,
            "start": sn.group(1) if sn else None,
            "end": en.group(1) if en else None,
        }
    return links

def main():
    speeds = parse_speeds(get("tn-ra:DesignSpeed"))
    links  = parse_links(get("tn-ra:RailwayLink"))
    print(f"  DesignSpeed: {len(speeds)} · RailwayLink: {len(links)}")
    out = []
    for code, lk in links.items():
        if code in speeds and lk["coords"]:
            out.append({"code": code, "speed_kmh": speeds[code],
                        "start": lk["start"], "end": lk["end"], "coords": lk["coords"]})
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    json.dump(out, open(OUT, "w", encoding="utf-8"))
    npts = sum(len(x["coords"]) for x in out)
    sset = sorted({x["speed_kmh"] for x in out})
    print(f"✓ {len(out)} enlaces con velocidad · {npts} vértices · {os.path.getsize(OUT)//1024} KB")
    print(f"  velocidades presentes (km/h): {sset}")

if __name__ == "__main__":
    main()
