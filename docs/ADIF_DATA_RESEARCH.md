# Fase A/B — Investigación de datos ADIF (calculador · fase 3)

Informe de qué datos oficiales de infraestructura hemos encontrado, en qué formato, y
cómo se integran. **Fecha de consulta: 2026-08-30.**

## Resumen ejecutivo

Sí es posible obtener **programáticamente** datos oficiales de ADIF de **velocidad máxima
de la infraestructura** y **geometría real de la vía**, vía el **WFS INSPIRE de IDEADIF**.
Ya está integrado (ver `scripts/fetch_adif_cvm.py` → `processed/adif/rfig_speed.json`).

## Fuente principal (INTEGRADA)

- **Organismo**: ADIF — IDEADIF (Infraestructura de Datos Espaciales de Adif).
- **Dataset**: "Red de Transporte Ferroviario de Adif" (INSPIRE Transport Networks, Annex I).
  Catálogo: `datos.gob.es` id **e0dat0002**; metadato CSW `62c487b1-6816-4594-a800-218abe896994`.
- **Servicio**: WFS 2.0.0 — `https://ideadif.adif.es/services/wfs`
  (GetCapabilities: `?service=WFS&request=GetCapabilities`).
- **Versión del dato**: `versionId` = **2026/01** (namespace `es.adif.ideadif`).
- **CRS**: EPSG:4258 (ETRS89, lat/lon ≈ WGS84).
- **Formato**: GML 3.2 (el servidor **no** ofrece GeoJSON: `outputFormat=application/json`
  devuelve `InvalidParameterValue`). Se parsea el GML directamente.

### Capas (feature types) relevantes

| typeName | Contenido | Uso |
|---|---|---|
| `tn-ra:DesignSpeed` | **Velocidad máxima de diseño** por enlace: `<tn-ra:speed uom="km/h">…</tn-ra:speed>` + `networkRef` al enlace | **CVM** |
| `tn-ra:RailwayLink` | **Geometría real** de la vía (`gml:LineString`/`gml:posList`) + `localId` + start/end node | Perfil espacial y distancia |
| `tn-ra:RailwayNode` / `RailwayStationNode` / `RailwayStationCode` | Nodos y estaciones (código ADIF) | (futuro) PK y mapeo estación→red |
| `tn-ra:NumberOfTracks`, `RailwayElectrification`, `RailwayUse`, `RailwayType` | vías, electrificación, uso, tipo | (futuro) metadatos |

**Unión**: `DesignSpeed_XXXXX` ↔ `RailwayLink_XXXXX` por el **código numérico** del `localId`
(1:1). Tamaño nacional: **1689 enlaces** (1567 con velocidad + geometría tras la unión).

### Datos obtenidos (reales, verificados)

`processed/adif/rfig_speed.json`: **1567 enlaces**, **769.720 vértices**, ~18 MB.
Velocidades presentes (km/h): 15,20,30,40,50,60,70,80,90,100,110,120,130,140,155,160,200,220,240,300…
Ejemplo corredor Barcelona–Maresme: tronco 140/160 km/h, ramales/entradas 50/60 km/h. Es un CVM real.

## Lo que este dato SÍ da

- **Velocidad máxima de infraestructura por tramo** (design speed) → `Vmax(x)=min(Vmax_tren, Vmax_ADIF(x))`.
- **Geometría real de la vía** → distancia mejor que la polilínea de estaciones (por proyección).

## Lo que este dato NO da (y sigue faltando)

- **PK oficiales por estación**: la capa base no da PK tabulado por estación (habría que
  derivarlo proyectando estaciones sobre `RailwayLink` + medir a lo largo; pendiente).
- **Pendientes/rampas y radios de curva**: NO están en esta capa INSPIRE. Posible fuente:
  RINF (ver abajo) o la Declaración sobre la Red (PDF, no tabular). Módulo dejado preparado
  y **desactivado** (sin datos).
- **Restricciones temporales de velocidad** (obras): fuera de alcance.

## Fuentes secundarias investigadas (NO integradas)

- **RINF (ERA)** — `rinf.era.europa.eu` / linked-data `rinf.data.era.europa.eu`. Velocidad
  máxima por *section of line* (más gruesa que DesignSpeed) y a veces gradiente. El portal/API
  **requiere cuenta**; se descarta por ahora frente al WFS de IDEADIF, que es abierto y más fino.
- **Declaración sobre la Red (ADIF)** — mapas de velocidades máximas en **PDF interactivo**
  por capas: no tabular, no automatizable de forma fiable. Sirve como contraste manual.
- **Curvas de esfuerzo tractor / aceleración / frenado por serie (447/450/456/470/490)**:
  **no publicadas** por Renfe/fabricante en fuentes accesibles. Se mantiene el modelo
  (potencia constante + tope) y se añade resistencia Davis **estimada** (genérica, etiquetada).
- **Circulación observada real (hora real por tren)**: sin dataset abierto oficial (solo
  tiempo real efímero / terceros como treneamos). Se mantiene "No disponible".

## Cómo se integra (Fase C/D)

1. `scripts/fetch_adif_cvm.py` descarga `DesignSpeed`+`RailwayLink`, une por código y escribe
   `processed/adif/rfig_speed.json` (gitignored; no se versiona el dato grande).
2. `infrastructure.rs` carga ese JSON en un índice espacial (rejilla) y, para una ruta,
   asigna a cada punto la **velocidad del enlace ADIF más cercano** (tolerancia configurable),
   construyendo un **perfil de velocidades** `Vmax(x)` y una **distancia ADIF** (proyección),
   con **cobertura %** y procedencia. Si falta el fichero → *fallback* al modelo anterior.
3. `physics.rs` respeta `Vmax(x)=min(Vmax_tren, Vmax_ADIF(x))` con frenado anticipado
   (envolvente sobre todas las reducciones futuras).

## Procedencia (no camuflar incertidumbre)

- 🟢 **Oficial**: velocidad de diseño ADIF (DesignSpeed), geometría de vía (RailwayLink),
  horarios/paradas (GTFS).
- 🟡 **Secundaria**: material rodante 490 (Wikipedia).
- 🟠 **Estimada**: resistencia al avance (Davis genérico), aceleración/frenado del modelo.
- 🔴 **No disponible**: pendientes, curvas de tracción por serie, PK por estación, circulación real.
