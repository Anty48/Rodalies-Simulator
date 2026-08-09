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

echo Compilant i executant rodalies-sim (release)...
echo.
cargo run --release
if errorlevel 1 (
  echo.
  echo [ERROR] La compilacio o l'execucio ha fallat.
  pause
  exit /b 1
)

echo.
echo Fet. El dashboard s'hauria d'haver obert al navegador.
echo (si no, obre  report\dashboard.html)
pause
