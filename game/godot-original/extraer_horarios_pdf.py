"""
extraer_horarios_pdf.py
========================
Extrae las horas OFICIALES de circulacion de los PDF de Rodalies (R1.pdf,
R2.pdf, R3.pdf, R4.pdf, R7.pdf, R8.pdf) que la autora ha colocado en la raiz
del proyecto, y las vuelca en un unico JSON limpio: horarios_oficiales.json.

Formato de las tablas oficiales (comprobado a mano con pdfplumber en R3.pdf):
- Cada pagina es UNA direccion (origen->destino) de una linea.
- Dentro de la pagina puede haber 1 o mas "bloques": cada bloque empieza con
  la etiqueta de calendario ("Feiners Laborables Weekdays" o "Dissabtes,
  diumenges i festius"), seguida de una cabecera con el nombre de cada
  estacion escrito en DIAGONAL (45 grados, caracter a caracter, en el orden
  real del stream de texto -- no hace falta reordenar, solo trocear por
  saltos de posicion grandes) y despues N filas, una por tren, con horas
  "H.MM" o "HH.MM" posicionadas por columna (celda vacia = el tren no para
  en esa estacion).
- Solo nos interesan los bloques "Feiners" (dias laborables). Los bloques de
  fin de semana/festivos se descartan por completo.

Salida por servicio: {"linea", "calendario", "origen", "destino", "salida",
"llegada", "paradas": {id_estacion: "HH:MM", ...}} -- "paradas" incluye TODAS
las estaciones con hora conocida (para poder recalibrar los tramos con datos
reales), pero el simulador solo necesita origen/destino/salida.
"""
import json
import math
import re
import unicodedata
import pdfplumber

BASE = "C:/Games/rodalia-447/"

LINEAS_PDF = ["R1", "R2", "R3", "R4", "R7", "R8"]

# Umbral (distancia euclidea entre caracteres consecutivos) para cortar una
# palabra de la cabecera diagonal. Los caracteres de una misma palabra estan
# separados ~2-8 unidades; entre palabras el salto es mucho mayor.
UMBRAL_CORTE_PALABRA = 15.0

RE_HORA = re.compile(r"^(\d{1,2})\.(\d{2})$")
# Variante sin anclas, para localizar cada valor "H.MM"/"HH.MM" DENTRO de una
# cadena mas larga (reconstruida caracter a caracter, ver extraer_bloques_pagina).
RE_HORA_INTERNA = re.compile(r"\d{1,2}\.\d{2}")


def normalizar(texto: str) -> str:
    """Minusculas, sin acentos, sin puntuacion -- para comparar nombres de
    estacion del PDF contra los de datos/estaciones.json sin depender de
    como cada uno haya escrito tildes/guiones/abreviaturas."""
    t = unicodedata.normalize("NFKD", texto)
    t = "".join(ch for ch in t if not unicodedata.combining(ch))
    t = t.lower()
    t = t.replace("�", "")
    t = re.sub(r"[^a-z0-9]+", " ", t)
    return t.strip()


# Abreviaturas propias de los PDF que el normalizado por si solo no resuelve.
EXPANSIONES = {
    "st": "sant", "sts": "sants", "sta": "santa",
    "pl": "placa", "ptge": "passatge",
}


def normalizar_expandido(texto: str) -> str:
    t = normalizar(texto)
    palabras = [EXPANSIONES.get(p, p) for p in t.split(" ")]
    return " ".join(palabras)


# Nombre de PDF (normalizado) -> id de datos/estaciones.json, solo para los
# casos donde el emparejamiento automatico (por substring) no basta.
OVERRIDES_ESTACION = {
    "barcelona sants": "SANTS",
    "barcelona pl catalunya": "CATALUNYA",
    "barcelona placa catalunya": "CATALUNYA",
    "barcelona passeig de gracia": "PASSEIG",
    "barcelona arc de triomf": "ARC_TRIOMF",
    "barcelona el clot arago": "CLOT",
    "barcelona el clot": "CLOT",
    "barcelona la sagrera meridiana": "SAGRERA",
    "barcelona sant andreu comtal": "ST_ANDREU",
    "barcelona sant andreu": "ST_ANDREU",
    "barcelona estacio de franca": "EDF",
    "barcelona fabra i puig": "FABRA_PUIG",
    "barcelona torre baro vallbona": "TORRE_BARO_VALLBONA",
    "santa perpetua de mogoda la florida": "STA_PERPETUA_R",
    "santa perpetua de mogoda": "STA_PERPETUA_R",
    "mollet santa rosa": "MOLLET_STA_ROSA",
    "mollet sant fost": "MOLLET_ST_FOST",
    "granollers canovelles": "GRANOLLERS_CAN",
    "granollers centre": "GRANOLLERS",
    "les franqueses del valles": "LES_FRANQUESES_VALLES",
    "les franqueses granollers nord": "LES_FRANQUESES_GRAN_NORD",
    "sant marti de centelles": "ST_MARTI_CENTELLES",
    "baleny els hostalets": "BALENYA_HOSTALETS",
    "baleny tona seva": "BALENYA_TONA_SEVA",
    "sant quirze de besora": "ST_QUIRZE_BESORA",
    "la farga de bebie": "LA_FARGA_BEBIE",
    "urtx alp": "URTX_ALP",
    "la tor de querol enveig": "LA_TOR_QUEROL",
    "la tor de querol": "LA_TOR_QUEROL",
    "l hospitalet de llobregat": "HOSPITALET",
    "mananet massanes": "MACANET",
    "sant vicenc de calders": "ST_VICENC",
    "sant vicenc de castellet": "ST_VICENC_CAST",
    "l arboc": "L_ARBOC",
    "vilafranca del penedes": "VILAFRANCA",
    "la granada": "LA_GRANADA_PENEDES",
    "sant sadurni d anoia": "ST_SADURNI",
    "martorell central": "MARTORELL",
    "sant feliu de llobregat": "ST_FELIU",
    "sant joan despi": "ST_JOAN_DESPI",
    "cornella": "CORNELLA",
    "montcada i reixac manresa": "MONTCADA_REIXAC_MANRESA",
    "montcada i reixac santa maria": "MONTCADA_REIXAC_STA_MARIA",
    "montcada i reixac": "MONTCADA_REIXAC",
    "montcada bifurcacio": "MONTCADA_BIF",
    "montcada ripollet": "MONTCADA_RIPOLLET",
    "cerdanyola del valles": "CERDANYOLA_V",
    "cerdanyola universitat": "CEDANYOLA_U",
    "barbera del valles": "BARBERA_V",
    "sabadell sud": "SABADELL_SUD",
    "sabadell centre": "SABADELL_CEN",
    "sabadell nord": "SABADELL_NORD",
    "terrassa est": "TERRASSA_EST",
    "terrassa estacio del nord": "TERRASSA",
    "sant miquel de gonteres": "ST_MIQUEL_GONTERES",
    "vacarisses torreblanca": "VACARISSES_TORREBLANCA",
    "castellbell i el vilar monistrol de montserrat": "CASTELLBELL_MONISTROL",
    "rubi can vallhonrat": "RUBI_CAN_VALLHONRAT",
    "sant cugat coll favа": "ST_CUGAT_COLL",
    "sant cugat coll fava": "ST_CUGAT_COLL",
    "cabrera de mar vilassar de mar": "CABRERA_VILASSAR",
    "sant andreu de llavaneres": "ST_ANDREU_LLAVANERES",
    "caldes d estrac": "CALDES_ESTRAC",
    "sant pol de mar": "ST_POL_MAR",
    "santa susanna": "SANTA_SUSANNA",
    "malgrat de mar": "MALGRAT_MAR",
    "vilanova i la geltru": "VILANOVA",
    "segur de calafell": "SEGUR_CALAFELL",
    "platja de castelldefels": "PLATJA_CASTELLDEFELS",
    "sant adria de besos": "ST_ADRIA",
    "premia de mar": "PREMIA_MAR",
    "vilassar de mar": "VILASSAR_MAR",
    "el prat de llobregat": "EL_PRAT",
    "bellvitge gornal": "BELLVITGE_GORNAL",
    "el masnou": "EL_MASNOU",
}


def construir_indice_estaciones():
    """id -> nombre normalizado (y expandido) para cada estacion real, mas el
    diccionario de overrides ya normalizado, listo para resolver nombres del PDF."""
    est = json.load(open(BASE + "datos/estaciones.json", encoding="utf-8"))
    por_nombre = {}
    for e in est:
        for clave in (normalizar(e["nombre"]), normalizar_expandido(e["nombre"])):
            por_nombre[clave] = e["id"]
    overrides = {normalizar_expandido(k): v for k, v in OVERRIDES_ESTACION.items()}
    return por_nombre, overrides, {e["id"] for e in est}


RELLENO = {"de", "del", "la", "les", "el", "i", "els"}


def _sin_relleno(clave):
    return " ".join(p for p in clave.split(" ") if p not in RELLENO)


def resolver_estacion(nombre_pdf, por_nombre, overrides, ids_validos):
    clave = normalizar_expandido(nombre_pdf)
    if clave in overrides:
        return overrides[clave]
    if clave in por_nombre:
        return por_nombre[clave]
    # Ultimo recurso: buscar por inclusion de substring en ambos sentidos,
    # ignorando palabras de relleno (de/del/la/els...) -- el mismo PDF a
    # veces omite alguna en la cabecera segun la pagina (visto en R2:
    # "Estaci� de Fran�a" en una direccion, "Estaci� Fran�a" en la otra).
    clave_sr = _sin_relleno(clave)
    candidatos = [i for n, i in por_nombre.items() if clave in n or n in clave]
    if not candidatos:
        candidatos = [i for n, i in por_nombre.items()
                      if clave_sr and (clave_sr in _sin_relleno(n) or _sin_relleno(n) in clave_sr)]
    if not candidatos:
        candidatos = [i for k, i in overrides.items()
                      if clave_sr and (clave_sr in _sin_relleno(k) or _sin_relleno(k) in clave_sr)]
    candidatos = [i for i in candidatos if i in ids_validos]
    if len(set(candidatos)) == 1:
        return candidatos[0]
    return None


def agrupar_palabras_diagonales(chars):
    """Trocea una lista de caracteres ROTADOS (mismo orden que el stream del
    PDF, que ya coincide con el orden de lectura) en palabras, cortando
    cuando el salto de posicion entre caracteres consecutivos es grande."""
    palabras = []
    actual = []
    anterior = None
    for c in chars:
        if anterior is not None:
            dx = c["x0"] - anterior["x0"]
            dy = c["top"] - anterior["top"]
            if math.hypot(dx, dy) > UMBRAL_CORTE_PALABRA:
                if actual:
                    palabras.append(actual)
                actual = []
        actual.append(c)
        anterior = c
    if actual:
        palabras.append(actual)
    return palabras


def extraer_cabecera(chars_rotados_bloque):
    """A partir de los caracteres rotados de UN bloque (cabecera diagonal),
    devuelve lista de (texto, x0) por estacion, en orden de columna (por x0)."""
    palabras = agrupar_palabras_diagonales(chars_rotados_bloque)
    resultado = []
    for w in palabras:
        texto = "".join(c["text"] for c in w).strip()
        if texto:
            resultado.append((texto, w[0]["x0"], w[0]["top"]))
    return resultado


# Palabras clave para detectar el calendario de un bloque. Cuando dos
# subtitulos bilingues casi solapados (mismo 'top') se funden en una sola
# "palabra" ilegible (visto en R1.pdf: "DissDaibstseas,b..."), la version
# CATALANA/CASTELLANA se corrompe pero la INGLESA suele sobrevivir intacta
# -- por eso se buscan varias palabras clave y con "in", no solo "startswith".
CLAVES_FEINERS = ("feiners", "laborables", "weekdays")
CLAVES_FESTIUS = ("dissabte", "diumenge", "festiu", "saturday", "sunday", "holiday")


def _es_marcador_calendario(texto_normalizado):
    if any(c in texto_normalizado for c in CLAVES_FESTIUS):
        return "festius"
    if any(c in texto_normalizado for c in CLAVES_FEINERS):
        return "feiners"
    return None


def extraer_bloques_pagina(page, por_nombre, overrides, ids_validos):
    """Devuelve una lista de bloques de UNA pagina. Cada bloque es una tabla
    completa (una direccion + un calendario): {"calendario", "columnas":
    [(id_estacion, x0)...], "filas": [{id_estacion: "HH.MM"}, ...]}."""
    chars = page.chars
    rotados = [c for c in chars if abs(c["matrix"][1]) > 0.01 or abs(c["matrix"][2]) > 0.01]
    if not rotados:
        return []

    # 1) Agrupar los caracteres rotados en cabeceras (bloques separados por
    # un salto grande en 'top' -- las cabeceras de dos tablas distintas en
    # la misma pagina estan a cientos de puntos de distancia).
    grupos_cabecera = []
    actual = [rotados[0]]
    for c in rotados[1:]:
        if abs(c["top"] - actual[-1]["top"]) > 200:
            grupos_cabecera.append(actual)
            actual = []
        actual.append(c)
    grupos_cabecera.append(actual)

    # 2) Palabras -> id de estacion, fusionando fragmentos consecutivos que
    # resuelven al mismo id (nombres largos partidos en varias palabras,
    # p.ej. "La Tor" + "de Querol-" + "Enveig" -> LA_TOR_QUEROL).
    cabeceras = []
    for grupo in grupos_cabecera:
        palabras = extraer_cabecera(grupo)
        # Alguna etiqueta destacada (p.ej. "Vic", estacion de enlace con
        # otra linea) se dibuja con una fuente/orden distinto en el stream
        # del PDF y aparece descolocada en el orden de lectura -- hay que
        # reordenar por posicion real (x0) antes de fusionar duplicados.
        palabras = sorted(palabras, key=lambda p: p[1])
        columnas = []
        for texto, x0, top in palabras:
            sid = resolver_estacion(texto, por_nombre, overrides, ids_validos)
            if sid is None:
                continue
            if columnas and columnas[-1][0] == sid:
                continue
            columnas.append((sid, x0))
        top_medio = sum(c["top"] for c in grupo) / len(grupo)
        cabeceras.append({"columnas": columnas, "top": top_medio})

    if not cabeceras:
        return []

    # 3) Marcadores de calendario (palabras "Feiners..." / "Dissabtes...").
    marcadores = []
    for w in page.extract_words():
        kind = _es_marcador_calendario(normalizar(w["text"]))
        if kind:
            marcadores.append((w["top"], kind))
    marcadores.sort()

    def calendario_de(top_cabecera):
        anteriores = [k for t, k in marcadores if t < top_cabecera]
        return anteriores[-1] if anteriores else "feiners"

    # 4) Caracteres de hora (digitos y punto, SIN rotar) de toda la pagina.
    # OJO: NO se puede usar page.extract_words() aqui -- en tablas con muchas
    # columnas (R4, 40 estaciones) el hueco entre dos valores consecutivos de
    # 2 digitos ("10.0010.03...") es mas pequeno que el margen que pdfplumber
    # exige para cortar palabra, y varios valores quedan pegados en una sola
    # "palabra" ilegible que el regex de hora no reconoce -- silenciosamente
    # se pierden TODOS los servicios a partir de las 10:00 (bug real, visto
    # con R4: la extraccion se cortaba en seco a las 09:58). Trabajando
    # caracter a caracter y asignando cada uno a su columna por posicion (ver
    # mas abajo) no depende de que pdfplumber acierte el corte de palabra.
    chars_hora = [c for c in page.chars
                  if not (abs(c["matrix"][1]) > 0.01 or abs(c["matrix"][2]) > 0.01)
                  and c["text"] in "0123456789."]

    bloques = []
    for i, cab in enumerate(cabeceras):
        top_min = cab["top"]
        top_max = cabeceras[i + 1]["top"] if i + 1 < len(cabeceras) else float("inf")
        columnas = cab["columnas"]
        if len(columnas) < 2:
            continue
        xs = [x0 for _, x0 in columnas]
        ids_col = [sid for sid, _ in columnas]

        chars_bloque = sorted((c for c in chars_hora if top_min < c["top"] < top_max),
                               key=lambda c: c["top"])

        # Agrupar por 'top' con un clustering por huecos, no por redondeo:
        # dentro de una misma fila el 'top' es identico entre columnas (se
        # ha comprobado en varias tablas), pero el salto real a la fila
        # siguiente es de ~12-14 unidades -- un umbral de 3 separa filas
        # limpiamente sin depender de que el redondeo caiga bien.
        UMBRAL_FILA = 3.0
        grupos_fila = []
        for c in chars_bloque:
            if grupos_fila and c["top"] - grupos_fila[-1][-1]["top"] <= UMBRAL_FILA:
                grupos_fila[-1].append(c)
            else:
                grupos_fila.append([c])

        filas = []
        for grupo in grupos_fila:
            # Reconstruir la fila entera como una cadena (en orden de x0) y
            # buscar cada valor "H.MM"/"HH.MM" completo con una regex, en vez
            # de asignar caracter a caracter a la columna mas cercana: el
            # ultimo digito de un valor cae a menudo PRACTICAMENTE en el
            # limite entre dos columnas (medio ancho de columna ~11, un valor
            # de 4 caracteres ya ocupa ~12), así que ir caracter a caracter
            # descolocaba un digito de cada pocos valores hacia la columna
            # siguiente -- se detecto con R4 (40 columnas, limites muy
            # ajustados) pero es un riesgo general. La regex, al exigir el
            # patron completo (incluido el punto), encuentra el corte real
            # entre valores sin depender de la geometria de columnas, tanto
            # si habia hueco real en el PDF como si dos valores quedaron
            # pegados sin espacio (ver comentario de chars_hora mas arriba).
            grupo_ordenado = sorted(grupo, key=lambda c: c["x0"])
            texto_fila = "".join(c["text"] for c in grupo_ordenado)
            x0_por_pos = [c["x0"] for c in grupo_ordenado]
            fila = {}
            for m in RE_HORA_INTERNA.finditer(texto_fila):
                valor = m.group(0)
                # Una "hora" >23 no existe (los PDF usan wraparound tras
                # medianoche, p.ej. "0.20", nunca "24.20") -- si aparece, es
                # ruido de la pagina (pie de pagina, numeros de telefono...)
                # que por casualidad encaja en el patron H.MM.
                if int(valor.split(".")[0]) > 23:
                    continue
                x0_valor = x0_por_pos[m.start()]
                idx = min(range(len(xs)), key=lambda k: abs(xs[k] - x0_valor))
                fila[ids_col[idx]] = valor
            filas.append(fila)
        bloques.append({
            "calendario": calendario_de(top_min),
            "columnas": ids_col,
            "filas": filas,
        })
    return bloques


def hma_a_hhmmss(texto_hma):
    """'4.24' o '13.5' -> '04:24:00'. Los PDF usan 'H.MM' (minutos siempre a
    2 digitos, hora sin cero a la izquierda)."""
    h, m = texto_hma.split(".")
    return "%02d:%02d:00" % (int(h), int(m))


def _hhmmss_a_min(hhmmss):
    h, m, _s = hhmmss.split(":")
    return int(h) * 60 + int(m)


def _paradas_coherentes(paradas_ordenadas_hhmmss):
    """Descarta filas con un valor fuera de orden cronologico (columna mal
    asignada por ruido puntual del PDF de origen) -- se permite UN unico
    salto hacia atras al final de la fila (parada tras medianoche)."""
    minutos = [_hhmmss_a_min(h) for _, h in paradas_ordenadas_hhmmss]
    saltos_atras = 0
    for i in range(1, len(minutos)):
        if minutos[i] < minutos[i - 1]:
            es_medianoche = minutos[i] < 60 and minutos[i - 1] > 22 * 60
            if not es_medianoche:
                return False
            saltos_atras += 1
    return saltos_atras <= 1


def bloque_a_servicios(bloque, id_linea):
    """Convierte un bloque (columnas + filas H.MM) en una lista de servicios
    {"linea","origen","destino","salida","llegada","paradas"}. El origen y
    destino de CADA fila son su primera/ultima parada realmente marcada (no
    los extremos del bloque -- una fila puede ser un servicio corto)."""
    servicios = []
    descartadas_por_orden = 0
    orden_columnas = bloque["columnas"]
    for fila in bloque["filas"]:
        paradas_ordenadas = [(sid, fila[sid]) for sid in orden_columnas if sid in fila]
        if len(paradas_ordenadas) < 2:
            continue
        origen_id, salida = paradas_ordenadas[0]
        destino_id, llegada = paradas_ordenadas[-1]
        if origen_id == destino_id:
            continue
        paradas_hhmmss = [(sid, hma_a_hhmmss(h)) for sid, h in paradas_ordenadas]
        if not _paradas_coherentes(paradas_hhmmss):
            descartadas_por_orden += 1
            continue
        servicios.append({
            "linea": id_linea,
            "origen": origen_id,
            "destino": destino_id,
            "salida": paradas_hhmmss[0][1],
            "llegada": paradas_hhmmss[-1][1],
            "paradas": dict(paradas_hhmmss),
        })
    return servicios, descartadas_por_orden


# R2.pdf trae UNA sola tabla combinada (Ma�anet-Massanes <-> Sant Vicen� de
# Calders, 34 estaciones) que mezcla trenes de las 3 lineas que el simulador
# ya distingue (R2, R2_NORD, R2_SUD -- ver datos/lineas.json). Para saber a
# cual pertenece cada tren real (fila) usamos la propia etiqueta "lineas" de
# cada estacion en datos/estaciones.json: una linea X es candidata para un
# servicio solo si TODAS sus paradas estan etiquetadas con X. Si la
# interseccion da una sola linea, resuelto; si da 0 (el tren toca estaciones
# exclusivas de dos ramas a la vez, p.ej. Aeroport Y Estaci� de Fran�a) es un
# recorrido de punta a punta que no cabe en el modelo actual de 3 lineas
# separadas -- se deja aparte en "R2_SIN_CLASIFICAR" para decidir con la autora.
LINEAS_R2 = {"R2", "R2_NORD", "R2_SUD"}


def _construir_tags_estacion():
    est = json.load(open(BASE + "datos/estaciones.json", encoding="utf-8"))
    return {e["id"]: set(e.get("lineas", [])) for e in est}


def clasificar_servicio_r2(servicio, tags_estacion):
    interseccion = None
    for sid in servicio["paradas"]:
        candidatas = tags_estacion.get(sid, set()) & LINEAS_R2
        interseccion = candidatas if interseccion is None else (interseccion & candidatas)
    if interseccion and len(interseccion) == 1:
        return next(iter(interseccion))
    return None


# Estaciones que delatan un servicio de R4 operando en realidad como
# lanzadera de la R7 (Barcelona-Fabra i Puig <-> Cerdanyola Universitat).
ESTACIONES_LANZADERA_R7 = {"CEDANYOLA_U", "CERDANYOLA_V"}


def procesar_linea(codigo_pdf, tags_estacion):
    """Procesa un PDF completo (todas sus paginas) y devuelve la lista de
    servicios en dias laborables, ya con el id de linea del simulador
    correcto (puede diferir de codigo_pdf, ver R2)."""
    por_nombre, overrides, ids_validos = INDICE
    servicios = []
    avisos = []
    with pdfplumber.open(BASE + codigo_pdf + ".pdf") as pdf:
        for pagina in pdf.pages:
            bloques = extraer_bloques_pagina(pagina, por_nombre, overrides, ids_validos)
            for b in bloques:
                if b["calendario"] != "feiners":
                    continue
                nuevos, descartadas = bloque_a_servicios(b, codigo_pdf)
                if descartadas:
                    avisos.append(f"{codigo_pdf}: {descartadas} filas descartadas "
                                   f"(hora fuera de orden cronologico, dato de origen ruidoso)")
                if codigo_pdf == "R2":
                    reclasificados = []
                    sin_clasificar = 0
                    for s in nuevos:
                        linea_real = clasificar_servicio_r2(s, tags_estacion)
                        if linea_real is None:
                            s["linea"] = "R2_SIN_CLASIFICAR"
                            sin_clasificar += 1
                        else:
                            s["linea"] = linea_real
                        reclasificados.append(s)
                    nuevos = reclasificados
                    if sin_clasificar:
                        avisos.append(f"R2: {sin_clasificar} servicios de punta a punta "
                                       f"sin encaje en R2/R2_NORD/R2_SUD")
                if codigo_pdf == "R4":
                    antes = len(nuevos)
                    nuevos = [s for s in nuevos
                              if s["origen"] not in ESTACIONES_LANZADERA_R7
                              and s["destino"] not in ESTACIONES_LANZADERA_R7]
                    if antes != len(nuevos):
                        avisos.append(f"R4: descartados {antes - len(nuevos)} servicios "
                                       f"lanzadera-R7 (Cerdanyola)")
                servicios.extend(nuevos)
    return servicios, avisos


if __name__ == "__main__":
    INDICE = construir_indice_estaciones()
    por_nombre, overrides, ids_validos = INDICE
    print("Indice de estaciones construido:", len(ids_validos), "estaciones")

    tags_estacion = _construir_tags_estacion()
    todos = {}
    for codigo in LINEAS_PDF:
        servicios, avisos = procesar_linea(codigo, tags_estacion)
        for a in avisos:
            print("  [aviso]", a)
        for s in servicios:
            todos.setdefault(s["linea"], []).append(s)
        print(f"{codigo}.pdf -> {len(servicios)} servicios laborables extraidos")

    # Decision de la autora: los servicios de R2 de punta a punta que no
    # encajan en ninguna de las 3 lineas (R2/R2_NORD/R2_SUD) del simulador
    # se descartan (ver conversacion -- 15 de 770, no se parten ni se
    # inventa una 4a linea).
    descartados_r2 = len(todos.pop("R2_SIN_CLASIFICAR", []))
    if descartados_r2:
        print(f"\nDescartados {descartados_r2} servicios R2 de punta a punta sin linea de encaje.")

    for id_linea, lst in todos.items():
        lst.sort(key=lambda s: s["salida"])

    resumen = {id_linea: len(lst) for id_linea, lst in todos.items()}
    print("\nResumen final por linea del simulador:", json.dumps(resumen, indent=2))

    out_path = BASE + "horarios_oficiales.json"
    with open(out_path, "w", encoding="utf-8") as f:
        json.dump(todos, f, indent=2, ensure_ascii=False)
    print("Escrito:", out_path)

    print("\n--- Muestra de 3 servicios de R3 ---")
    for s in todos.get("R3", [])[:3]:
        print(json.dumps(s, indent=2, ensure_ascii=False))
