"""
generar_horario_oficial.py
============================
Segunda fase del pipeline de horarios OFICIALES. Toma horarios_oficiales.json
(generado por extraer_horarios_pdf.py a partir de los PDF reales de Rodalies)
y produce los DOS ficheros que el simulador realmente lee:
- datos/horario_topologia.json: orden de estaciones + duracion de cada tramo
  (en segundos), calibrada con tiempos REALES sacados del PDF (no ya una
  estimacion proporcional a la distancia).
- datos/horario_servicios.json: lista de servicios {origen, destino, salida}
  por linea, tal y como los consume ScheduleManager.gd.

Sustituye COMPLETAMENTE el pipeline anterior (gtfs_processor.py ->
generar_horarios_definitivos.py -> generar_r3_historico.py), que dependia de
un GTFS desactualizado (R3 truncada por obras) y de tiempos de tramo dados a
mano para R3. Ahora las 8 lineas del simulador (R1, R2, R2_NORD, R2_SUD, R3,
R4, R7, R8) usan la misma fuente: los PDF oficiales.
"""
import json
import statistics

BASE = "C:/Games/rodalia-447/"

LINEAS_MOTOR = ["R1", "R2", "R2_NORD", "R2_SUD", "R3", "R4", "R7", "R8"]

ESPERA_SEG_DEFECTO = 37.0  # igual que el pipeline anterior (parada tipica en estacion)

# Ajustes manuales sobre el horario oficial, por capacidad real de vias (no
# por error de los datos). FABRA_PUIG solo tiene 2 apartaderos reales para R7
# (4 vias sin corredores propios), pero la R7 necesita nacer 7 trenes ahi a
# lo largo del dia; CEDANYOLA_U solo puede absorber 3 de los 5 que sobran por
# reposicionamiento automatico (ver ScheduleManager._buscar_reserva_cercana),
# asi que los servicios de 09:33 y 17:51 se quedaban sin apartadero en ningun
# lado y el tren dormia en via principal, bloqueando Fabra i Puig. Se
# eliminan esos dos servicios (confirmado con la autora, ver conversacion
# 2026-07-12): con 5 huecos en vez de 7 (2 propios + 3 reposicionados), la
# flota de R7 cabe entera sin ocupar ninguna via principal para dormir.
SERVICIOS_A_ELIMINAR = {
    "R7": {("FABRA_PUIG", "CEDANYOLA_U", "09:33:00"), ("FABRA_PUIG", "CEDANYOLA_U", "17:51:00")},
}


def hhmmss_a_seg(hhmmss):
    h, m, s = (int(x) for x in hhmmss.split(":"))
    return h * 3600 + m * 60 + s


def construir_orden_r2(tags_estacion):
    """Orden maestro de las 34 estaciones del corredor completo de R2.pdf
    (Ma�anet-Massanes -> Sant Vicen� de Calders), sacado directamente de la
    cabecera de la pagina 0 -- se reextrae aqui en vez de guardarlo en
    horarios_oficiales.json porque ese fichero solo trae servicios, no la
    topologia cruda."""
    import pdfplumber
    from extraer_horarios_pdf import extraer_bloques_pagina, construir_indice_estaciones
    idx = construir_indice_estaciones()
    with pdfplumber.open(BASE + "R2.pdf") as pdf:
        bloques = extraer_bloques_pagina(pdf.pages[0], *idx)
    return bloques[0]["columnas"]


def construir_orden_linea(codigo_pdf):
    """Orden de estaciones de la direccion 'ida' (primera pagina) de un PDF
    de una sola linea (R1, R3, R4, R7, R8)."""
    import pdfplumber
    from extraer_horarios_pdf import extraer_bloques_pagina, construir_indice_estaciones
    idx = construir_indice_estaciones()
    with pdfplumber.open(BASE + codigo_pdf + ".pdf") as pdf:
        for pagina in pdf.pages:
            bloques = extraer_bloques_pagina(pagina, *idx)
            for b in bloques:
                if b["calendario"] == "feiners" and len(b["columnas"]) >= 2:
                    return b["columnas"]
    raise RuntimeError("No se encontro cabecera valida en " + codigo_pdf + ".pdf")


def calcular_ordenes(tags_estacion):
    orden_r2_completo = construir_orden_r2(tags_estacion)
    ordenes = {
        "R2": [s for s in orden_r2_completo if "R2" in tags_estacion.get(s, set())],
        "R2_NORD": [s for s in orden_r2_completo if "R2_NORD" in tags_estacion.get(s, set())],
        "R2_SUD": [s for s in orden_r2_completo if "R2_SUD" in tags_estacion.get(s, set())],
    }
    for codigo in ["R1", "R3", "R4", "R7", "R8"]:
        ordenes[codigo] = construir_orden_linea(codigo)
    return ordenes


def calibrar_tramos(orden, servicios):
    """Duracion real (mediana, en segundos) de cada tramo consecutivo de
    'orden', usando solo pares de paradas que EN LA MISMA fila del PDF son
    ademas consecutivas en 'orden' (o sea: el tren no se salto ninguna
    estacion intermedia real entre esas dos paradas)."""
    posicion = {sid: i for i, sid in enumerate(orden)}
    muestras = [[] for _ in range(len(orden) - 1)]

    for s in servicios:
        paradas = [(sid, hhmmss_a_seg(h)) for sid, h in s["paradas"].items() if sid in posicion]
        paradas.sort(key=lambda p: posicion[p[0]])
        for (sid_a, t_a), (sid_b, t_b) in zip(paradas, paradas[1:]):
            i_a, i_b = posicion[sid_a], posicion[sid_b]
            if i_b == i_a + 1 and t_b > t_a:
                muestras[i_a].append(t_b - t_a)

    tramos = []
    huecos = []
    for i, m in enumerate(muestras):
        if m:
            tramos.append(statistics.median(m))
        else:
            tramos.append(None)
            huecos.append((orden[i], orden[i + 1]))
    return tramos, huecos


def rellenar_huecos_por_distancia(orden, tramos, est_por_id):
    """Para tramos sin ninguna observacion directa (huecos), reparte
    proporcionalmente a la distancia real el tiempo del primer 'salto largo'
    disponible que los englobe -- mismo criterio que el pipeline anterior,
    usado solo como ultimo recurso."""
    import math

    def distancia_km(a, b):
        lat1, lon1 = est_por_id[a]["lat"], est_por_id[a]["lon"]
        lat2, lon2 = est_por_id[b]["lat"], est_por_id[b]["lon"]
        cos_lat = math.cos(math.radians((lat1 + lat2) / 2.0))
        dlat_km = (lat2 - lat1) * 111.0
        dlon_km = (lon2 - lon1) * cos_lat * 111.0
        return math.sqrt(dlat_km * dlat_km + dlon_km * dlon_km)

    i = 0
    n = len(tramos)
    while i < n:
        if tramos[i] is not None:
            i += 1
            continue
        j = i
        while j < n and tramos[j] is None:
            j += 1
        # tramos[i..j-1] son None; necesitamos un tiempo total para ese
        # tramo agregado, que no tenemos (no hay fila que lo cubra entero).
        # Fallback: velocidad media generica 60 km/h.
        for k in range(i, j):
            d = distancia_km(orden[k], orden[k + 1])
            tramos[k] = max(30.0, d / 60.0 * 3600.0)
        i = j
    return tramos


def main():
    servicios_todos = json.load(open(BASE + "horarios_oficiales.json", encoding="utf-8"))
    est = json.load(open(BASE + "datos/estaciones.json", encoding="utf-8"))
    est_por_id = {e["id"]: e for e in est}
    tags_estacion = {e["id"]: set(e.get("lineas", [])) for e in est}

    ordenes = calcular_ordenes(tags_estacion)

    topologia = {}
    servicios_final = {}
    for linea in LINEAS_MOTOR:
        orden = ordenes[linea]
        servicios = servicios_todos.get(linea, [])
        a_eliminar = SERVICIOS_A_ELIMINAR.get(linea, set())
        if a_eliminar:
            servicios = [s for s in servicios
                         if (s["origen"], s["destino"], s["salida"]) not in a_eliminar]
        tramos, huecos = calibrar_tramos(orden, servicios)
        if huecos:
            print(f"{linea}: {len(huecos)} tramos sin observacion directa, "
                  f"rellenados por distancia -> {huecos}")
            tramos = rellenar_huecos_por_distancia(orden, tramos, est_por_id)

        topologia[linea] = {
            "estaciones": [{"id": sid, "espera_seg": ESPERA_SEG_DEFECTO} for sid in orden],
            "tramos": tramos,
        }
        servicios_final[linea] = sorted(
            [{"origen": s["origen"], "destino": s["destino"], "salida": s["salida"]}
             for s in servicios],
            key=lambda s: s["salida"],
        )
        print(f"{linea}: {len(orden)} estaciones, {len(tramos)} tramos, "
              f"{len(servicios_final[linea])} servicios/dia")

    with open(BASE + "datos/horario_topologia.json", "w", encoding="utf-8") as f:
        json.dump(topologia, f, indent=2, ensure_ascii=False)
    with open(BASE + "datos/horario_servicios.json", "w", encoding="utf-8") as f:
        json.dump(servicios_final, f, indent=2, ensure_ascii=False)
    print("\nEscritos datos/horario_topologia.json y datos/horario_servicios.json")


if __name__ == "__main__":
    main()
