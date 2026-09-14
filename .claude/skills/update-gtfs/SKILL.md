---
name: update-gtfs
description: Regenerar ./data/gtfs filtrando el feed nacional Fomento/Renfe a Rodalies de Catalunya (líneas R*). Úsala cuando falte data/gtfs, el usuario pida actualizar el GTFS, o haya un feed nuevo en ./raw.
---

# Regenerar los datos GTFS de Rodalies

El binario lee `stops.txt`, `routes.txt`, `trips.txt`, `stop_times.txt` de `./data/gtfs` y **sale
si falta la carpeta o cualquier fichero**. `data/` y el feed crudo están gitignored (datos
externos; `stop_times.txt` filtrado ~71 MB). Ver [[rodalies-sim-project]] y [[renfe-gtfs-source]].

## Opción A — filtrar un feed ya descargado (offline)

1. Verifica que el feed nacional está en `./raw/fomento_transit/` (los 4 CSV base).
2. Filtra a Rodalies (`route_short_name` que empieza por `R`):
   ```bash
   bash scripts/prep_gtfs.sh
   ```
3. Comprueba el resultado:
   ```bash
   ls -la data/gtfs/
   wc -l data/gtfs/*.txt
   ```
   Referencia sana histórica: ~204 estaciones, ~18 líneas, ~760 trenes/día laborable.

## Opción B — actualizar al feed de hoy desde la app (en caliente)

El servidor tiene un botón **"Actualizar GTFS"** en `/game` que descarga
`https://ssl.renfe.com/ftransit/Fichero_CER_FOMENTO/fomento_transit.zip`, lo filtra en Rust
(`src/gtfs_update.rs`, equivalente a `prep_gtfs.sh`) y **recarga el servidor sin reiniciar**.
Endpoints: `/api/gtfs/update`, `/api/gtfs/status` (polling), `/api/gtfs/upload` (subida manual de
un .zip como respaldo). Sugiere esta opción al usuario si el servidor ya está corriendo.

## Notas del feed real (no romper)
- Campos con relleno de espacios (ancho fijo) → leer con `csv::Trim::All`.
- No trae `trip_short_name` (el nº de tren cae a `trip_id`) ni `parent_station` (queda `None`).
- Tras regenerar, valida con la skill `verify-game`.
