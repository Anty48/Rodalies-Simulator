---
name: verify-game
description: Compilar el binario Rust, arrancar el servidor local y verificar el juego web (/game) en el navegador — consola sin errores, red y mecánicas de simulación. Úsala tras tocar game/web/*.js|css|html, src/game.rs o src/server.rs, o cuando el usuario pida "verificar el juego".
---

# Verificar el juego web (Rod4lia 2.0)

Flujo de verificación real de este proyecto (no hay tests JS; ver [[game-web-engine-internals]]).
La herramienta **Bash** ya tiene `cargo` en el PATH (`~/.cargo/bin`); **PowerShell no** — si usas
PowerShell, antepón `$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path";` (ver CLAUDE.md).

## Pasos

1. **Compilar** (debe quedar sin warnings):
   ```bash
   cargo check
   ```
   Para probar en runtime: `cargo build --release`.

2. **Prerrequisito de datos**: el binario sale si falta `./data/gtfs`
   (`stops/routes/trips/stop_times.txt`). Si no existe, avisa al usuario — no lo regeneres aquí
   (usa la skill `update-gtfs`).

3. **Arrancar el servidor sin abrir navegador**, en segundo plano:
   ```bash
   cargo run --release -- --no-open
   ```
   (`run_in_background: true`). Sirve en http://127.0.0.1:8080 y bloquea hasta Ctrl+C.

4. **Comprobar API** antes de tocar el navegador (compara contra los datos crudos si hay dudas):
   ```bash
   curl -s "http://127.0.0.1:8080/health"
   curl -s "http://127.0.0.1:8080/api/game/network" | head -c 400
   curl -s "http://127.0.0.1:8080/api/game/schedule?source=gtfs" | head -c 400
   ```

5. **Verificar el juego en el navegador** con el MCP de Playwright (servidor `playwright` en
   `.mcp.json`): navega a `http://127.0.0.1:8080/game`, revisa **errores de consola**, toma una
   captura, y ejercita lo que hayas cambiado.

6. **Regla de oro del motor** (ver [[feedback-simulation-engine-design]]): cualquier lógica nueva
   dependiente del tiempo se prueba **a ×25**, no solo a ×1 — es la velocidad donde afloran los
   bugs de "ventana de tiempo fija" del bucle de simulación.

7. **Parar el servidor** de fondo al terminar.

## Qué reportar
Compilación (warnings), estado de los endpoints, errores de consola del navegador, y si la
mecánica concreta que se tocó se comporta como debe a ×1 y ×25.
