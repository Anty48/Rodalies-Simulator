#!/usr/bin/env python3
"""Descarga las Limitaciones Temporales de Velocidad (LTV) de ADIF y las procesa.

Fuente OFICIAL: ADIF — HUB LTV (ArcGIS) · FeatureServer
  https://services7.arcgis.com/XTupIrLX53AjaJqO/arcgis/rest/services/LTV_2/FeatureServer/0
  (portal https://ltv-adif.hub.arcgis.com). Geometría: punto (inicio de la LTV);
  atributos: RESTRICCIONVELOCIDAD (km/h), PKINI/PKFIN, CODLINEA/DESCLINEA, MOTIVO, fechas.

Escribe processed/adif/ltv.json con la FECHA del snapshot (dato temporal, cambia a diario).
Uso: python scripts/fetch_adif_ltv.py
"""
import json, os, urllib.request, datetime

FS = "https://services7.arcgis.com/XTupIrLX53AjaJqO/arcgis/rest/services/LTV_2/FeatureServer/0/query"
OUT = os.path.join("processed", "adif", "ltv.json")

def page(offset):
    q = (f"{FS}?where=1=1&outFields=CODLINEA,DESCLINEA,PKINI,PKFIN,RESTRICCIONVELOCIDAD,"
         f"MOTIVO,FECHAVIGORLTV,FECHAFINPREV&outSR=4326&f=geojson"
         f"&resultRecordCount=1000&resultOffset={offset}")
    with urllib.request.urlopen(q, timeout=120) as r:
        return json.loads(r.read().decode("utf-8", "replace"))

def main():
    feats, off = [], 0
    while True:
        d = page(off)
        f = d.get("features", [])
        feats += f
        if len(f) < 1000 or not d.get("properties", {}).get("exceededTransferLimit"):
            if len(f) < 1000:
                break
        off += 1000
        if off > 20000:
            break
    out = []
    for f in feats:
        p = f.get("properties", {})
        g = f.get("geometry") or {}
        c = g.get("coordinates")
        v = p.get("RESTRICCIONVELOCIDAD")
        if not c or v in (None, 0):
            continue
        pki, pkf = p.get("PKINI"), p.get("PKFIN")
        span = abs((pkf - pki) * 1000.0) if (pki is not None and pkf is not None) else 0.0
        out.append({
            "lon": round(c[0], 6), "lat": round(c[1], 6),
            "speed_kmh": float(v), "span_m": round(span, 1),
            "line": p.get("CODLINEA"), "desc": p.get("DESCLINEA"),
            "motivo": p.get("MOTIVO"),
        })
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    doc = {"snapshot": datetime.date.today().isoformat(),
           "source": "ADIF — HUB LTV (ArcGIS FeatureServer LTV_2)",
           "count": len(out), "ltvs": out}
    json.dump(doc, open(OUT, "w", encoding="utf-8"), ensure_ascii=False)
    print(f"OK: {len(out)} LTV -> {OUT} (snapshot {doc['snapshot']})")

if __name__ == "__main__":
    main()
