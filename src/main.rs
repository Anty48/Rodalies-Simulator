//! rodalies-sim — Simulador de tráfico ferroviario e incidencias para Rodalies de
//! Barcelona, construido 100% dinámicamente desde GTFS histórico (`./data/gtfs`).

mod gtfs_loader;
mod passenger_model;
mod report;
mod simulation_engine;

use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use rayon::prelude::*;

use gtfs_loader::{fmt_hms, Network, TrainService};
use passenger_model::PassengerModel;
use report::{ExampleView, ResRow, ResView, SimView, StopRow, SummaryView};
use simulation_engine::{key_station_names, Incident, SimConfig, Simulator};

const GTFS_DIR: &str = "./data/gtfs";
/// Tren de ejemplo pedido en el enunciado (se cae a cualquiera si no existe).
const EXAMPLE_TRAIN: &str = "25412";

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

    // 3. Resum de la xarxa.
    let summary = network_summary(&net, load_ms);
    let example = example_route(&net, EXAMPLE_TRAIN);

    // 4. Simulació de prova de 2 hores (07:00–09:00) amb incidències.
    let sim = run_simulation(&net);

    // 5. Anàlisi de resiliència en paral·lel (rayon + rand).
    let res = run_resilience_sweep(&net);

    // 6. Generar i obrir el dashboard HTML.
    generate_dashboard(&summary, &example, &sim, &res);
}

/// Escribe el dashboard HTML autocontenido y lo abre en el navegador.
fn generate_dashboard(
    summary: &SummaryView,
    example: &Option<ExampleView>,
    sim: &SimView,
    res: &ResView,
) {
    let html = report::render_html(summary, example, sim, res, &now_utc_string());
    let dir = Path::new("report");
    if let Err(e) = std::fs::create_dir_all(dir) {
        eprintln!("✗ No puc crear la carpeta report/: {e}");
        return;
    }
    let path = dir.join("dashboard.html");
    if let Err(e) = std::fs::write(&path, html) {
        eprintln!("✗ No puc escriure el dashboard: {e}");
        return;
    }
    let abs = std::fs::canonicalize(&path).unwrap_or(path);
    let abs_str = abs.to_string_lossy().replace(r"\\?\", ""); // limpia prefijo UNC de Windows
    println!("\n🖥  Dashboard generat: {}", abs_str);

    if std::env::args().any(|a| a == "--no-open") {
        println!("   (obre'l manualment; --no-open actiu)");
        return;
    }
    println!("   Obrint al navegador…");
    let _ = open_in_browser(&abs_str);
}

#[cfg(target_os = "windows")]
fn open_in_browser(path: &str) -> std::io::Result<()> {
    std::process::Command::new("cmd")
        .args(["/C", "start", "", path])
        .spawn()
        .map(|_| ())
}

#[cfg(not(target_os = "windows"))]
fn open_in_browser(path: &str) -> std::io::Result<()> {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    std::process::Command::new(opener).arg(path).spawn().map(|_| ())
}

/// Fecha/hora UTC actual como texto (sin dependencias externas).
fn now_utc_string() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86400) as i64;
    let tod = secs % 86400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02} UTC",
        y,
        m,
        d,
        tod / 3600,
        (tod % 3600) / 60
    )
}

/// Algoritmo de Howard Hinnant: días desde epoch → (año, mes, día) civil.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// --------------------------------------------------------------------------

fn network_summary(net: &Network, load_ms: f64) -> SummaryView {
    let n_stops = net.graph.node_count();
    let n_edges = net.graph.edge_count();
    let n_services = net.services.len();
    let n_routes = net.routes.len();
    let with_parent = net
        .graph
        .node_weights()
        .filter(|n| n.parent_station.is_some())
        .count();

    let fastest_edge = net.graph.edge_weights().min_by_key(|e| e.nominal_run_secs).map(|e| {
        (
            net.stop_name(&e.from_stop).to_string(),
            net.stop_name(&e.to_stop).to_string(),
            e.nominal_run_secs,
        )
    });

    // Servicios por línea (short_name).
    let mut by_route: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for s in &net.services {
        *by_route.entry(s.route_short_name.as_str()).or_insert(0) += 1;
    }
    let mut linia: Vec<(String, usize)> =
        by_route.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    linia.sort_by(|a, b| b.1.cmp(&a.1));
    let per_line: Vec<(String, usize)> = linia.into_iter().take(10).collect();

    // --- Consola ---
    println!("┌─ RESUM DE LA XARXA CARREGADA ─────────────────────────────────────┐");
    println!("  Vies/andanes mapejats (nodes) .... {}", n_stops);
    println!("  ├ amb parent_station definida ..... {}", with_parent);
    println!("  Cantons/seccions (arestes) ....... {}", n_edges);
    println!("  Serveis de tren carregats ........ {}", n_services);
    println!("  Línies (routes) .................. {}", n_routes);
    if let Some((a, b, s)) = &fastest_edge {
        println!("  Cantó més ràpid .................. {} → {}  ({}s)", a, b, s);
    }
    let top: Vec<String> = per_line.iter().map(|(r, c)| format!("{}={}", r, c)).collect();
    println!("  Serveis per línia (top) .......... {}", top.join("  "));
    let sample: Vec<&str> = net.services.iter().take(6).map(|s| s.train_number.as_str()).collect();
    println!("  Exemples de nº de circulació ..... {}", sample.join(", "));
    println!("└───────────────────────────────────────────────────────────────────┘\n");

    SummaryView {
        load_ms,
        n_stops,
        n_edges,
        n_services,
        n_routes,
        with_parent,
        per_line,
        fastest_edge,
    }
}

/// Elige el servicio de ejemplo: el número exacto si existe; si no, el más largo
/// que pase por Clot; si no, el más largo de la red.
fn pick_example<'a>(net: &'a Network, wanted: &str) -> Option<&'a TrainService> {
    if let Some(s) = net.services.iter().find(|s| s.train_number == wanted) {
        return Some(s);
    }
    let clot = net
        .services
        .iter()
        .filter(|s| {
            s.schedule
                .iter()
                .any(|st| net.stop_name(&st.stop_id).to_lowercase().contains("clot"))
        })
        .max_by_key(|s| s.schedule.len());
    clot.or_else(|| net.services.iter().max_by_key(|s| s.schedule.len()))
}

fn example_route(net: &Network, wanted: &str) -> Option<ExampleView> {
    let svc = match pick_example(net, wanted) {
        Some(s) => s,
        None => {
            println!("(No hi ha serveis per mostrar)\n");
            return None;
        }
    };
    let found = svc.train_number == wanted;
    let cap = 2; // capacidad de andenes usada en el resumen estático

    let mut stops: Vec<StopRow> = Vec::new();
    for (i, st) in svc.schedule.iter().enumerate() {
        let run = if i + 1 < svc.schedule.len() {
            let nx = &svc.schedule[i + 1];
            net.edge_between(&st.stop_id, &nx.stop_id)
                .map(|e| e.nominal_run_secs)
                .unwrap_or_else(|| nx.arrival_sec.saturating_sub(st.departure_sec))
        } else {
            0
        };
        stops.push(StopRow {
            seq: st.seq,
            name: net.stop_name(&st.stop_id).to_string(),
            arr: st.arrival_sec,
            dep: st.departure_sec,
            run_secs: run,
            track: net.assigned_track(&st.stop_id, st.seq, cap),
        });
    }

    // --- Consola ---
    if found {
        println!("┌─ RUTA DETALLADA DEL TREN {} ─────────────────────────────────┐", wanted);
    } else {
        println!(
            "┌─ TREN {} no trobat; mostro el servei d'exemple {} ({}) ─┐",
            wanted, svc.train_number, svc.route_short_name
        );
    }
    println!(
        "  Línia {} (route_id {})  ·  trip_id {}  ·  {} parades",
        svc.route_short_name, svc.route_id, svc.trip_id, svc.schedule.len()
    );
    println!("  {:<4} {:<30} {:>8} {:>8} {:>8}  {:<4}", "seq", "Estació", "arribada", "sortida", "marxa", "via");
    println!("  {}", "─".repeat(70));
    for s in &stops {
        let run_txt = if s.run_secs > 0 {
            format!("{}m{:02}s", s.run_secs / 60, s.run_secs % 60)
        } else {
            "—".into()
        };
        println!(
            "  {:<4} {:<30} {:>8} {:>8} {:>8}  {:<4}",
            s.seq, trunc(&s.name, 30), fmt_hms(s.arr), fmt_hms(s.dep), run_txt, s.track
        );
    }
    println!("└───────────────────────────────────────────────────────────────────┘\n");

    Some(ExampleView {
        found,
        wanted: wanted.to_string(),
        train_number: svc.train_number.clone(),
        route_short: svc.route_short_name.clone(),
        route_id: svc.route_id.clone(),
        trip_id: svc.trip_id.clone(),
        headsign: svc.headsign.clone().unwrap_or_default(),
        stops,
    })
}

fn run_simulation(net: &Network) -> SimView {
    let service_id = net.dominant_service().unwrap_or_default();
    let cfg = SimConfig::default();
    let (start, end) = (cfg.start_sec, cfg.end_sec);
    let window = format!("{}–{}", fmt_hms(start), fmt_hms(end));

    println!("┌─ SIMULACIÓ CTC · {} (service_id dominant: {}) ─┐", window, service_id);

    let present = key_station_names(net, &cfg.key_stations);
    let mut key_stations: Vec<String> = present.into_iter().collect();
    key_stations.sort();
    println!("  Estacions clau monitoritzades: {}", key_stations.join(", "));

    let busiest = net.busiest_segment(&service_id, start, end).map(|(from, to, cnt)| {
        (net.stop_name(&from).to_string(), net.stop_name(&to).to_string(), cnt)
    });
    if let Some((a, b, cnt)) = &busiest {
        println!("  (cantó més carregat: {} → {}, {} circulacions/finestra)", a, b, cnt);
    }
    println!("└───────────────────────────────────────────────────────────────────┘\n");

    let mut sim = Simulator::new(net, cfg);

    // Incidència 1: bloqueig del cantó MÉS transitat de la finestra (impacte garantit).
    if let Some((from, to, _)) = net.busiest_segment(&service_id, start, end) {
        // S'injecta per NOM d'estació (com a l'exemple de l'enunciat).
        sim.add_incident(Incident::BlockSegment {
            from_stop_name: net.stop_name(&from).to_string(),
            to_stop_name: net.stop_name(&to).to_string(),
            from_sec: start + 20 * 60, // 07:20
            dur_secs: 12 * 60,         // 12 min
        });
    }
    // Incidència 2: retard puntual de +8 min al primer tren que passa per Clot.
    if let Some(tn) = first_train_through(net, &service_id, "clot") {
        sim.add_incident(Incident::TrainDelay {
            train_number: tn,
            at_stop_name: "Clot".into(),
            extra_secs: 8 * 60, // +8 min
        });
    }

    let res = sim.run(&service_id);

    // Log tipus CTC (limitat per no inundar la consola).
    println!("── LOG CTC (entrades/sortides a trams clau) ──────────────────────────");
    let max_lines = 60usize;
    for line in res.log.iter().take(max_lines) {
        println!("{}", line);
    }
    if res.log.len() > max_lines {
        println!("… ({} línies més al log)", res.log.len() - max_lines);
    }

    println!("\n── MÈTRIQUES D'ESTABILITAT ───────────────────────────────────────────");
    println!("  Trens simulats en la finestra ...... {}", res.trains_run);
    println!("  Arribades processades .............. {}", res.total_arrivals);
    println!("  Retencions per senyalització ....... {}", res.held_events);
    println!(
        "  Pic de retard acumulat de la xarxa . {} s ({:.1} min·tren) a les {}",
        res.peak_total_delay,
        res.peak_total_delay as f64 / 60.0,
        fmt_hms(res.peak_time)
    );
    println!("  Màxim de trens retardats alhora .... {}", res.peak_delayed);
    if let Some(pk) = res.timeline.iter().find(|s| s.time == res.peak_time) {
        println!(
            "  Retard mitjà per tren actiu al pic . {:.0} s (sobre {} trens en circulació)",
            pk.mean_active, pk.active
        );
    }
    let recovery = res.recovery_time.map(|t| {
        format!("{} (+{} min del pic)", fmt_hms(t), (t.saturating_sub(res.peak_time)) / 60)
    });
    match &recovery {
        Some(txt) => println!("  Retorn a l'equilibri (<120 s) ...... {}", txt),
        None => println!("  Retorn a l'equilibri ............... NO assolit dins la finestra"),
    }

    // Mini perfil temporal del retard acumulat de la xarxa.
    let scale = (res.peak_total_delay.max(40) as f64) / 40.0; // 40 columnes al pic
    println!("\n  Perfil de retard ACUMULAT de la xarxa (mostres cada 10 min):");
    for s in res.timeline.iter().step_by(10) {
        let bars = (s.total_delay as f64 / scale).round() as usize;
        println!(
            "   {}  {:>5} s  [{}] ({} trens, {} retardats)",
            fmt_hms(s.time),
            s.total_delay,
            "█".repeat(bars.min(40)),
            s.active,
            s.delayed
        );
    }
    println!();

    let peak_mean = res
        .timeline
        .iter()
        .find(|s| s.time == res.peak_time)
        .map(|s| s.mean_active)
        .unwrap_or(0.0);

    SimView {
        window,
        service_id,
        key_stations,
        busiest,
        incidents: res.incidents.clone(),
        events: res.events.clone(),
        trains_run: res.trains_run,
        arrivals: res.total_arrivals,
        held: res.held_events,
        peak_total: res.peak_total_delay,
        peak_time: fmt_hms(res.peak_time),
        peak_delayed: res.peak_delayed,
        peak_mean,
        recovery,
        timeline: res.timeline.clone(),
    }
}

/// Barrido de resiliencia en paralelo: varias severidades de incidencia a la vez.
fn run_resilience_sweep(net: &Network) -> ResView {
    let empty = ResView { segment: "—".into(), rows: Vec::new() };
    let Some(service_id) = net.dominant_service() else {
        return empty;
    };

    let cfg0 = SimConfig::default();
    let (start, end) = (cfg0.start_sec, cfg0.end_sec);
    let Some((from, to, _)) = net.busiest_segment(&service_id, start, end) else {
        return empty;
    };
    let segment = format!(
        "{} → {} a les {}",
        net.stop_name(&from),
        net.stop_name(&to),
        fmt_hms(start + 20 * 60)
    );

    // Cada escenario bloquea el cantón más transitado durante una duración creciente.
    let durations: Vec<u32> = vec![0, 3, 6, 9, 12, 18]; // minuts de bloqueig

    println!("┌─ ANÀLISI DE RESILIÈNCIA (rayon · {} escenaris Monte Carlo en paral·lel) ─┐",
        durations.len());
    println!("  Escenari: bloqueig del cantó {} → {} a les {}",
        net.stop_name(&from), net.stop_name(&to), fmt_hms(start + 20 * 60));

    let mut results: Vec<(u32, i64, usize, Option<u32>, usize)> = durations
        .par_iter()
        .map(|&mins| {
            let mut cfg = SimConfig::default();
            // Model de pasatgers una mica més agressiu per estressar el sistema.
            cfg.passenger = PassengerModel {
                base_arrival_rate: 4.5,
                ..PassengerModel::default()
            };
            // Generació estocàstica de passatgers (Monte Carlo) amb llavor per escenari.
            cfg.seed = Some(1000 + mins as u64);
            let mut sim = Simulator::new(net, cfg);
            if mins > 0 {
                sim.add_incident(Incident::BlockSegmentById {
                    from_stop_id: from.clone(),
                    to_stop_id: to.clone(),
                    from_sec: start + 20 * 60,
                    dur_secs: mins * 60,
                });
            }
            let r = sim.run(&service_id);
            let recov_min = r
                .recovery_time
                .map(|t| (t.saturating_sub(r.peak_time)) / 60);
            (mins, r.peak_total_delay, r.peak_delayed, recov_min, r.held_events)
        })
        .collect();
    results.sort_by_key(|r| r.0);

    println!("  {:<12} {:>14} {:>12} {:>16} {:>12}",
        "bloqueig", "pic acumulat", "trens ret.", "recuperació", "retencions");
    println!("  {}", "─".repeat(70));
    let mut rows: Vec<ResRow> = Vec::new();
    for (mins, peak, delayed, recov, held) in results {
        let recov_txt = match recov {
            Some(m) => format!("{} min", m),
            None => "no recupera".into(),
        };
        println!(
            "  {:<12} {:>12} s {:>12} {:>16} {:>12}",
            format!("{} min", mins),
            peak,
            delayed,
            recov_txt,
            held
        );
        rows.push(ResRow { mins, peak, delayed, recovery: recov, held });
    }
    println!("└───────────────────────────────────────────────────────────────────┘");

    ResView { segment, rows }
}

// --------------------------------------------------------------------------
// Ayudantes
// --------------------------------------------------------------------------

fn first_train_through(net: &Network, service_id: &str, station_needle: &str) -> Option<String> {
    let cfg = SimConfig::default();
    net.services
        .iter()
        .filter(|s| s.service_id == service_id)
        .filter(|s| matches!(s.first_time(), Some(t) if t >= cfg.start_sec && t <= cfg.end_sec))
        .find(|s| {
            s.schedule.iter().any(|st| {
                net.stop_name(&st.stop_id)
                    .to_lowercase()
                    .contains(station_needle)
            })
        })
        .map(|s| s.train_number.clone())
}

fn trunc(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{}…", t)
    }
}
