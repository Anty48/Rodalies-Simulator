//! rodalies-sim — Simulador de tráfico ferroviario e incidencias para Rodalies de
//! Barcelona, construido 100% dinámicamente desde GTFS histórico (`./data/gtfs`).
//!
//! Modos:
//!   * por defecto: imprime el resumen en consola, escribe `report/dashboard.html`
//!     y arranca un servidor web interactivo en http://127.0.0.1:8080.
//!   * `--static`: solo escribe y abre el dashboard HTML (offline, sin servidor).
//!   * `--no-open`: no abre el navegador automáticamente.

mod cadence;
mod calculator;
mod cloud;
mod compare;
mod demand;
mod engine;
mod exporter;
mod freight;
mod game;
mod gtfs_loader;
mod gtfs_update;
mod map;
mod optimizer;
mod realism;
mod recovery;
mod report;
mod runs_export;
mod scenario;
mod server;
mod topology;
mod track_geometry;

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use gtfs_loader::{fmt_hms, Network};
use optimizer::PotentialWeights;
use report::{ExampleView, ResView, SimView, SummaryView};
use scenario::SimParams;

const GTFS_DIR: &str = "./data/gtfs";
const EXAMPLE_TRAIN: &str = "25412";
const PORT: u16 = 8080;

#[tokio::main]
async fn main() {
    println!("╔════════════════════════════════════════════════════════════════════╗");
    println!("║   RODALIES-SIM · Simulador de tràfic ferroviari (GTFS dinàmic)      ║");
    println!("╚════════════════════════════════════════════════════════════════════╝\n");

    // 1. Comprobar la carpeta GTFS.
    let dir = Path::new(GTFS_DIR);
    if !dir.is_dir() {
        eprintln!("✗ No trobo la carpeta GTFS a `{}`.", GTFS_DIR);
        eprintln!("  Col·loca-hi stops.txt, routes.txt, trips.txt i stop_times.txt.");
        std::process::exit(1);
    }
    for f in ["stops.txt", "routes.txt", "trips.txt", "stop_times.txt"] {
        if !dir.join(f).is_file() {
            eprintln!("✗ Falta l'arxiu GTFS obligatori: {}/{}", GTFS_DIR, f);
            std::process::exit(1);
        }
    }

    // 2. Cargar midiendo el tiempo en milisegundos.
    println!("→ Carregant infraestructura des de `{}` …", GTFS_DIR);
    let t0 = Instant::now();
    let net = match gtfs_loader::load(dir) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("✗ Error carregant el GTFS: {e}");
            std::process::exit(1);
        }
    };
    let load_ms = t0.elapsed().as_secs_f64() * 1000.0;
    println!("✓ Càrrega completada en {:.1} ms", load_ms);
    println!(
        "✓ Correccions del feed: {} viatges reetiquetats (R2S→R15, R2N→R11) · color R13 · {} viatges fora de la xarxa descartats\n",
        net.feed_fixes, net.isolated_trips
    );

    // Modo motor unificado (Fase 1, headless): `cargo run --release -- simulate`
    if std::env::args().any(|a| a == "simulate") {
        run_engine_demo(&net);
        return;
    }
    // Comparación con Renfe: `cargo run --release -- compare [semilla]` (antes vs ahora vs real).
    if std::env::args().any(|a| a == "compare") {
        let seed = std::env::args().skip_while(|a| a != "compare").nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);
        let adif = calculator::infrastructure::AdifNet::load_default();
        let ltv = calculator::ltv::load_default();
        let t0 = Instant::now();
        let rep = compare::compare(&net, adif.as_ref(), ltv.as_ref(), seed);
        compare::print_and_save(&rep);
        println!("  ({:.1} s)", t0.elapsed().as_secs_f64());
        return;
    }
    // Diagnóstico de UN tren: `cargo run --release -- diag <trainid>` (rastrea retraso/retenciones).
    if std::env::args().any(|a| a == "diag") {
        if let Some(id) = std::env::args().skip_while(|a| a != "diag").nth(1) {
            run_train_diag(&net, &id);
        } else {
            eprintln!("Ús: diag <train_id>");
        }
        return;
    }
    // Sonda de ocupación de un tramo: `probe "<from>" "<to>" [hIni hFin]` (quién ocupa el cantón).
    if std::env::args().any(|a| a == "probe") {
        let mut it = std::env::args().skip_while(|a| a != "probe").skip(1);
        let (from, to) = (it.next().unwrap_or_default(), it.next().unwrap_or_default());
        let h0: f64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
        let h1: f64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(24.0);
        run_block_probe(&net, &from, &to, h0, h1);
        return;
    }
    // Modo optimizador del SISTEMA: `cargo run --release -- optimize`
    if std::env::args().any(|a| a == "optimize" || a == "--optimize") {
        run_system_optimizer(&net);
        return;
    }

    // Física del motor unificado (misma para el dashboard estático y el servidor interactivo).
    let adif = calculator::infrastructure::AdifNet::load_default();
    match &adif {
        Some(a) => println!("✓ CVM ADIF carregada: {} segments de via (processed/adif)", a.n_segments()),
        None => println!("· CVM ADIF no trobada (processed/adif/rfig_speed.json). Executa scripts/fetch_adif_cvm.py"),
    }
    match track_geometry::global() {
        Some(g) => println!("✓ Traçat real de via (OSM) carregat: {} cantons", g.len()),
        None => println!("· Traçat de via OSM no trobat: cantons en línia recta. Executa scripts/fetch_osm_track.py"),
    }
    let ltv = calculator::ltv::load_default();
    match &ltv {
        Some(l) => println!("✓ LTV carregades: {} limitacions temporals (snapshot {})", l.count(), l.snapshot),
        None => println!("· LTV no trobades. Deixa el ZIP diari a raw/ltv/ o executa scripts/fetch_adif_ltv.py"),
    }

    // 3. Construir las vistas (con parámetros por defecto) y volcarlas a consola.
    let params = SimParams::default();
    let summary = scenario::summary_view(&net, load_ms);
    let example = scenario::example_view(&net, EXAMPLE_TRAIN, None);
    let sim = scenario::sim_view(&net, adif.as_ref(), ltv.as_ref(), &params);
    let res = scenario::res_view(&net, adif.as_ref(), ltv.as_ref(), &params);

    print_summary(&net, &summary);
    print_example(&example);
    print_sim(&sim);
    print_res(&res);

    // 4. Escribir el dashboard HTML estático (copia offline).
    let static_path = write_static_dashboard(&summary, &example, &sim, &res);

    let no_open = std::env::args().any(|a| a == "--no-open");
    let static_only = std::env::args().any(|a| a == "--static");

    if static_only {
        if let Some(p) = &static_path {
            println!("\n🖥  Dashboard estàtic: {}", p);
            if !no_open {
                let _ = open_in_browser(p);
            }
        }
        return;
    }

    // 5. Arrancar el servidor web interactivo.
    let lines = scenario::distinct_lines(&net);
    let state = Arc::new(server::ServerState {
        net: std::sync::RwLock::new(Arc::new(net)),
        load_ms,
        lines: std::sync::RwLock::new(lines),
        line_cache: std::sync::Mutex::new(std::collections::HashMap::new()),
        adif,
        ltv: std::sync::RwLock::new(ltv.map(std::sync::Arc::new)),
        opt: Arc::new(std::sync::Mutex::new(server::OptJob::default())),
        gtfs_job: Arc::new(std::sync::Mutex::new(server::GtfsJob::default())),
        cadence_cat: std::sync::Mutex::new(None),
        cadences: std::sync::Mutex::new(std::collections::HashMap::new()),
    });
    let url = format!("http://127.0.0.1:{}", PORT);
    if !no_open {
        let _ = open_in_browser(&url);
    }
    if let Err(e) = server::serve(state, PORT).await {
        eprintln!("✗ El servidor ha fallat (potser el port {PORT} està ocupat): {e}");
        if let Some(p) = &static_path {
            eprintln!("  Pots obrir el dashboard estàtic: {}", p);
        }
        std::process::exit(1);
    }
}

// --------------------------------------------------------------------------
// Modo optimizador de horarios
// --------------------------------------------------------------------------

/// Optimización del SISTEMA completo (todas las líneas, día laborable, incidencias).
fn run_system_optimizer(net: &Network) {
    use optimizer::SystemSearch;
    let sc = SystemSearch::default();
    let w = PotentialWeights::default();

    println!("┌─ OPTIMITZACIÓ DEL SISTEMA (Recuit Simulat + Monte Carlo · rayon) ─┐");
    println!(
        "  Dia laborable {}–{}  ·  {} iteracions × {} sims Monte-Carlo (llavor per run)",
        fmt_hms(sc.window.0), fmt_hms(sc.window.1), sc.iters, sc.mc_runs
    );
    println!("  Sistema SENCER: es coordinen totes les línies (conflictes entre línies).");
    println!("  Motor UNIFICAT (engine.rs): velocitats per física, blocs, vía única, incidències");
    println!("  probabilístiques per plantilla — la MATEIXA física que veu el joc.");
    println!("└──────────────────────────────────────────────────────────────────┘");

    let total = sc.iters;
    let progress = move |it: usize, cur: f64, best: f64| {
        if it == 0 || it % 20 == 0 || it == total {
            println!("   iter {:>4}/{}  ·  V actual {:>10.1}  ·  millor {:>10.1}", it, total, cur, best);
        }
    };

    // Física del motor unificado (misma que ve el juego); si falta, cae a Vmax por defecto.
    let adif = calculator::infrastructure::AdifNet::load_default();
    let ltv = calculator::ltv::load_default();

    let t0 = Instant::now();
    let result = optimizer::optimize_system(net, adif.as_ref(), ltv.as_ref(), sc, w, &progress);
    let secs = t0.elapsed().as_secs_f64();

    let Some(result) = result else {
        eprintln!("✗ No hi ha prou trens per optimitzar.");
        return;
    };
    exporter::print_system_comparison(&result);

    let service_id = net.dominant_service().unwrap_or_default();
    let dir = Path::new("report").join("optimized");
    println!("\n  Exportant horaris optimitzats (CSV + PDF) per línia a {}\\ …", dir.display());
    let files = exporter::export_system(net, &result, &service_id, sc.window, &dir);
    for (line, _csv, pdf) in &files {
        println!("   ✓ {:<4} → {}", line, pdf.file_name().unwrap().to_string_lossy());
    }
    println!("\n✓ Optimització del sistema completada en {:.1} s.", secs);
}

// --------------------------------------------------------------------------
// Salida por consola (desde las vistas ya calculadas)
// --------------------------------------------------------------------------

fn print_summary(net: &Network, s: &SummaryView) {
    println!("┌─ RESUM DE LA XARXA CARREGADA ─────────────────────────────────────┐");
    println!("  Vies/andanes mapejats (nodes) .... {}", s.n_stops);
    println!("  ├ amb parent_station definida ..... {}", s.with_parent);
    println!("  Cantons/seccions (arestes) ....... {}", s.n_edges);
    println!("  Serveis de tren carregats ........ {}", s.n_services);
    println!("  Línies (routes) .................. {}", s.n_routes);
    if let Some((a, b, secs)) = &s.fastest_edge {
        println!("  Cantó més ràpid .................. {} → {}  ({}s)", a, b, secs);
    }
    let top: Vec<String> = s.per_line.iter().map(|(r, c)| format!("{}={}", r, c)).collect();
    println!("  Serveis per línia (top) .......... {}", top.join("  "));
    let sample: Vec<&str> = net.services.iter().take(6).map(|x| x.train_number.as_str()).collect();
    println!("  Exemples de nº de circulació ..... {}", sample.join(", "));
    println!("└───────────────────────────────────────────────────────────────────┘\n");
}

fn print_example(example: &Option<ExampleView>) {
    let Some(ex) = example else {
        println!("(No hi ha serveis per mostrar)\n");
        return;
    };
    if ex.found {
        println!("┌─ RUTA DETALLADA DEL TREN {} ─────────────────────────────────┐", ex.wanted);
    } else {
        println!(
            "┌─ TREN {} no trobat; mostro el servei d'exemple {} ({}) ─┐",
            ex.wanted, ex.train_number, ex.route_short
        );
    }
    println!(
        "  Línia {} (route_id {}) · trip_id {} · {} parades",
        ex.route_short, ex.route_id, ex.trip_id, ex.stops.len()
    );
    println!("  {:<4} {:<30} {:>8} {:>8} {:>8}  {:<4}", "seq", "Estació", "arribada", "sortida", "marxa", "via");
    println!("  {}", "─".repeat(70));
    for st in &ex.stops {
        let run_txt = if st.run_secs > 0 {
            format!("{}m{:02}s", st.run_secs / 60, st.run_secs % 60)
        } else {
            "—".into()
        };
        println!(
            "  {:<4} {:<30} {:>8} {:>8} {:>8}  {:<4}",
            st.seq, trunc(&st.name, 30), fmt_hms(st.arr), fmt_hms(st.dep), run_txt, st.track
        );
    }
    println!("└───────────────────────────────────────────────────────────────────┘\n");
}

fn print_sim(sim: &SimView) {
    println!("┌─ SIMULACIÓ CTC · {} (service_id dominant: {}) ─┐", sim.window, sim.service_id);
    println!("  Estacions clau monitoritzades: {}", sim.key_stations.join(", "));
    if let Some((a, b, c)) = &sim.busiest {
        println!("  Cantó més carregat: {} → {} ({} circulacions/finestra)", a, b, c);
    }
    println!("└───────────────────────────────────────────────────────────────────┘\n");

    println!("── INCIDÈNCIES ───────────────────────────────────────────────────────");
    if sim.incidents.is_empty() {
        println!("  (cap incidència)");
    }
    for i in &sim.incidents {
        println!("  · {}", i);
    }

    println!("\n── MÈTRIQUES D'ESTABILITAT (motor unificat) ──────────────────────────");
    println!("  Trens simulats en la finestra ...... {}", sim.trains_run);
    println!("  Retencions (conflictes) ............ {}", sim.held);
    println!("  Pic de retard acumulat ............. {} s a les {}", sim.peak_total, sim.peak_time);
    match &sim.recovery {
        Some(t) => println!("  Retorn a l'equilibri (<120 s) ...... {}", t),
        None => println!("  Retorn a l'equilibri ............... NO assolit dins la finestra"),
    }
    println!();
}

fn print_res(res: &ResView) {
    println!("┌─ ANÀLISI DE RESILIÈNCIA (rayon · Monte Carlo en paral·lel) ─┐");
    println!("  Escenari: bloqueig del cantó {}", res.segment);
    println!("  {:<12} {:>14} {:>16} {:>12}", "bloqueig", "pic acumulat", "recuperació", "conflictes");
    println!("  {}", "─".repeat(58));
    for r in &res.rows {
        let recov = match r.recovery {
            Some(m) => format!("{} min", m),
            None => "no recupera".into(),
        };
        println!("  {:<12} {:>12} s {:>16} {:>12}", format!("{} min", r.mins), r.peak, recov, r.held);
    }
    println!("└───────────────────────────────────────────────────────────────────┘");
}

// --------------------------------------------------------------------------
// Dashboard estático + apertura de navegador
// --------------------------------------------------------------------------

/// Fase 1b del motor unificado (headless): corre la jornada con SEÑALIZACIÓN DE BLOQUES (1 tren/
/// bloque, retención, vía única con testigo y prioridad regional, aspecto de señal), muestra una
/// foto a mediodía (camino de VER: `observe`) y las métricas finales (camino de LOTE:
/// `run`/`metrics`). Regla de oro: los efectos de congestión afloran a ×25 en el frontend.
fn run_engine_demo(net: &Network) {
    use engine::{Engine, EngineCfg};
    let mut cfg = EngineCfg::day();
    // Semilla opcional del Monte-Carlo: `simulate <seed>` (para explorar días con incidencias).
    if let Some(seed) = std::env::args().skip_while(|a| a != "simulate").nth(1).and_then(|s| s.parse().ok()) {
        cfg.seed = seed;
    }
    // Datos reales de infraestructura para la física (Vmax de vía CVM/ADIF + LTV).
    let adif = calculator::infrastructure::AdifNet::load_default();
    let ltv = calculator::ltv::load_default();
    match &adif {
        Some(a) => println!("✓ CVM ADIF: {} segments de via (física per cantó)", a.n_segments()),
        None => println!("· CVM ADIF no trobada → Vmax de via per defecte (100 km/h). Executa scripts/fetch_adif_cvm.py"),
    }
    match demand::global() {
        Some(d) => println!(
            "✓ Demanda Renfe 2018: {} estacions · {:.0} viatgers/dia (pujades per franja de 30 min)",
            d.stations.len(),
            d.total_board_day()
        ),
        None => println!("· Sense dades de demanda (raw/renfe) → demanda estimada. Executa scripts/fetch_renfe_demand.py"),
    }
    // `simulate noinc` → jornada SIN incidencias (solo retrasos estructurales GTFS vs física).
    let noinc = std::env::args().any(|a| a == "noinc");
    let templates = if noinc { Vec::new() } else { engine::default_templates() };
    let mut eng = Engine::new(net, cfg, adif.as_ref(), ltv.as_ref(), &templates, None, None);
    let incs = eng.incidents().to_vec();
    if incs.is_empty() {
        println!("· Cap incidència avui (llavor {})", cfg.seed);
    } else {
        println!("→ Incidències del dia (llavor {}): {}", cfg.seed, incs.len());
        for i in &incs {
            let ef = match i.effect {
                engine::IncidentEffect::RedSignal { both: true } => "senyal roig (2 sentits)",
                engine::IncidentEffect::RedSignal { both: false } => "senyal roig (1 sentit)",
                engine::IncidentEffect::SingleTrack => "tram a via única",
            };
            println!(
                "   · {} · {} · {} → {} · {}–{}",
                i.motivo, ef, i.from, i.to,
                (i.t0 as u32) / 3600, (i.t1 as u32) / 3600
            );
        }
    }

    let t0 = Instant::now();
    let noon = 12.0 * 3600.0;
    while eng.clock() < noon {
        eng.step(cfg.dt_sec);
    }
    let frame = eng.observe();
    let esperant = frame.iter().filter(|t| t.waiting).count();
    println!(
        "→ Motor (Fase B) · trens en marxa a les 12:00: {} ({} retinguts)",
        frame.len(),
        esperant
    );
    if let Some(t) = frame.first() {
        let asp = ["verd", "ambre", "vermell"][t.aspect as usize];
        println!(
            "   ex.: {} ({}) {} → {} · {:.0}% del cantó · {:.0} km/h · senyal {} · retard {:.0}s · pos ({:.4},{:.4})",
            t.id, t.line, t.from, t.to, t.frac * 100.0, t.speed_kmh, asp, t.delay_s, t.lat, t.lon
        );
    }

    let m = eng.run();
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    println!(
        "✓ Jornada de {:.0} h · {} trens · retard acumulat {:.0} tren·min · pic {:.0}s · {:.1} ms",
        m.sim_seconds as f64 / 3600.0,
        m.n_trains,
        m.train_min_delay,
        m.max_delay_s,
        ms
    );
    println!(
        "   Senyalització de blocs · pic retinguts {} · retinguts al final {} · KPI {:.0} PAX·min de retard",
        m.peak_waiting, m.final_waiting, m.pax_min
    );
    println!(
        "   (Fase B = velocitats per FÍSICA: Vmax de via CVM/ADIF + LTV + material real 447/450/465/470, sortida no abans de l'horari; PENDENT: incidències — veure docs/PLAN-motor-unificado.md)"
    );

    // Diagnóstico: retrasos SEVEROS (>15 min) — dónde se quedó parado cada tren y a qué hora.
    const SEVERE_S: u16 = 15 * 60;
    let mut severe: Vec<&engine::LateArrival> =
        eng.late_arrivals().iter().filter(|a| a.delay >= SEVERE_S).collect();
    // Peor retraso por (tren, estación): el motor registra una llegada por parada; nos quedamos
    // con el instante en que ese tren tocó su pico de retraso en cada estación afectada.
    severe.sort_by(|a, b| b.delay.cmp(&a.delay));
    println!(
        "\n── RETARDS SEVERS (>15 min){} ──────────────────────────────",
        if noinc { " · SENSE INCIDÈNCIES" } else { "" }
    );
    if severe.is_empty() {
        println!("  (cap tren supera els 15 min de retard)");
    } else {
        println!("  {} llegades ≥15 min tard. {:<6} {:<5} {:>8}  {}", severe.len(), "tren", "línia", "retard", "estació @ hora");
        for a in severe.iter().take(60) {
            println!(
                "  {:<6} {:<5} {:>5} min  {} @ {}",
                a.train, a.line, a.delay / 60,
                net.stop_name(&a.stop), fmt_hms(a.t)
            );
        }
        if severe.len() > 60 {
            println!("  … i {} més", severe.len() - 60);
        }
    }
}

/// Rastreo por step de UN tren durante la jornada SIN incidencias: imprime cada llegada a estación
/// (con retraso e incremento respecto a la parada anterior) y cada tramo de RETENCIÓN por señal/vía
/// única (dónde empieza, dónde acaba, cuánto). Sirve para ver si el retraso viene de la FÍSICA
/// (marcha más lenta que el GTFS) o de RETENCIONES (señales/cruces).
fn run_train_diag(net: &Network, id: &str) {
    use engine::{Engine, EngineCfg};
    let cfg = EngineCfg::day();
    let adif = calculator::infrastructure::AdifNet::load_default();
    let ltv = calculator::ltv::load_default();
    let mut eng = Engine::new(net, cfg, adif.as_ref(), ltv.as_ref(), &[], None, None);

    println!("┌─ DIAGNÒSTIC DEL TREN {} (jornada SENSE incidències) ─┐", id);
    println!("  {:<8} {:<7} {:<32} {:>7} {:>7}", "hora", "fase", "ubicació", "retard", "Δ");

    let mut prev_phase = 3u8;
    let mut prev_from = String::new();
    let mut prev_delay = 0.0f64;
    let mut held_since: Option<(f64, String, String)> = None;
    let mut found_any = false;

    let end = 24.0 * 3600.0;
    while eng.clock() < end {
        eng.step(cfg.dt_sec);
        let now = eng.clock();
        let Some(d) = eng.train_diag(id) else { continue };
        if !found_any {
            println!("  (línia {} · {:.0} km/h màx)", d.line, d.speed_kmh);
        }
        found_any = true;

        // Inicio/fin de una retención por señal.
        match (&held_since, d.waiting) {
            (None, true) => held_since = Some((now, d.from_id.clone(), d.to_id.clone())),
            (Some((t0, f, t)), false) => {
                println!(
                    "  {:<8} {:<7} {:<32} {:>6.0}s  (RETINGUT {:.0}s a {} → {})",
                    fmt_hms(now as u32), "▓hold", "", now - t0, now - t0,
                    net.stop_name(f), net.stop_name(t)
                );
                held_since = None;
            }
            _ => {}
        }

        // Llegada a una estación nueva (transición a dwell en una estación distinta).
        let arrived_station = d.phase == 0 && (prev_phase != 0 || d.from_id != prev_from);
        if arrived_station && !d.from_id.is_empty() {
            let delta = d.delay_s - prev_delay;
            let flag = if d.delay_s >= 900.0 { " ⚠" } else { "" };
            println!(
                "  {:<8} {:<7} {:<32} {:>5.0}m  {:>+5.0}s{}",
                fmt_hms(now as u32), "parada", trunc(net.stop_name(&d.from_id), 32),
                d.delay_s / 60.0, delta, flag
            );
            prev_delay = d.delay_s;
            prev_from = d.from_id.clone();
        }
        prev_phase = d.phase;
        if d.phase == 2 {
            break;
        }
    }
    if !found_any {
        println!("  (el tren {} no apareix a la jornada — comprova l'id)", id);
    }
    println!("└──────────────────────────────────────────────────────────────┘");
}

/// Sonda de ocupación de un cantón dirigido `from → to` durante una franja: imprime cada CAMBIO
/// en la lista de trenes que ocupan sus sub-bloques (jornada SIN incidencias). Revela quién bloquea
/// un tramo y cuánto (cabeza de un atasco).
fn run_block_probe(net: &Network, from: &str, to: &str, h0: f64, h1: f64) {
    use engine::{Engine, EngineCfg};
    let (Some(fs), Some(ts)) = (net.find_stop_by_name(from), net.find_stop_by_name(to)) else {
        eprintln!("No trobo alguna estació: '{}' / '{}'", from, to);
        return;
    };
    let (fid, tid) = (fs.stop_id.clone(), ts.stop_id.clone());
    println!("┌─ SONDA cantó {} → {} ({}–{}h) ─┐", fs.stop_name, ts.stop_name, h0, h1);
    let cfg = EngineCfg::day();
    let adif = calculator::infrastructure::AdifNet::load_default();
    let ltv = calculator::ltv::load_default();
    let mut eng = Engine::new(net, cfg, adif.as_ref(), ltv.as_ref(), &[], None, None);

    let (s0, s1) = (h0 * 3600.0, h1 * 3600.0);
    let mut prev = String::new();
    while eng.clock() < 24.0 * 3600.0 {
        eng.step(cfg.dt_sec);
        let now = eng.clock();
        if now < s0 || now > s1 {
            if now > s1 { break; }
            continue;
        }
        if let Some(mut occ) = eng.block_occupants(&fid, &tid) {
            occ.sort();
            let sig = occ.iter().map(|(sb, id)| format!("sb{}:{}", sb, id)).collect::<Vec<_>>().join("  ");
            if sig != prev {
                let show = if sig.is_empty() { "—lliure—" } else { sig.as_str() };
                println!("  {}  [{}]", fmt_hms(now as u32), show);
                prev = sig;
            }
        }
    }
    println!("└──────────────────────────────────────────────────────────────┘");
}

fn write_static_dashboard(
    summary: &SummaryView,
    example: &Option<ExampleView>,
    sim: &SimView,
    res: &ResView,
) -> Option<String> {
    let html = report::render_html(summary, example, sim, res, &scenario::now_utc_string());
    let dir = Path::new("report");
    if let Err(e) = std::fs::create_dir_all(dir) {
        eprintln!("✗ No puc crear la carpeta report/: {e}");
        return None;
    }
    let path = dir.join("dashboard.html");
    if let Err(e) = std::fs::write(&path, html) {
        eprintln!("✗ No puc escriure el dashboard: {e}");
        return None;
    }
    let abs = std::fs::canonicalize(&path).unwrap_or(path);
    Some(abs.to_string_lossy().replace(r"\\?\", ""))
}

#[cfg(target_os = "windows")]
fn open_in_browser(target: &str) -> std::io::Result<()> {
    std::process::Command::new("cmd")
        .args(["/C", "start", "", target])
        .spawn()
        .map(|_| ())
}

#[cfg(not(target_os = "windows"))]
fn open_in_browser(target: &str) -> std::io::Result<()> {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    std::process::Command::new(opener).arg(target).spawn().map(|_| ())
}

fn trunc(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{}…", t)
    }
}
