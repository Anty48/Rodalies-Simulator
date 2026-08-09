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
- Run the simulator: `cargo run --release`
- Tests: `cargo test` · single test: `cargo test dwell_grows_with_delay`
  (unit tests live in `#[cfg(test)]` modules inside `src/passenger_model.rs`)

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

- **`main.rs`** — `#[tokio::main]`. Checks `./data/gtfs`, loads (timed in ms), prints the
  network summary + a detailed example route (train `25412` if present, else an equivalent),
  runs a 2h CTC simulation (07:00–09:00) with injected incidents, then a **rayon** parallel
  resilience sweep (varying block duration, stochastic passengers) — this is why `rayon`,
  `rand` and `tokio` are dependencies.

## Conventions

- Console output and code comments are in Catalan/Spanish; keep that voice.
- Keep `cargo check` clean: the model structs' fields are deliberately surfaced in reports so
  they aren't dead code — if you add a field, actually use it (in a report or the engine)
  rather than silencing the warning.
