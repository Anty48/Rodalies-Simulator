# Diagramas de vías — trenscat.com

Esquemas de vías ("sch-vies_*.gif") descargados de [trenscat.com](https://www.trenscat.com/RENFE/index.html)
(sesión 2026-09-10), organizados por `stop_id` del GTFS de Rodalies (una carpeta por estación).
`index.json` mapea cada `stop_id` a su nombre, la página de origen (`source_page`) y los
ficheros de imagen descargados (`images`).

**Cobertura actual: 100 de 210 estaciones de Rodalies** (las que tienen esquema de vías
publicado en trenscat.com y cuyo nombre se pudo emparejar automáticamente con el GTFS — quedan
~44 estaciones adicionales emparejadas sin diagrama publicado, y ~44 sin emparejar por
diferencias de nombre). Ampliable más adelante repitiendo el proceso de emparejamiento con
mejor normalización, o añadiendo enlaces manualmente.

**Uso previsto:** referencia visual para extraer manualmente la disposición real de vías,
agujas y apartaderos de cada estación (cantones) — los datos que el GTFS no aporta. Se sirven
en el juego web (clic en una estación con diagrama disponible) desde `/reference/trenscat/…`.

**Atribución y uso:** material con derechos de trenscat.com, conservado aquí como referencia
técnica interna para este proyecto (no redistribución pública). Cada estación en `index.json`
enlaza a su página de origen.
