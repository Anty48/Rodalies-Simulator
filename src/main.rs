//! rodalies-sim — Simulador de tráfico ferroviario e incidencias para Rodalies de
//! Barcelona, construido 100% dinámicamente desde GTFS histórico (`./data/gtfs`).
//!
//! Modos:
//!   * por defecto: imprime el resumen en consola, escribe `report/dashboard.html`
//!     y arranca un servidor web interactivo en http://127.0.0.1:8080.
//!   * `--static`: solo escribe y abre el dashboard HTML (offline, sin servidor).
//!   * `--no-open`: no abre el navegador automáticamente.

mod exporter;
mod gtfs_loader;
mod map;
mod optimizer;
mod passenger_model;
mod report;
mod scenario;
mod server;
mod signaling;
mod simulation_engine;

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use gtfs_loader::{fmt_hms, Network};
use optimizer::{PotentialWeights, SearchConfig};
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
    println!("✓ Càrrega completada en {:.1} ms\n", load_ms);

    // Modo optimizador: `cargo run --release -- optimize [R1 R4 ...]`
    if std::env::args().any(|a| a == "optimize" || a == "--optimize") {
        run_optimizer(&net);
        return;
    }

    // 3. Construir las vistas (con parámetros por defecto) y volcarlas a consola.
    let params = SimParams::default();
    let summary = scenario::summary_view(&net, load_ms);
    let example = scenario::example_view(&net, EXAMPLE_TRAIN, None);
    let sim = scenario::sim_view(&net, &params);
    let res = scenario::res_view(&net, &params);

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
        net: Arc::new(net),
        load_ms,
        lines,
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

fn run_optimizer(net: &Network) {
    let service_id = match net.dominant_service() {
        Some(s) => s,
        None => {
            eprintln!("✗ No hi ha cap servei per optimitzar.");
            return;
        }
    };

    // Líneas pedidas por CLI (las que empiezan por 'R'), o un conjunto por defecto.
    let available = scenario::distinct_lines(net);
    let requested: Vec<String> = std::env::args()
        .filter(|a| a.len() >= 2 && a.starts_with('R') && a.chars().nth(1).unwrap().is_ascii_digit())
        .filter(|a| available.contains(a))
        .collect();
    let lines: Vec<String> = if !requested.is_empty() {
        requested
    } else {
        // Por defecto, las 4 líneas con más servicios.
        available.into_iter().take(4).collect()
    };

    let sc = SearchConfig::default();
    let w = PotentialWeights::default();

    println!("┌─ OPTIMITZACIÓ DE HORARIS (Recuit Simulat + Monte Carlo · rayon) ─┐");
    println!(
        "  Línies: {}  ·  finestra {}–{}  ·  {} iteracions × {} sims MC  ·  offset ±{} min",
        lines.join(", "),
        fmt_hms(sc.window.0),
        fmt_hms(sc.window.1),
        sc.iters,
        sc.mc_runs,
        sc.max_offset_min
    );
    println!("  Física: senyalització estricta per cantons (capacitat 1, groc/vermell)");
    println!("  service_id dominant: {}", service_id);
    println!("└─────────────────────────────────────────────────────────────────┘");

    let t0 = Instant::now();
    let results = optimizer::optimize_lines(net, &lines, &service_id, sc, w);
    let secs = t0.elapsed().as_secs_f64();

    if results.is_empty() {
        eprintln!("✗ Cap línia amb prou viatges a la finestra per optimitzar.");
        return;
    }

    exporter::print_comparison(&results);

    let dir = Path::new("report").join("optimized");
    println!("\n  Exportant taules d'horaris optimitzades a {}\\ …", dir.display());
    for r in &results {
        match exporter::export_line_csv(net, r, &dir) {
            Ok(p) => println!(
                "   ✓ {:<4} → {}  ({} viatges, {} amb ajust)",
                r.line,
                p.file_name().unwrap().to_string_lossy(),
                r.n_trips,
                r.offsets.len()
            ),
            Err(e) => eprintln!("   ✗ {}: {}", r.line, e),
        }
    }
    println!("\n✓ Optimització completada en {:.1} s.", secs);
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

    println!("── LOG CTC (entrades/sortides a trams clau) ──────────────────────────");
    for i in &sim.incidents {
        println!("{}", i);
    }
    let max = 40usize;
    for e in sim.events.iter().take(max) {
        if e.kind == "INCIDÈNCIA" {
            println!("  ⛔ {}", e.station);
        } else {
            println!(
                "[{}] Tren {:>8} ({:<3}) {:<6} {:<30} via {}  ({:+} s)",
                fmt_hms(e.time), e.train, e.line, e.kind, e.station, e.track, e.delay
            );
        }
    }
    if sim.events.len() > max {
        println!("… ({} esdeveniments més al dashboard)", sim.events.len() - max);
    }

    println!("\n── MÈTRIQUES D'ESTABILITAT ───────────────────────────────────────────");
    println!("  Trens simulats en la finestra ...... {}", sim.trains_run);
    println!("  Arribades processades .............. {}", sim.arrivals);
    println!("  Retencions per senyalització ....... {}", sim.held);
    println!("  Pic de retard acumulat ............. {} s a les {}", sim.peak_total, sim.peak_time);
    println!("  Màxim de trens retardats alhora .... {}", sim.peak_delayed);
    match &sim.recovery {
        Some(t) => println!("  Retorn a l'equilibri (<120 s) ...... {}", t),
        None => println!("  Retorn a l'equilibri ............... NO assolit dins la finestra"),
    }
    println!();
}

fn print_res(res: &ResView) {
    println!("┌─ ANÀLISI DE RESILIÈNCIA (rayon · Monte Carlo en paral·lel) ─┐");
    println!("  Escenari: bloqueig del cantó {}", res.segment);
    println!("  {:<12} {:>14} {:>12} {:>16} {:>12}", "bloqueig", "pic acumulat", "trens ret.", "recuperació", "retencions");
    println!("  {}", "─".repeat(70));
    for r in &res.rows {
        let recov = match r.recovery {
            Some(m) => format!("{} min", m),
            None => "no recupera".into(),
        };
        println!("  {:<12} {:>12} s {:>12} {:>16} {:>12}", format!("{} min", r.mins), r.peak, r.delayed, recov, r.held);
    }
    println!("└───────────────────────────────────────────────────────────────────┘");
}

// --------------------------------------------------------------------------
// Dashboard estático + apertura de navegador
// --------------------------------------------------------------------------

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
