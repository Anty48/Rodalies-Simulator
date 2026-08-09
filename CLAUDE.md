# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Toolchain (Windows quirk)

Rust was installed via rustup but `cargo`/`rustc` are **not on the shell PATH by default**.
Prepend the cargo bin dir before every invocation (PowerShell):

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"; & cargo <cmd>
```

## Commands

- Build / typecheck: `cargo check` (must stay warning-free) · `cargo build --release`
- Run: `cargo run --release` prints a console summary, writes `report/dashboard.html`, and
  starts an interactive web server on http://127.0.0.1:8080 (blocks until Ctrl+C).
  `-- --static` = write+open the offline HTML and exit; `-- --no-open` = don't open a browser
  (use for CI/tests). `run.bat` is the double-click launcher.
- Optimize timetables: `cargo run --release -- optimize [R1 R4 …]` runs simulated annealing
  and writes `report/optimized/R*_optimized.csv` + a console comparison. No line args = top-4.
- Tests: `cargo test` · single test: `cargo test dwell_grows_with_delay`
  (unit tests live in `#[cfg(test)]` modules inside `src/passenger_model.rs`)
- Manual server check: run with `--no-open`, then `curl http://127.0.0.1:8080/api/render?block=12&delay=8`

## Data prerequisite

The binary reads `stops.txt`, `routes.txt`, `trips.txt`, `stop_times.txt` from `./data/gtfs`
and **exits if the folder or any file is missing**. `data/` and `fomento_transit/` are
gitignored (external data; the national feed and other raw sources live under `./raw/`).
To (re)generate `./data/gtfs`, place the national GTFS feed in `./raw/fomento_transit/` and
run `bash scripts/prep_gtfs.sh` — it filters the feed to Rodalies de Catalunya
(`route_short_name` starting with `R`).

## Architecture

Discrete-event railway simulator whose entire network is built **dynamically from GTFS** at
runtime — there is no hardcoded map. Data flows in one direction: GTFS → `Network` → `Simulator`.

- **`gtfs_loader.rs`** — parses the four GTFS files into a `Network`:
  - `petgraph::DiGraph` where each stop is a node (`StopNode`) and each pair of consecutive
    stops in a trip is a directed edge (`TrackEdge` = a *cantón*/block) weighted with the
    nominal run time (`arrival_next − departure_current`). Edges are deduped, keeping the min run time.
  - `TrainService` per trip with an ordered `schedule`. `train_number` comes from
    `trip_short_name` **with fallback to `trip_id`** (the real Renfe feed lacks that column).
  - Real feed quirks handled here: fixed-width space padding (read with `csv::Trim::All`) and
    optional/missing columns (`parent_station`, `trip_short_name`) via `#[serde(default)]`.
  - Helpers used by the sim/reports: `dominant_service()`, `busiest_segment()`,
    `find_stop_by_name()`, `edge_between()`, `parse_gtfs_time()`/`fmt_hms()` (seconds-since-midnight).

- **`passenger_model.rs`** — dwell time. Key invariant: **extra dwell is 0 when arrival
  delay is 0** (a punctual train runs to timetable), so the "return to equilibrium" metric is
  meaningful. Extra passengers ∝ delay, and boarding time grows **exponentially** with delay
  (`e^(k·delay_min)`). `dwell_time` (deterministic) and `dwell_time_random` (Monte Carlo via `rand`).

- **`simulation_engine.rs`** — `Simulator::run(service_id)` drives a `BinaryHeap` priority
  queue of `Arrive`/`Depart` events over a time window, for trains of one `service_id`.
  - **Signalling**: effective block capacity = `max(1, run_secs / min_block_headway_secs)`
    (models multiple signal blocks per section, so the Barcelona trunk doesn't gridlock).
    A train that can't enter an occupied block or a full platform is re-queued every
    `RETRY_STEP` seconds, accumulating delay — this is how knock-on delay propagates.
  - **Incidents** (`Incident` enum): `TrainDelay` (by train number + station name),
    `BlockSegment` (by station name) and `BlockSegmentById` (by stop_id, exact).
  - **Stability metric**: samples network total/mean delay each minute (`Sample`), reports
    peak accumulated delay and the recovery time back below the equilibrium threshold.

- **`topology.rs`** — operational data GTFS lacks: `platform_tracks(name)` (real track counts,
  Sants 14…, default 2) and `single_track_pairs(net)` (undirected segments that are single-track;
  configured in `SINGLE_TRACK`, e.g. R3 north of Montcada Bifurcació). Derived from public/Godot
  reference data. Buses are `TrainService.is_bus` (GTFS `route_type==3`).

- **`signaling.rs`** — pure three-aspect block logic (`Aspect` Green/Yellow/Red, `Signals`).
  Green = nominal; Yellow (destination platform occupied) = run time × `yellow_slowdown`; Red
  (canton occupied) = the engine holds the train. The engine calls it when `strict_signaling`
  is on (capacity 1 per canton and platform).

- **`optimizer/`** — timetable optimization. `potential.rs`: `V(H)` = regularity (peak-weighted
  headway std) + passenger-weighted delay (timeline integral) + conflict penalty (`held_events`).
  `search.rs`: `optimize_line` runs simulated annealing over per-trip departure offsets (±5 min);
  each candidate is scored by the mean `V` over a FIXED set of Monte-Carlo incidents evaluated in
  parallel with rayon, using the engine in `strict_signaling` + `line_filter` + `offsets` mode.
  `mod.rs::optimize_lines` parallelizes across lines. Conflicts count distinct red stops (a
  `waiting` flag on `TrainRt`), not every 10s retry.

- **`exporter.rs`** — writes `report/optimized/<LINE>_optimized.csv`, a per-line **PDF**
  timetable (`<LINE>_horari.pdf`, printpdf; stations×trains grid, paginated), and the console
  comparison. Strict rail sim in the optimizer sets `strict_signaling` + `exclude_buses` +
  `single_track` (from `topology`) and per-station platform capacity from `topology`.

- **`map.rs`** — `network_map_svg` projects stations from lat/lon (equirectangular, cos-lat
  corrected), draws cantons, and animates a sample of trains along their real routes with SMIL
  `animateMotion` (keyTimes from the schedule) — no JS, works in the static file. Returned via
  `SimView.map_svg`.

- **`scenario.rs`** — the single source of truth for turning parameters (`SimParams`:
  window, line filter, block/delay minutes, platform capacity, min block headway, stochastic
  toggle) into `report::*View` structs, with **no console output**. Used by both `main` (console
  + static dashboard) and `server` (per-request). `res_view` runs the resilience sweep with
  **rayon**. Also holds `now_utc_string` (date without external crates).

- **`report.rs`** — renders the dashboard from `*View` structs. `render_body` = the sections
  (inline CSS + Rust-generated **SVG** chart; no external assets). `render_html` wraps it for
  the offline file; `render_interactive_page` adds the controls form + a small vanilla-JS
  `<script>` that `fetch`es `/api/render` and swaps `#dashboard`. CSS/JS live in `const`s so
  `format!` never has to brace-escape them. `SimView` reuses the engine's `CtcEvent`. If you
  add a field to a `*View`, render it (otherwise dead-code warning).

- **`server.rs`** — minimal tokio HTTP/1.1 server (GET only, `Connection: close`, localhost).
  Routes: `/`, `/api/render?…`, `/health`, plus the optimizer: `/api/optimize/start?line=` (spawns
  a std::thread running `optimize_line_cb`, progress written to `Arc<Mutex<OptJob>>`),
  `/api/optimize/status` (live JSON: iter/total, base_v, best_v, history…), and
  `/report/optimized/<file>` (serves generated CSV/PDF; response body is `Vec<u8>` for binaries).
  The web `OPT_SCRIPT` polls status and draws the descending V(H) curve on a canvas.

- **`main.rs`** — `#[tokio::main]`. Checks `./data/gtfs`, loads (timed in ms), builds the
  default `*View`s via `scenario`, prints them, writes the static dashboard, then either
  exits (`--static`) or serves the interactive UI. This is why `rayon`, `rand` and `tokio`
  are all real dependencies.

## Conventions

- Console output and code comments are in Catalan/Spanish; keep that voice.
- Keep `cargo check` clean: the model structs' fields are deliberately surfaced in reports so
  they aren't dead code — if you add a field, actually use it (in a report or the engine)
  rather than silencing the warning.
