//! HTML del calculador de tiempo mínimo: el panel con el formulario (desplegables de
//! estaciones + selección de series) y el fragmento de resultados que devuelve
//! `/api/mintime`. Sin dependencias externas: SVG generado en Rust, como el resto.

use super::rolling_stock::{self, Provenance, Sourced};
use super::{CalcView, TrainResult};

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// mm:ss a partir de segundos.
fn mmss(s: f64) -> String {
    let total = s.round() as i64;
    format!("{}:{:02}", total / 60, total % 60)
}

/// Color estable por serie.
fn series_color(id: &str) -> &'static str {
    match id {
        "447" => "#e2231a",
        "450" => "#58a6ff",
        "470" => "#3fb950",
        "490" => "#f5a623",
        _ => "#8b98a5",
    }
}

/// Insignia de procedencia.
fn prov_badge(p: Provenance) -> String {
    format!(
        "<span class=\"provb\" style=\"color:{}\">{}</span>",
        p.css(),
        esc(p.label())
    )
}

/// Fila "dato + valor + procedencia + fuente" de la ficha técnica.
fn ficha_row(label: &str, val: String, s: &Sourced) -> String {
    format!(
        "<tr><td>{}</td><td class=\"mono\">{}</td><td>{}</td><td class=\"muted\">{}</td></tr>",
        esc(label),
        esc(&val),
        prov_badge(s.prov),
        esc(s.source)
    )
}

// --------------------------------------------------------------------------
// Panel (formulario) — parte estática de la pestaña
// --------------------------------------------------------------------------

/// Panel del calculador: formulario + secciones fijas de Fuentes y Limitaciones.
pub fn panel(stations: &[(String, String)]) -> String {
    // Opciones de estaciones, con defaults razonables (Sants → Mataró si existen).
    let default_origin = stations
        .iter()
        .find(|(_, n)| n.to_lowercase().contains("sants"))
        .map(|(id, _)| id.clone());
    let default_dest = stations
        .iter()
        .find(|(_, n)| n.to_lowercase().contains("mataró") || n.to_lowercase().contains("mataro"))
        .map(|(id, _)| id.clone());

    let opts = |selected: &Option<String>| -> String {
        let mut s = String::new();
        for (id, name) in stations {
            let sel = if selected.as_deref() == Some(id.as_str()) { " selected" } else { "" };
            s.push_str(&format!(
                "<option value=\"{}\"{}>{}</option>",
                esc(id),
                sel,
                esc(name)
            ));
        }
        s
    };

    // Casillas de series.
    let mut series_boxes = String::new();
    for s in rolling_stock::all() {
        let checked = if s.available { " checked" } else { "" };
        let dis = if s.available { "" } else { " disabled" };
        let tag = if s.available {
            String::new()
        } else {
            " <span class=\"provb\" style=\"color:#f85149\">no disponible</span>".into()
        };
        series_boxes.push_str(&format!(
            "<label class=\"serie-chk\"><input type=\"checkbox\" class=\"mc_serie\" value=\"{}\"{}{}> \
             <b>{}</b>{}</label>",
            esc(s.id), checked, dis, esc(s.id), tag
        ));
    }

    format!(
        r#"<link rel="stylesheet" href="https://unpkg.com/leaflet@1.9.4/dist/leaflet.css"/>
  <script src="https://unpkg.com/leaflet@1.9.4/dist/leaflet.js"></script>
  <script src="https://cdn.plot.ly/plotly-2.35.2.min.js"></script>

  <div class="card">
    <h2>Calculador de temps mínim entre estacions</h2>
    <p class="muted">Simula el <b>moviment físic</b> del tren (acceleració, creuer i frenada anticipada) per estimar el <b>temps mínim teòric</b>. Tria estacions al mapa o als desplegables. Prioritat: <b>exactitud de dades &gt; aparença</b>.</p>

    <div id="mc_map" class="calcmap"></div>
    <div class="legend">
      <span><i style="background:#3fb950"></i> Origen</span>
      <span><i style="background:#e2231a"></i> Destí</span>
      <span><i style="background:#f5a623"></i> Recorregut seleccionat</span>
      <span><i style="background:#8b98a5"></i> Estacions</span>
      <span class="muted">Clic a una estació = origen; segon clic = destí. Base: OpenStreetMap.</span>
    </div>

    <div class="controls-grid" style="margin-top:16px">
      <div class="field"><label>Origen</label><select id="mc_origin">{origin_opts}</select></div>
      <div class="field"><label>Destí</label><select id="mc_dest">{dest_opts}</select></div>
      <div class="field"><label>Pas d'integració dt</label>
        <select id="mc_dt"><option value="0.05">0,05 s</option><option value="0.1" selected>0,1 s</option><option value="0.2">0,2 s</option></select></div>
      <div class="field" style="grid-column:1/-1"><label>Sèries a comparar</label>
        <div class="series-row">{series_boxes}</div></div>
      <div class="field"><label class="chk"><input type="checkbox" id="mc_ltv"> Aplicar LTV (temporals)</label></div>
      <div class="field"><button class="btn" id="mc_run">▶ Calcular tram</button></div>
    </div>
  </div>
  <div id="mc_result"></div>

  <div class="card">
    <h2>Anàlisi de línia completa</h2>
    <p class="muted">Calcula <b>tots els trams consecutius</b> d'una línia (des del GTFS, sense hardcodejar), suma <b>temps de marxa + temps de parada</b> i ho compara amb el <b>temps programat</b>. Tria línia i sentit.</p>
    <div class="controls-grid" style="margin-top:12px">
      <div class="field"><label>Línia</label><select id="ln_line"></select></div>
      <div class="field" style="grid-column:span 2"><label>Sentit</label><select id="ln_dir"></select></div>
      <div class="field"><label>Temps de parada</label>
        <select id="ln_dwell"><option value="auto" selected>Automàtic (GTFS)</option><option value="fixed">Fix</option></select></div>
      <div class="field"><label>Parada fixa <b id="ln_dwv">30</b> s</label>
        <input type="range" id="ln_dwell_s" min="0" max="120" step="5" value="30" data-out="ln_dwv"></div>
      <div class="field"><label class="chk"><input type="checkbox" id="ln_ltv"> Aplicar LTV (temporals)</label></div>
      <div class="field"><button class="btn" id="ln_run">▶ Calcular línia completa</button></div>
    </div>
    <div style="margin-top:10px;display:flex;gap:12px;align-items:center;flex-wrap:wrap">
      <span class="muted" id="ltv_status">LTV: —</span>
      <button class="zoombtn" style="float:none" id="ltv_reload">↻ Recarregar LTV (ZIP diari a raw/ltv/)</button>
    </div>
    <div id="ln_progress" class="muted" style="margin-top:10px"></div>
    <div class="progress" id="ln_bar" style="display:none"><i></i></div>
  </div>

  <div id="ln_out"></div>

  {methodology}

  <div class="modal" id="mc_modal"><div class="modal-inner">
    <button class="modal-close" id="mc_modal_close">✕ Tancar</button>
    <div class="modal-plot" id="mc_modal_plot"></div>
  </div></div>"#,
        origin_opts = opts(&default_origin),
        dest_opts = opts(&default_dest),
        series_boxes = series_boxes,
        methodology = methodology_section(),
    )
}

/// Secciones fijas: Fuentes y metodología + Precisión y limitaciones.
fn methodology_section() -> String {
    // Tabla de fichas técnicas de todas las series (con procedencia por dato).
    let mut fichas = String::new();
    for s in rolling_stock::all() {
        let vmax = if s.available { format!("{:.0} km/h", s.vmax_kmh.value) } else { "—".into() };
        let pw = if s.available { format!("{:.0} kW", s.power_kw.value) } else { "—".into() };
        let ms = if s.available { format!("{:.1} t", s.mass_t.value) } else { "—".into() };
        let acc = if s.available { format!("{:.2} m/s²", s.accel_start.value) } else { "—".into() };
        let dec = if s.available { format!("{:.2} m/s²", s.decel_service.value) } else { "—".into() };
        fichas.push_str(&format!(
            "<div class=\"ficha\"><h4>Sèrie {} · <span class=\"muted\">{}</span></h4>\
             <div class=\"scroll\"><table><thead><tr><th>Dada</th><th>Valor</th><th>Procedència</th><th>Font</th></tr></thead><tbody>{}{}{}{}{}</tbody></table></div>\
             <p class=\"muted\" style=\"margin-top:6px\">{}</p></div>",
            esc(s.id), esc(s.builder),
            ficha_row("Velocitat màxima", vmax, &s.vmax_kmh),
            ficha_row("Potència", pw, &s.power_kw),
            ficha_row("Massa (en buit)", ms, &s.mass_t),
            ficha_row("Acceleració d'arrencada", acc, &s.accel_start),
            ficha_row("Deceleració de servei", dec, &s.decel_service),
            esc(s.notes),
        ));
    }

    format!(
        r#"<section class="card">
    <h2>Fonts i metodologia</h2>
    <h3>Infraestructura i distància</h3>
    <ul class="src">
      <li><b>Distància ferroviària</b> — <span class="provb" style="color:#f5a623">Aproximació</span> No hi ha dataset de PK ni de geometria de via d'Adif (RINF/CVM) al projecte. Es fa servir la <b>polilínia d'estacions reals</b> del recorregut (seqüència del GTFS) sumant distàncies geodèsiques entre estacions consecutives. <b>No</b> és una recta origen→destí, però <b>infravalora</b> la longitud real de via (ignora la sinuositat entre estacions). Font: <code>data/gtfs</code> (Rodalies/Cercanías).</li>
      <li><b>Perfil de velocitats màximes de la via (CVM)</b> — <span class="provb" style="color:#f85149">No disponible</span> No es disposa del Quadre de Velocitats Màximes d'Adif per tram, així que <b>no s'inventa</b>: al model, el límit el posa la Vmax del propi tren. Com a dada REAL de context es mostra la <b>velocitat comercial mitjana observada</b> per tram (horaris GTFS).</li>
      <li><b>Estacions i coordenades</b> — <span class="provb" style="color:#3fb950">Oficial</span> GTFS de Rodalies/Cercanías (stops.txt: nom + lat/lon). Referència creuada amb Renfe Data (<code>listado-estaciones-rodalies-barcelona.csv</code>) i Adif (<code>estaciones.csv</code>).</li>
    </ul>
    <h3>Material rodant (sèries)</h3>
    <p class="muted">Fonts: <b>Renfe — informacion-trenes.csv</b> (data.renfe.com; Vmax, potència, massa de 447/450/470), <b>Wikipedia EN — Renfe Class 490</b> (Vmax/potència/massa de la 490), <b>Wikipedia EN — Renfe Class 447</b> (contrast). <b>Cap font</b> publica corbes d'acceleració ni de frenada per sèrie → es modelen (veure sota). La <b>sèrie 456 no s'ha trobat documentada</b> i es marca com a no disponible (no s'inventa).</p>
    {fichas}
    <h3>Model físic (tracció i frenada)</h3>
    <div class="formula">a_tracció(v) = min( a_arrencada , η · P / (m·(1+λ) · v) )&nbsp;&nbsp;·&nbsp;&nbsp;frenada = deceleració constant b</div>
    <p class="muted" style="margin-top:8px">Integració per passos (dt) amb <b>frenada anticipada</b>: a cada punt es mira endavant per no superar mai cap reducció ni el destí (v=0). Model de <b>potència constant</b> amb tope d'arrencada. Paràmetres del model (suposicions, iguals per a totes les sèries perquè no hi ha dades publicades): a_arrencada=1,0 m/s², b=0,9 m/s², η=0,85, λ=0,10.</p>

    <h2 style="margin-top:26px">Precisió i limitacions</h2>
    <p class="muted">El resultat és un <b>«temps mínim teòric»</b>, NO el temps que un maquinista pot o ha de fer en servei real. El model <b>NO</b> té en compte, entre d'altres:</p>
    <ul class="src">
      <li>Senyalització, bloqueig i trànsit d'altres trens (marxa en tènue, aspectes grocs/vermells).</li>
      <li>Límits de velocitat reals de la infraestructura per tram (CVM d'Adif no disponible).</li>
      <li>Resistència a l'avanç (Davis): els coeficients no estan disponibles per sèrie i no s'inventen → el temps és una <b>cota inferior optimista</b>.</li>
      <li>Pendents i rampes del perfil longitudinal (no disponibles).</li>
      <li>Corbes d'esforç tractor reals (no publicades) i adherència variable.</li>
      <li>Temps de parada, obertura/tancament de portes i marges de seguretat.</li>
      <li>Restriccions temporals (obres, precaucions) i limitacions operatives.</li>
      <li>La distància és una polilínia d'estacions (infravalora la via real).</li>
    </ul>
  </section>"#,
        fichas = fichas,
    )
}

// --------------------------------------------------------------------------
// Fragmento de resultados (respuesta de /api/mintime)
// --------------------------------------------------------------------------

pub fn fragment(v: &CalcView) -> String {
    if let Some(err) = &v.error {
        return format!("<div class=\"card\"><p class=\"muted\">⚠ {}</p></div>", esc(err));
    }

    // --- Cabecera de resultado (por serie disponible: tarjeta grande) ---
    let mut cards = String::new();
    for r in v.results.iter().filter(|r| r.available) {
        let warn = if !r.reached_end {
            "<div class=\"muted\" style=\"color:#f85149\">⚠ el tren no arriba a completar el recorregut amb aquests paràmetres</div>"
        } else {
            ""
        };
        let refined = match r.time_ref_s {
            Some(t) => format!(
                "<div class=\"kpi-sub\" style=\"margin-top:4px\">Amb <b>CVM ADIF</b>: <b style=\"color:#3fb950\">{}</b> (Vmàx {:.0} km/h)</div>",
                mmss(t),
                r.vmax_ref_reached_kmh.unwrap_or(0.0)
            ),
            None => String::new(),
        };
        cards.push_str(&format!(
            r#"<div class="kpi" style="border-left:4px solid {color}">
              <div class="kpi-sub">Sèrie {id} · {name}</div>
              <div class="kpi-val">{time}</div>
              <div class="kpi-label">temps mínim (Vmax tren)</div>
              <div class="kpi-sub" style="margin-top:6px">Vmàx assolida <b>{vreach:.0}</b> km/h · límit tren {vficha:.0} km/h</div>
              {refined}
              <div class="kpi-sub" style="margin-top:4px">Fiabilitat dades: {prov}</div>
              {warn}
            </div>"#,
            color = series_color(&r.id),
            id = esc(&r.id),
            name = esc(&r.name),
            time = mmss(r.time_s),
            vreach = r.vmax_reached_kmh,
            vficha = r.vmax_ficha_kmh,
            refined = refined,
            prov = prov_badge(r.prov),
            warn = warn,
        ));
    }

    // --- Tabla comparativa ---
    let mut comp = String::new();
    for r in &v.results {
        if r.available {
            comp.push_str(&format!(
                "<tr><td class=\"mono\"><span class=\"dot\" style=\"background:{}\"></span>{}</td><td class=\"num\">{:.2} km</td><td class=\"num mono\">{}</td><td class=\"num\">{:.0} km/h</td><td class=\"num\">{:.0} km/h</td></tr>",
                series_color(&r.id), esc(&r.id), v.distance_km, mmss(r.time_s), r.vmax_reached_kmh, r.vmax_ficha_kmh
            ));
        } else {
            comp.push_str(&format!(
                "<tr class=\"muted\"><td class=\"mono\">{}</td><td class=\"num\">{:.2} km</td><td colspan=3>sèrie no disponible — no es calcula (no s'inventen dades)</td></tr>",
                esc(&r.id), v.distance_km
            ));
        }
    }

    // --- Notas por serie (procedencia/aproximaciones) ---
    let mut notes = String::new();
    for r in &v.results {
        notes.push_str(&format!(
            "<li><b>{}</b>: {}</li>",
            esc(&r.id),
            esc(&r.note)
        ));
    }

    // --- Gráficas ---
    let chart_vx = chart(v, true);
    let chart_vt = chart(v, false);

    // --- Fases (del primer tren disponible; el resto, resumen) ---
    let phases_html = phases_block(v);

    // --- Perfil de velocidades (infraestructura) ---
    let profile_html = format!(
        "<tr><td class=\"mono\">0,0 km</td><td class=\"mono\">{:.1} km</td><td>Vmàx del tren (sense dada CVM) <span class=\"provb\" style=\"color:#f85149\">No disponible</span></td></tr>",
        v.distance_km
    );

    // --- Velocidades comerciales observadas (GTFS) ---
    let mut obs_html = String::new();
    for o in &v.observed {
        obs_html.push_str(&format!(
            "<tr><td>{} → {}</td><td class=\"num\">{:.2} km</td><td class=\"num mono\">{}:{:02}</td><td class=\"num\">{:.0} km/h</td></tr>",
            esc(&o.from), esc(&o.to), o.dist_m / 1000.0, o.run_s / 60, o.run_s % 60, o.avg_kmh
        ));
    }
    if obs_html.is_empty() {
        obs_html = "<tr><td colspan=4 class=\"muted\">Sense horaris de referència per a aquesta ruta (ruta per graf).</td></tr>".into();
    }

    // --- Estaciones de la ruta ---
    let mut route_html = String::new();
    for s in &v.route {
        let arr = s.arr.map(fmt_min).unwrap_or_else(|| "—".into());
        route_html.push_str(&format!(
            "<tr><td>{}</td><td class=\"num mono\">{:.2} km</td><td class=\"num mono\">{}</td></tr>",
            esc(&s.name), s.cum_km, arr
        ));
    }

    let line_txt = v
        .line
        .as_ref()
        .map(|l| format!(" · línia {}", esc(l)))
        .unwrap_or_default();

    let times_note = if v.has_times {
        "Ruta d'un servei real del GTFS (amb horaris de referència)."
    } else {
        "Ruta calculada pel graf d'estacions (sense horari de referència directe)."
    };
    let adif_note = if v.adif_available {
        let ad = v.adif_distance_km.unwrap_or(0.0);
        let diff = ad - v.distance_km;
        let pct = if v.distance_km > 0.0 { diff / v.distance_km * 100.0 } else { 0.0 };
        format!(
            "<p class=\"muted\"><span class=\"qdot q-oficial\"></span> <b>CVM ADIF aplicada</b> · cobertura {:.0}% · Vmàx mín. infraestructura {} · distància GTFS {:.2} km vs ADIF {:.2} km ({}{:.2} km, {}{:.1}%)</p>",
            v.coverage_pct,
            v.min_vmax_kmh.map(|x| format!("{:.0} km/h", x)).unwrap_or_else(|| "—".into()),
            v.distance_km, ad,
            if diff >= 0.0 { "+" } else { "" }, diff,
            if pct >= 0.0 { "+" } else { "" }, pct,
        )
    } else {
        "<p class=\"muted\"><span class=\"qdot q-nd\"></span> Sense CVM ADIF (executa scripts/fetch_adif_cvm.py per activar-la).</p>".to_string()
    };
    let ltv_note = match &v.ltv_snapshot {
        Some(snap) => format!(
            "<p class=\"muted\"><span class=\"qdot q-estimacion\"></span> <b>LTV aplicades</b> (snapshot {}): {} al recorregut{}. <span class=\"muted\">Temporal/fechat.</span></p>",
            esc(snap),
            v.ltv_applied,
            v.min_ltv_kmh.map(|x| format!(" · mín {:.0} km/h", x)).unwrap_or_default(),
        ),
        None => String::new(),
    };
    let adif_note = format!("{adif_note}{ltv_note}");

    format!(
        r#"<div class="card">
    <h2>Resultat · {origin} → {dest}{line}</h2>
    <p class="muted">Distància ferroviària <b>{dist:.2} km</b> · dt {dt} s · {nstops} estacions</p>
    <p class="muted src-note">Font distància: {dsrc}</p>
    <p class="muted">{times_note}</p>
    {adif_note}
    <div class="grid-kpi" style="margin-top:12px">{cards}</div>
  </div>

  <div class="card">
    <h2>Comparació entre sèries</h2>
    <div class="scroll"><table>
      <thead><tr><th>Tren</th><th>Distància</th><th>Temps mínim</th><th>Vmàx assolida</th><th>Límit tren</th></tr></thead>
      <tbody>{comp}</tbody></table></div>
    <ul class="src" style="margin-top:12px">{notes}</ul>
  </div>

  <div class="cols">
    <div class="card">
      <h2>Velocitat · distància</h2>
      {chart_vx}
    </div>
    <div class="card">
      <h2>Velocitat · temps</h2>
      {chart_vt}
    </div>
  </div>

  <div class="card">
    <h2>Fases del moviment</h2>
    {phases_html}
  </div>

  <div class="cols">
    <div class="card">
      <h2>Perfil de velocitats (infraestructura)</h2>
      <div class="scroll"><table><thead><tr><th>Inici</th><th>Fi</th><th>Vmàx</th></tr></thead><tbody>{profile_html}</tbody></table></div>
      <p class="muted" style="margin-top:8px">Sense CVM d'Adif no es defineixen límits per tram (no s'inventen). Vegeu les velocitats comercials observades a la dreta com a dada real de context.</p>
    </div>
    <div class="card">
      <h2>Velocitat comercial observada (GTFS)</h2>
      <div class="scroll"><table><thead><tr><th>Tram</th><th>Distància</th><th>Temps real</th><th>V mitjana</th></tr></thead><tbody>{obs_html}</tbody></table></div>
      <p class="muted" style="margin-top:8px">Mitjana real inclou acceleració/frenada però NO és una Vmàx.</p>
    </div>
  </div>

  <div class="card">
    <h2>Estacions de la ruta</h2>
    <div class="scroll"><table><thead><tr><th>Estació</th><th>PK acumulat (aprox.)</th><th>Arribada (GTFS)</th></tr></thead><tbody>{route_html}</tbody></table></div>
  </div>"#,
        origin = esc(&v.origin_name),
        dest = esc(&v.dest_name),
        line = line_txt,
        dist = v.distance_km,
        dt = v.dt,
        nstops = v.route.len(),
        dsrc = esc(&v.distance_source),
        times_note = times_note,
        adif_note = adif_note,
        cards = cards,
        comp = comp,
        notes = notes,
        chart_vx = chart_vx,
        chart_vt = chart_vt,
        phases_html = phases_html,
        profile_html = profile_html,
        obs_html = obs_html,
        route_html = route_html,
    )
}

fn fmt_min(sec: u32) -> String {
    let h = sec / 3600;
    let m = (sec % 3600) / 60;
    format!("{:02}:{:02}", h, m)
}

/// Bloque de fases: tabla por serie disponible.
fn phases_block(v: &CalcView) -> String {
    let mut out = String::new();
    for r in v.results.iter().filter(|r| r.available) {
        let mut rows = String::new();
        for p in &r.phases {
            rows.push_str(&format!(
                "<tr><td class=\"mono\">{}–{}</td><td>{}</td><td class=\"muted\">{}</td></tr>",
                mmss(p.t0),
                mmss(p.t1),
                esc(&p.kind),
                esc(&p.detail)
            ));
        }
        out.push_str(&format!(
            "<div class=\"ficha\"><h4><span class=\"dot\" style=\"background:{}\"></span>Sèrie {} · {}</h4>\
             <div class=\"scroll\"><table><thead><tr><th>Interval</th><th>Fase</th><th>Detall</th></tr></thead><tbody>{}</tbody></table></div></div>",
            series_color(&r.id), esc(&r.id), mmss(r.time_s), rows
        ));
    }
    if out.is_empty() {
        out = "<p class=\"muted\">Cap sèrie disponible seleccionada.</p>".into();
    }
    out
}

// --------------------------------------------------------------------------
// Gráficas SVG (superponen todas las series disponibles)
// --------------------------------------------------------------------------

/// `by_distance`: true → velocidad·distancia (eje X km); false → velocidad·tiempo (X s).
fn chart(v: &CalcView, by_distance: bool) -> String {
    let avail: Vec<&TrainResult> = v.results.iter().filter(|r| r.available).collect();
    if avail.is_empty() {
        return "<p class=\"muted\">Sense dades.</p>".into();
    }
    let w = 900.0;
    let h = 320.0;
    let (pl, pr, pt, pb) = (52.0, 16.0, 16.0, 40.0);
    let plot_w = w - pl - pr;
    let plot_h = h - pt - pb;

    fn data(r: &TrainResult, by_distance: bool) -> &Vec<(f64, f64)> {
        if by_distance {
            &r.xv
        } else {
            &r.tv
        }
    }

    let xmax = avail
        .iter()
        .flat_map(|r| data(r, by_distance).iter().map(|(x, _)| *x))
        .fold(0.0f64, f64::max)
        .max(1e-3);
    let ymax = avail
        .iter()
        .flat_map(|r| data(r, by_distance).iter().map(|(_, y)| *y))
        .fold(0.0f64, f64::max)
        .max(1.0);
    // Redondear ymax hacia arriba a múltiplo de 20.
    let ymax = (ymax / 20.0).ceil() * 20.0;

    let sx = |x: f64| pl + plot_w * (x / xmax);
    let sy = |y: f64| pt + plot_h * (1.0 - y / ymax);

    // Rejilla Y.
    let mut grid = String::new();
    let ysteps = (ymax / 20.0).round() as i32;
    for i in 0..=ysteps {
        let yv = 20.0 * i as f64;
        let yy = sy(yv);
        grid.push_str(&format!(
            "<line x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" class=\"grid\"/><text x=\"{:.1}\" y=\"{:.1}\" class=\"axis\" text-anchor=\"end\">{:.0}</text>",
            pl, yy, w - pr, yy, pl - 6.0, yy + 4.0, yv
        ));
    }
    // Etiquetas X (5 marcas).
    for i in 0..=5 {
        let xv = xmax * i as f64 / 5.0;
        let xx = sx(xv);
        let lab = if by_distance { format!("{:.1}", xv) } else { mmss(xv) };
        grid.push_str(&format!(
            "<text x=\"{:.1}\" y=\"{:.1}\" class=\"axis\" text-anchor=\"middle\">{}</text>",
            xx,
            h - 14.0,
            esc(&lab)
        ));
    }

    // Marcadores de estación (solo en velocidad·distancia).
    let mut markers = String::new();
    if by_distance {
        for s in &v.route {
            let xx = sx(s.cum_km);
            markers.push_str(&format!(
                "<line x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" class=\"stationmark\"><title>{}</title></line>",
                xx, pt, xx, pt + plot_h, esc(&s.name)
            ));
        }
    }

    // Líneas por serie.
    let mut lines = String::new();
    let mut legend = String::new();
    for (i, r) in avail.iter().enumerate() {
        let pts = data(r, by_distance);
        if pts.is_empty() {
            continue;
        }
        let mut d = String::new();
        for (j, (x, y)) in pts.iter().enumerate() {
            d.push_str(&format!(
                "{} {:.1} {:.1}",
                if j == 0 { "M" } else { "L" },
                sx(*x),
                sy(*y)
            ));
        }
        lines.push_str(&format!(
            "<path d=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"2\"/>",
            d,
            series_color(&r.id)
        ));
        legend.push_str(&format!(
            "<g transform=\"translate({:.0},{:.0})\"><rect width=\"11\" height=\"11\" rx=\"2\" fill=\"{}\"/><text x=\"15\" y=\"10\" class=\"axis\" style=\"fill:var(--text)\">{}</text></g>",
            pl + 8.0 + i as f64 * 74.0,
            pt + 4.0,
            series_color(&r.id),
            esc(&r.id)
        ));
    }

    let xlabel = if by_distance { "Distància (km)" } else { "Temps (mm:ss)" };
    format!(
        r##"<svg viewBox="0 0 {w:.0} {h:.0}" preserveAspectRatio="xMidYMid meet" class="chart" role="img" aria-label="{xlabel}">
  <style>.stationmark{{stroke:var(--border);stroke-width:1;stroke-dasharray:3 3;}}</style>
  {grid}
  {markers}
  {lines}
  {legend}
  <text x="{xcap:.0}" y="{ycap:.0}" class="axis" text-anchor="middle">{xlabel} · Y: km/h</text>
</svg>"##,
        w = w,
        h = h,
        grid = grid,
        markers = markers,
        lines = lines,
        legend = legend,
        xlabel = xlabel,
        xcap = pl + plot_w / 2.0,
        ycap = h - 2.0,
    )
}
