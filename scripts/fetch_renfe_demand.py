#!/usr/bin/env python3
"""Descarga la demanda REAL de viajeros por estación y franja de 30 min de Rodalies Barcelona.

Fuente OFICIAL: Renfe Data · «Volumen de viajeros por franja horaria (Barcelona)» (2018)
  https://data.renfe.com/dataset/volumen-de-viajeros-por-franja-horaria-barcelona
Columnas: CODIGO_ESTACION;NOMBRE_ESTACION;NUCLEO_CERCANIAS;TRAMO_HORARIO;VIAJEROS_SUBIDOS;VIAJEROS_BAJADOS
CODIGO_ESTACION = stop_id del GTFS. La usa el motor (src/demand.rs) para generar viajeros.

Uso: python scripts/fetch_renfe_demand.py
"""
import os
import urllib.request

URL = ("https://data.renfe.com/dataset/9190f983-e138-42da-901a-b37205562fe4/resource/"
       "1417396e-4d6a-466a-a987-03d07aa92bed/download/barcelona_viajeros_por_franja_csv.csv")
OUT = os.path.join(os.path.dirname(__file__), "..", "raw", "renfe", "barcelona_viajeros_por_franja.csv")


def main():
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    req = urllib.request.Request(URL, headers={"User-Agent": "rod4lia-sim/1.0"})
    with urllib.request.urlopen(req, timeout=120) as r:
        data = r.read()
    with open(OUT, "wb") as f:
        f.write(data)
    print(f"OK {OUT} ({len(data)} bytes)")


if __name__ == "__main__":
    main()
