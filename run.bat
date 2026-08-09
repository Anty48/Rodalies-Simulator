@echo off
REM ============================================================
REM  rodalies-sim  ·  llançador (doble clic per executar)
REM  Compila, executa la simulacio i obre el dashboard HTML.
REM ============================================================
setlocal
cd /d "%~dp0"

REM Afegeix cargo al PATH si no hi es (instal.lacio estandard de rustup)
where cargo >nul 2>nul || set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"

where cargo >nul 2>nul
if errorlevel 1 (
  echo [ERROR] No trobo 'cargo'. Instal.la Rust des de https://rustup.rs
  echo         o executa:  winget install Rustlang.Rustup
  pause
  exit /b 1
)

echo Compilant i arrencant rodalies-sim (release)...
echo El navegador s'obrira amb la UI interactiva a http://127.0.0.1:8080
echo Prem Ctrl+C en aquesta finestra per aturar el servidor.
echo.
cargo run --release
if errorlevel 1 (
  echo.
  echo [ERROR] La compilacio o l'execucio ha fallat.
  pause
  exit /b 1
)
pause
