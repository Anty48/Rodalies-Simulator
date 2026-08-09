#!/usr/bin/env bash
set -euo pipefail
cd "C:/Projects/Rod4lia 2.0"
SRC=raw/fomento_transit
OUT=data/gtfs
TMP="$(mktemp -d)"
mkdir -p "$OUT"

echo "[1/4] routes -> Rodalies (short_name ^R)"
awk -F',' 'NR==1{print; next} {s=$2; gsub(/[ \t\r]/,"",s); if (s ~ /^R/) print}' "$SRC/routes.txt" > "$OUT/routes.txt"
awk -F',' 'NR>1{r=$1; gsub(/[ \t\r]/,"",r); print r}' "$OUT/routes.txt" | sort -u > "$TMP/routeids.txt"
echo "    routes kept: $(( $(wc -l < "$OUT/routes.txt") - 1 ))"

echo "[2/4] trips filtered by route_id"
awk -F',' 'NR==FNR{set[$1]=1; next} FNR==1{print; next} {r=$1; gsub(/[ \t\r]/,"",r); if (r in set) print}' "$TMP/routeids.txt" "$SRC/trips.txt" > "$OUT/trips.txt"
awk -F',' 'NR>1{t=$3; gsub(/[ \t\r]/,"",t); print t}' "$OUT/trips.txt" | sort -u > "$TMP/tripids.txt"
echo "    trips kept: $(( $(wc -l < "$OUT/trips.txt") - 1 ))"

echo "[3/4] stop_times filtered by trip_id (large pass)"
awk -F',' 'NR==FNR{set[$1]=1; next} FNR==1{print; next} {t=$1; gsub(/[ \t\r]/,"",t); if (t in set) print}' "$TMP/tripids.txt" "$SRC/stop_times.txt" > "$OUT/stop_times.txt"
awk -F',' 'NR>1{s=$4; gsub(/[ \t\r]/,"",s); print s}' "$OUT/stop_times.txt" | sort -u > "$TMP/stopids.txt"
echo "    stop_times kept: $(( $(wc -l < "$OUT/stop_times.txt") - 1 ))"

echo "[4/4] stops filtered by stop_id"
awk -F',' 'NR==FNR{set[$1]=1; next} FNR==1{print; next} {s=$1; gsub(/[ \t\r]/,"",s); if (s in set) print}' "$TMP/stopids.txt" "$SRC/stops.txt" > "$OUT/stops.txt"
echo "    stops kept: $(( $(wc -l < "$OUT/stops.txt") - 1 ))"

echo "DONE. Sizes:"
ls -la "$OUT"
rm -rf "$TMP"
