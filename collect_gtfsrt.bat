@echo off
REM ============================================================
REM  rodalies-sim  ·  recol.lector de retards REALS (GTFS-RT Renfe)
REM  Deixa'l obert hores o dies: grava raw\gtfsrt\AAAA-MM-DD.jsonl.
REM  Despres:  python scripts\analyze_gtfsrt.py
REM            cargo run --release -- compare
REM ============================================================
setlocal
cd /d "%~dp0"
echo Gravant posicions i retards de Rodalies cada 20 s (Ctrl+C per aturar)...
python scripts\collect_gtfsrt.py --every 20
pause
