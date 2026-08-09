//! rodalies-sim — Simulador de tráfico ferroviario e incidencias para Rodalies de
//! Barcelona, construido 100% dinámicamente desde GTFS histórico (`./data/gtfs`).

mod gtfs_loader;
mod passenger_model;
mod simulation_engine;

use std::path::Path;
use std::time::Instant;

use rayon::prelude::*;

use gtfs_loader::{fmt_hms, Network, TrainService};
use passenger_model::PassengerModel;
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
    print_network_summary(&net);
    print_example_route(&net, EXAMPLE_TRAIN);

    // 4. Simulació de prova de 2 hores (07:00–09:00) amb incidències.
    run_simulation(&net);

    // 5. Anàlisi de resiliència en paral·lel (rayon + rand).
    run_resilience_sweep(&net);
}

// --------------------------------------------------------------------------

fn print_network_summary(net: &Network) {
    let n_stops = net.graph.node_count();
    let n_edges = net.graph.edge_count();
    let n_services = net.services.len();

    println!("┌─ RESUM DE LA XARXA CARREGADA ─────────────────────────────────────┐");
    let with_parent = net
        .graph
        .node_weights()
        .filter(|n| n.parent_station.is_some())
        .count();

    println!("  Vies/andanes mapejats (nodes) .... {}", n_stops);
    println!("  ├ amb parent_station definida ..... {}", with_parent);
    println!("  Cantons/seccions (arestes) ....... {}", n_edges);
    println!("  Serveis de tren carregats ........ {}", n_services);
    println!("  Línies (routes) .................. {}", net.routes.len());

    // Cantó d'exemple: mostra pes (temps de marxa) i capacitat de secció.
    if let Some(e) = net.graph.edge_weights().min_by_key(|e| e.nominal_run_secs) {
        println!(
            "  Cantó més ràpid .................. {} → {}  ({}s, capacitat {} tren/secció)",
            net.stop_name(&e.from_stop),
            net.stop_name(&e.to_stop),
            e.nominal_run_secs,
            e.capacity
        );
    }

    // Servicios por línea (short_name).
    let mut by_route: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for s in &net.services {
        *by_route.entry(s.route_short_name.as_str()).or_insert(0) += 1;
    }
    let mut linia: Vec<_> = by_route.into_iter().collect();
    linia.sort_by(|a, b| b.1.cmp(&a.1));
    let top: Vec<String> = linia
        .iter()
        .take(10)
        .map(|(r, c)| format!("{}={}", r, c))
        .collect();
    println!("  Serveis per línia (top) .......... {}", top.join("  "));

    // Muestra de números oficiales de Renfe.
    let sample: Vec<&str> = net
        .services
        .iter()
        .take(6)
        .map(|s| s.train_number.as_str())
        .collect();
    println!("  Exemples de nº de circulació ..... {}", sample.join(", "));
    println!("└───────────────────────────────────────────────────────────────────┘\n");
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

fn print_example_route(net: &Network, wanted: &str) {
    let Some(svc) = pick_example(net, wanted) else {
        println!("(No hi ha serveis per mostrar)\n");
        return;
    };

    if svc.train_number == wanted {
        println!("┌─ RUTA DETALLADA DEL TREN {} ─────────────────────────────────┐", wanted);
    } else {
        println!(
            "┌─ TREN {} no trobat; mostro el servei d'exemple {} ({}) ─┐",
            wanted, svc.train_number, svc.route_short_name
        );
    }
    println!(
        "  Línia {} (route_id {})  ·  trip_id {}  ·  {} parades  ·  {}",
        svc.route_short_name,
        svc.route_id,
        svc.trip_id,
        svc.schedule.len(),
        svc.headsign.clone().unwrap_or_else(|| "—".into())
    );
    println!(
        "  {:<4} {:<30} {:>8} {:>8} {:>8}  {:<4}",
        "seq", "Estació", "arribada", "sortida", "marxa", "via"
    );
    println!("  {}", "─".repeat(70));

    let cap = 2; // capacidad de andenes usada en el resumen estático
    for (i, st) in svc.schedule.iter().enumerate() {
        let name = net.stop_name(&st.stop_id);
        let run = if i + 1 < svc.schedule.len() {
            let nx = &svc.schedule[i + 1];
            net.edge_between(&st.stop_id, &nx.stop_id)
                .map(|e| e.nominal_run_secs)
                .unwrap_or_else(|| nx.arrival_sec.saturating_sub(st.departure_sec))
        } else {
            0
        };
        let run_txt = if run > 0 {
            format!("{}m{:02}s", run / 60, run % 60)
        } else {
            "—".into()
        };
        let track = net.assigned_track(&st.stop_id, st.seq, cap);
        println!(
            "  {:<4} {:<30} {:>8} {:>8} {:>8}  {:<4}",
            st.seq,
            trunc(name, 30),
            fmt_hms(st.arrival_sec),
            fmt_hms(st.departure_sec),
            run_txt,
            track
        );
    }
    println!("└───────────────────────────────────────────────────────────────────┘\n");
}

fn run_simulation(net: &Network) {
    let service_id = match net.dominant_service() {
        Some(s) => s,
        None => {
            println!("(Cap servei per simular)\n");
            return;
        }
    };

    let cfg = SimConfig::default();
    println!("┌─ SIMULACIÓ CTC · {}–{} (service_id dominant: {}) ─┐",
        fmt_hms(cfg.start_sec), fmt_hms(cfg.end_sec), service_id);

    let present = key_station_names(net, &cfg.key_stations);
    let mut present_v: Vec<_> = present.into_iter().collect();
    present_v.sort();
    println!("  Estacions clau monitoritzades: {}", present_v.join(", "));
    println!("└───────────────────────────────────────────────────────────────────┘\n");

    let (start, end) = (cfg.start_sec, cfg.end_sec);
    let mut sim = Simulator::new(net, cfg);

    // Incidència 1: bloqueig del cantó MÉS transitat de la finestra (impacte garantit).
    if let Some((from, to, cnt)) = net.busiest_segment(&service_id, start, end) {
        println!(
            "  (cantó més carregat: {} → {}, {} circulacions/finestra)\n",
            net.stop_name(&from),
            net.stop_name(&to),
            cnt
        );
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
    match res.recovery_time {
        Some(t) => println!(
            "  Retorn a l'equilibri (<120 s) ...... {}  ({} min després del pic)",
            fmt_hms(t),
            (t.saturating_sub(res.peak_time)) / 60
        ),
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
}

/// Barrido de resiliencia en paralelo: varias severidades de incidencia a la vez.
fn run_resilience_sweep(net: &Network) {
    let Some(service_id) = net.dominant_service() else {
        return;
    };

    let cfg0 = SimConfig::default();
    let (start, end) = (cfg0.start_sec, cfg0.end_sec);
    let Some((from, to, _)) = net.busiest_segment(&service_id, start, end) else {
        return;
    };

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
    }
    println!("└───────────────────────────────────────────────────────────────────┘");
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
