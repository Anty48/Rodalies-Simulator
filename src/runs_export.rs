//! Exportación de los resultados de `/numeros` (varias runs del motor headless) a **Excel (.xlsx)**
//! y **PDF**, opcionalmente con los **horarios usados** (el horario del día que consumió el motor,
//! `game::build_schedule`, sin los puntos de paso sintéticos).
//!
//! Sin dependencias nuevas: el .xlsx se escribe a mano (Office Open XML mínimo: libro + hojas con
//! celdas en línea + estilo negrita) empaquetado con el crate `zip` que ya usa la actualización del
//! GTFS; el PDF con `printpdf` (fuentes base Helvetica → texto en WinAnsi, ver `pdf_safe`).

use std::io::{Cursor, Write};

use printpdf::{BuiltinFont, IndirectFontRef, Mm, PdfDocument, PdfDocumentReference, PdfLayerReference};
use serde::Deserialize;

use crate::game::GameTrain;
use crate::gtfs_loader::Network;

// ---- Petición (lo que ya tiene el navegador tras correr las runs) ----------------------------

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct RunInc {
    pub motivo: String,
    pub effect: String,
    pub from_name: String,
    pub to_name: String,
    pub t0_hm: String,
    pub t1_hm: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct RunRow {
    pub seed: f64,
    pub n_trains: f64,
    pub max_delay_min: f64,
    pub peak_waiting: f64,
    pub final_waiting: f64,
    pub pax_min: f64,
    pub train_min_delay: f64,
    pub incidents: Vec<RunInc>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct TplInfo {
    pub motivo: String,
    pub effect: String,
    pub dur_min: f64,
    pub prob: f64,
    pub every_h: f64,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct ExportReq {
    pub runs: Vec<RunRow>,
    /// Plantillas de incidencia SELECCIONADAS en esa tanda (escenario reproducible).
    pub templates: Vec<TplInfo>,
    /// Variabilidad de velocidad de la tanda (% de Vmax); `None` = apagada (todos a Vmax).
    pub speed_var_pct: Option<f64>,
}

/// Metadatos del escenario que añade el servidor (de qué GTFS salen los horarios).
pub struct Meta {
    pub generated: String,
    pub gtfs_date: String,
    pub service_id: String,
}

fn effect_label(e: &str) -> &'static str {
    match e {
        "red_one" => "Señal roja (1 sentido)",
        "red_both" => "Señal roja (2 sentidos)",
        "single" => "Vía única",
        _ => "Otro",
    }
}

/// Texto del parámetro de variabilidad de velocidad para el escenario exportado.
fn speed_var_label(p: Option<f64>) -> String {
    match p {
        Some(p) if p > 0.0 => format!("Activada · factor {p:.1} % a 120 km/h (media Vmax-{p:.1} %, entre Vmax-{:.1} % y Vmax; escala con la Vmax, sorteo por tramo entre señales)", 2.0 * p),
        _ => "Desactivada (todos a Vmax)".into(),
    }
}

fn hms(s: u32) -> String {
    format!("{:02}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
}

/// Paradas REALES (sin puntos de paso sintéticos del troceo en cantones) de cada tren, ordenados
/// por línea y hora de salida: es el horario que ha consumido el motor.
fn sorted_trains(sched: &[GameTrain]) -> Vec<&GameTrain> {
    let mut v: Vec<&GameTrain> = sched.iter().filter(|t| t.stops.iter().filter(|s| !s.pass).count() >= 2).collect();
    v.sort_by(|a, b| a.line.cmp(&b.line).then(a.stops[0].d.cmp(&b.stops[0].d)).then(a.train.cmp(&b.train)));
    v
}

// ==============================================================================================
// XLSX
// ==============================================================================================

enum Cell {
    /// Texto normal.
    S(String),
    /// Texto en negrita (cabeceras).
    H(String),
    N(f64),
}

struct Sheet {
    name: String,
    widths: Vec<f64>,
    rows: Vec<Vec<Cell>>,
}

fn xml_esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            c if (c as u32) < 0x20 && c != '\t' && c != '\n' => {}
            c => o.push(c),
        }
    }
    o
}

/// Letra(s) de columna Excel (0 → A, 25 → Z, 26 → AA).
fn col_ref(mut i: usize) -> String {
    let mut s = String::new();
    loop {
        s.insert(0, (b'A' + (i % 26) as u8) as char);
        if i < 26 {
            break;
        }
        i = i / 26 - 1;
    }
    s
}

fn sheet_xml(sh: &Sheet) -> String {
    let mut x = String::with_capacity(64 * sh.rows.len() * 4 + 512);
    x.push_str(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">"#);
    // Cabecera fija al hacer scroll.
    x.push_str(r#"<sheetViews><sheetView workbookViewId="0"><pane ySplit="1" topLeftCell="A2" activePane="bottomLeft" state="frozen"/></sheetView></sheetViews>"#);
    if !sh.widths.is_empty() {
        x.push_str("<cols>");
        for (i, w) in sh.widths.iter().enumerate() {
            x.push_str(&format!(r#"<col min="{0}" max="{0}" width="{1}" customWidth="1"/>"#, i + 1, w));
        }
        x.push_str("</cols>");
    }
    x.push_str("<sheetData>");
    for (ri, row) in sh.rows.iter().enumerate() {
        x.push_str(&format!(r#"<row r="{}">"#, ri + 1));
        for (ci, c) in row.iter().enumerate() {
            let r = format!("{}{}", col_ref(ci), ri + 1);
            match c {
                Cell::S(s) => x.push_str(&format!(r#"<c r="{r}" t="inlineStr"><is><t xml:space="preserve">{}</t></is></c>"#, xml_esc(s))),
                Cell::H(s) => x.push_str(&format!(r#"<c r="{r}" t="inlineStr" s="1"><is><t xml:space="preserve">{}</t></is></c>"#, xml_esc(s))),
                Cell::N(v) if v.is_finite() => x.push_str(&format!(r#"<c r="{r}"><v>{v}</v></c>"#)),
                Cell::N(_) => {}
            }
        }
        x.push_str("</row>");
    }
    x.push_str("</sheetData></worksheet>");
    x
}

fn write_xlsx(sheets: &[Sheet]) -> Result<Vec<u8>, String> {
    let mut zw = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opt = zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let put = |zw: &mut zip::ZipWriter<Cursor<Vec<u8>>>, name: &str, body: &str| -> Result<(), String> {
        zw.start_file(name, opt).map_err(|e| e.to_string())?;
        zw.write_all(body.as_bytes()).map_err(|e| e.to_string())
    };
    let mut ct = String::from(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>"#);
    let mut wb = String::from(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets>"#);
    let mut rels = String::from(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#);
    for (i, sh) in sheets.iter().enumerate() {
        let n = i + 1;
        ct.push_str(&format!(r#"<Override PartName="/xl/worksheets/sheet{n}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#));
        wb.push_str(&format!(r#"<sheet name="{}" sheetId="{n}" r:id="rId{n}"/>"#, xml_esc(&sh.name)));
        rels.push_str(&format!(r#"<Relationship Id="rId{n}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{n}.xml"/>"#));
    }
    ct.push_str("</Types>");
    wb.push_str("</sheets></workbook>");
    rels.push_str(&format!(r#"<Relationship Id="rId{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#, sheets.len() + 1));

    put(&mut zw, "[Content_Types].xml", &ct)?;
    put(&mut zw, "_rels/.rels", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#)?;
    put(&mut zw, "xl/workbook.xml", &wb)?;
    put(&mut zw, "xl/_rels/workbook.xml.rels", &rels)?;
    // Estilos mínimos: 0 = normal, 1 = negrita (cabeceras).
    put(&mut zw, "xl/styles.xml", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="2"><font><sz val="11"/><name val="Calibri"/></font><font><b/><sz val="11"/><name val="Calibri"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="0" fontId="1" fillId="0" borderId="0" xfId="0" applyFont="1"/></cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles></styleSheet>"#)?;
    for (i, sh) in sheets.iter().enumerate() {
        put(&mut zw, &format!("xl/worksheets/sheet{}.xml", i + 1), &sheet_xml(sh))?;
    }
    let cur = zw.finish().map_err(|e| e.to_string())?;
    Ok(cur.into_inner())
}

fn h(s: &str) -> Cell {
    Cell::H(s.to_string())
}
fn s(x: impl Into<String>) -> Cell {
    Cell::S(x.into())
}

pub fn xlsx(req: &ExportReq, meta: &Meta, sched: Option<(&Network, &[GameTrain])>) -> Result<Vec<u8>, String> {
    let mut sheets = Vec::new();

    // 1) Runs (una fila por semilla).
    let mut rows = vec![vec![
        h("Semilla"), h("Trenes"), h("Retraso máx (min)"), h("Pico retenidos"), h("Final retenidos"),
        h("PAX·min"), h("Tren·min de retraso"), h("Nº incidencias"),
    ]];
    for r in &req.runs {
        rows.push(vec![
            Cell::N(r.seed), Cell::N(r.n_trains), Cell::N((r.max_delay_min * 10.0).round() / 10.0),
            Cell::N(r.peak_waiting), Cell::N(r.final_waiting), Cell::N(r.pax_min.round()),
            Cell::N(r.train_min_delay.round()), Cell::N(r.incidents.len() as f64),
        ]);
    }
    sheets.push(Sheet { name: "Runs".into(), widths: vec![9.0, 9.0, 17.0, 15.0, 15.0, 13.0, 19.0, 14.0], rows });

    // 2) Incidencias de cada run (dónde y cuándo).
    let mut rows = vec![vec![h("Semilla"), h("Motivo"), h("Efecto"), h("Desde"), h("Hasta"), h("Inicio"), h("Fin")]];
    for r in &req.runs {
        for i in &r.incidents {
            rows.push(vec![
                Cell::N(r.seed), s(&i.motivo), s(effect_label(&i.effect)), s(&i.from_name), s(&i.to_name),
                s(&i.t0_hm), s(&i.t1_hm),
            ]);
        }
    }
    sheets.push(Sheet { name: "Incidencias".into(), widths: vec![9.0, 30.0, 22.0, 30.0, 30.0, 9.0, 9.0], rows });

    // 3) Escenario: de dónde salen los horarios + plantillas seleccionadas (reproducible).
    let mut rows = vec![
        vec![h("Parámetro"), h("Valor")],
        vec![s("Generado"), s(&meta.generated)],
        vec![s("GTFS (fecha del conjunto)"), s(&meta.gtfs_date)],
        vec![s("Servicio (service_id)"), s(&meta.service_id)],
        vec![s("Runs"), Cell::N(req.runs.len() as f64)],
        vec![s("Variabilidad de velocidad"), s(&speed_var_label(req.speed_var_pct))],
        vec![],
        vec![h("Plantilla"), h("Efecto"), h("Duración (min)"), h("Probabilidad"), h("Cada (h)")],
    ];
    if req.templates.is_empty() {
        rows.push(vec![s("(ninguna: jornadas sin incidencias)")]);
    }
    for t in &req.templates {
        rows.push(vec![s(&t.motivo), s(effect_label(&t.effect)), Cell::N(t.dur_min), Cell::N(t.prob), Cell::N(t.every_h)]);
    }
    sheets.push(Sheet { name: "Escenario".into(), widths: vec![30.0, 24.0, 15.0, 13.0, 10.0], rows });

    // 4) Horarios usados (opcional): una fila por tren y parada real.
    if let Some((net, sched)) = sched {
        let mut rows = vec![vec![h("Línea"), h("Tren"), h("Nº parada"), h("stop_id"), h("Estación"), h("Llegada"), h("Salida")]];
        for t in sorted_trains(sched) {
            for (k, st) in t.stops.iter().filter(|x| !x.pass).enumerate() {
                rows.push(vec![
                    s(&t.line), s(&t.train), Cell::N((k + 1) as f64), s(&st.s), s(net.stop_name(&st.s)),
                    s(hms(st.a)), s(hms(st.d)),
                ]);
            }
        }
        sheets.push(Sheet { name: "Horarios".into(), widths: vec![8.0, 14.0, 10.0, 10.0, 34.0, 10.0, 10.0], rows });
    }
    write_xlsx(&sheets)
}

// ==============================================================================================
// PDF
// ==============================================================================================

/// Las fuentes base del PDF usan WinAnsi (Latin-1 + algunos signos): los acentos del castellano/
/// catalán salen bien, pero flechas y símbolos matemáticos no → se transliteran.
fn pdf_safe(t: &str) -> String {
    t.chars()
        .map(|c| match c {
            '→' => "->".to_string(),
            '←' => "<-".to_string(),
            '≥' => ">=".to_string(),
            '≤' => "<=".to_string(),
            '′' => "'".to_string(),
            c if (c as u32) < 0x100 || "—–‘’“”…€•".contains(c) => c.to_string(),
            _ => "?".to_string(),
        })
        .collect()
}

/// Cursor de escritura sobre un documento PDF paginado (A4). Salta de página solo.
struct Pdf {
    doc: PdfDocumentReference,
    font: IndirectFontRef,
    bold: IndirectFontRef,
    layer: PdfLayerReference,
    w: f32,
    h: f32,
    y: f32,
}

impl Pdf {
    const MARGIN: f32 = 14.0;

    fn new(title: &str) -> Result<Self, String> {
        let (w, h) = (210.0, 297.0);
        let (doc, p, l) = PdfDocument::new(title, Mm(w), Mm(h), "capa");
        let font = doc.add_builtin_font(BuiltinFont::Helvetica).map_err(|e| e.to_string())?;
        let bold = doc.add_builtin_font(BuiltinFont::HelveticaBold).map_err(|e| e.to_string())?;
        let layer = doc.get_page(p).get_layer(l);
        Ok(Pdf { doc, font, bold, layer, w, h, y: h - Self::MARGIN })
    }
    fn page(&mut self, w: f32, h: f32) {
        let (p, l) = self.doc.add_page(Mm(w), Mm(h), "capa");
        self.layer = self.doc.get_page(p).get_layer(l);
        self.w = w;
        self.h = h;
        self.y = h - Self::MARGIN;
    }
    /// Reserva `dy` mm; si no cabe, página nueva (mismo tamaño).
    fn need(&mut self, dy: f32) {
        if self.y - dy < Self::MARGIN {
            let (w, h) = (self.w, self.h);
            self.page(w, h);
        }
    }
    fn text_at(&self, t: &str, size: f32, x: f32, y: f32, bold: bool) {
        let f = if bold { &self.bold } else { &self.font };
        self.layer.use_text(pdf_safe(t), size, Mm(x), Mm(y), f);
    }
    fn line(&mut self, t: &str, size: f32, bold: bool) {
        let dy = size * 0.45 + 1.4;
        self.need(dy);
        self.y -= dy;
        self.text_at(t, size, Self::MARGIN, self.y, bold);
    }
    /// Fila de tabla: celdas en columnas con x fijas (mm desde el margen).
    fn row(&mut self, cells: &[String], xs: &[f32], size: f32, bold: bool) {
        let dy = size * 0.45 + 1.6;
        self.need(dy);
        self.y -= dy;
        for (c, x) in cells.iter().zip(xs) {
            self.text_at(c, size, Self::MARGIN + x, self.y, bold);
        }
    }
    fn gap(&mut self, dy: f32) {
        self.y -= dy;
    }
}

/// Recorta a `n` caracteres (celdas estrechas de la tabla).
fn clip(t: &str, n: usize) -> String {
    if t.chars().count() <= n {
        t.to_string()
    } else {
        t.chars().take(n.saturating_sub(1)).collect::<String>() + "."
    }
}

fn stats(v: impl Iterator<Item = f64>) -> Option<(f64, f64, f64)> {
    let v: Vec<f64> = v.collect();
    if v.is_empty() {
        return None;
    }
    let mean = v.iter().sum::<f64>() / v.len() as f64;
    let min = v.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = v.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    Some((mean, min, max))
}

fn es_num(x: f64) -> String {
    // Separador de miles con punto (convención es-ES), sin decimales.
    let n = x.round() as i64;
    let s = n.abs().to_string();
    let mut o = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            o.push('.');
        }
        o.push(c);
    }
    if n < 0 {
        format!("-{o}")
    } else {
        o
    }
}

pub fn pdf(req: &ExportReq, meta: &Meta, sched: Option<(&Network, &[GameTrain])>) -> Result<Vec<u8>, String> {
    let mut p = Pdf::new("Rodalies Simulator · resultados de runs")?;
    p.line("Rodalies Simulator · Resultados de varias runs", 16.0, true);
    p.line("Motor unificado headless (física + incidencias). Mismo motor que el modo visual.", 8.5, false);
    p.gap(2.0);
    p.line(&format!("Generado: {}   ·   GTFS del {}   ·   service_id {}", meta.generated, meta.gtfs_date, meta.service_id), 8.5, false);
    p.line(&format!("Runs: {}", req.runs.len()), 8.5, false);
    p.line(&format!("Variabilidad de velocidad: {}", speed_var_label(req.speed_var_pct)), 8.5, false);

    // Resumen estadístico.
    p.gap(3.0);
    p.line("Resumen", 11.0, true);
    let xs = [0.0, 52.0, 82.0, 112.0];
    p.row(&["Métrica".into(), "Media".into(), "Mín".into(), "Máx".into()], &xs, 8.0, true);
    let add = |name: &str, it: Vec<f64>, p: &mut Pdf| {
        if let Some((m, lo, hi)) = stats(it.into_iter()) {
            // Magnitudes grandes (PAX·min) con separador de miles; las pequeñas con 1 decimal.
            let f = |x: f64| if x.abs() >= 1000.0 { es_num(x) } else { format!("{x:.1}") };
            p.row(&[name.into(), f(m), f(lo), f(hi)], &xs, 8.0, false);
        }
    };
    add("Retraso máx (min)", req.runs.iter().map(|r| r.max_delay_min).collect(), &mut p);
    add("Pico retenidos", req.runs.iter().map(|r| r.peak_waiting).collect(), &mut p);
    add("PAX·min", req.runs.iter().map(|r| r.pax_min).collect(), &mut p);
    add("Nº incidencias", req.runs.iter().map(|r| r.incidents.len() as f64).collect(), &mut p);

    // Plantillas seleccionadas.
    p.gap(3.0);
    p.line("Incidencias en juego (plantillas seleccionadas)", 11.0, true);
    if req.templates.is_empty() {
        p.line("Ninguna: jornadas sin incidencias.", 8.5, false);
    } else {
        let xs = [0.0, 70.0, 115.0, 140.0];
        p.row(&["Motivo".into(), "Efecto".into(), "Duración".into(), "Probabilidad".into()], &xs, 8.0, true);
        for t in &req.templates {
            p.row(
                &[clip(&t.motivo, 40), effect_label(&t.effect).into(), format!("{:.0} min", t.dur_min), format!("{:.0} % cada {} h", t.prob * 100.0, t.every_h)],
                &xs, 8.0, false,
            );
        }
    }

    // Tabla de runs.
    p.gap(3.0);
    p.line("Runs", 11.0, true);
    let xs = [0.0, 18.0, 36.0, 64.0, 90.0, 116.0, 146.0];
    p.row(
        &["Semilla".into(), "Trenes".into(), "Retr. máx".into(), "Pico ret.".into(), "Final ret.".into(), "PAX·min".into(), "Incid.".into()],
        &xs, 8.0, true,
    );
    for r in &req.runs {
        p.row(
            &[
                format!("{}", r.seed), format!("{}", r.n_trains), format!("{:.0} min", r.max_delay_min),
                format!("{}", r.peak_waiting), format!("{}", r.final_waiting), es_num(r.pax_min), format!("{}", r.incidents.len()),
            ],
            &xs, 8.0, false,
        );
    }

    // Incidencias de cada run: dónde y cuándo.
    p.gap(3.0);
    p.line("Incidencias de cada run (dónde y cuándo)", 11.0, true);
    let xs = [0.0, 16.0, 40.0, 100.0];
    p.row(&["Semilla".into(), "Hora".into(), "Motivo · efecto".into(), "Tramo".into()], &xs, 8.0, true);
    let mut any = false;
    for r in &req.runs {
        for i in &r.incidents {
            any = true;
            p.row(
                &[
                    format!("{}", r.seed), format!("{}-{}", i.t0_hm, i.t1_hm),
                    clip(&format!("{} · {}", i.motivo, effect_label(&i.effect)), 34),
                    clip(&format!("{} → {}", i.from_name, i.to_name), 48),
                ],
                &xs, 7.5, false,
            );
        }
    }
    if !any {
        p.line("Ninguna run tuvo incidencias.", 8.5, false);
    }

    if let Some((net, sched)) = sched {
        timetable_pages(&mut p, net, sched);
    }
    p.doc.save_to_bytes().map_err(|e| e.to_string())
}

/// Horarios usados: A4 apaisado, una tabla por LÍNEA y SENTIDO (filas = trenes, columnas =
/// estaciones del recorrido más largo de ese sentido), paginada. Mismo formato que las mallas del
/// optimizador (`exporter.rs`), pero sobre el horario del motor (`GameTrain`).
fn timetable_pages(p: &mut Pdf, net: &Network, sched: &[GameTrain]) {
    use std::collections::BTreeMap;
    let trains = sorted_trains(sched);
    // Los ids de tren del feed comparten un prefijo largo (servicio + raíz): se quita el prefijo
    // COMÚN para que la etiqueta de fila muestre la parte que distingue a cada tren.
    let prefix_len = {
        let mut it = trains.iter().map(|t| t.train.as_str());
        let first = it.next().unwrap_or("");
        let mut n = first.len();
        for t in it {
            n = n.min(first.bytes().zip(t.bytes()).take_while(|(a, b)| a == b).count());
        }
        // No comerse todo: deja al menos 5 caracteres del id más corto.
        let min_len = trains.iter().map(|t| t.train.len()).min().unwrap_or(0);
        n.min(min_len.saturating_sub(5))
    };
    let label = |t: &GameTrain| -> String { t.train.get(prefix_len..).unwrap_or(&t.train).to_string() };
    let mut by_line: BTreeMap<&str, Vec<&GameTrain>> = BTreeMap::new();
    for t in trains {
        by_line.entry(t.line.as_str()).or_default().push(t);
    }
    let (w, h) = (297.0f32, 210.0f32);
    for (line, ts) in by_line {
        let stops = |t: &GameTrain| -> Vec<String> { t.stops.iter().filter(|s| !s.pass).map(|s| s.s.clone()).collect() };
        let Some(canon) = ts.iter().max_by_key(|t| stops(t).len()) else { continue };
        let fwd: Vec<String> = stops(canon);
        let pos = |id: &str| fwd.iter().position(|x| x == id);
        // Sentido: ida si su primera parada aparece antes que la última en el recorrido canónico.
        let (mut ida, mut vuelta) = (Vec::new(), Vec::new());
        for t in &ts {
            let st = stops(t);
            match (pos(&st[0]), pos(st.last().unwrap())) {
                (Some(a), Some(b)) if a > b => vuelta.push(*t),
                _ => ida.push(*t),
            }
        }
        let rev: Vec<String> = fwd.iter().rev().cloned().collect();
        for (cols, group, sentido) in [(&fwd, ida, "ida"), (&rev, vuelta, "vuelta")] {
            if group.is_empty() {
                continue;
            }
            let x_first = 24.0f32;
            let colw = ((w - 2.0 * Pdf::MARGIN - x_first) / cols.len().max(1) as f32).max(3.0);
            let fs: f32 = if colw < 6.5 { 4.3 } else if colw < 9.0 { 5.2 } else { 6.2 };
            let per_page = (((h - 2.0 * Pdf::MARGIN - 22.0) / 4.2).floor() as usize).max(1);
            let chunks: Vec<&[&GameTrain]> = group.chunks(per_page).collect();
            let n = chunks.len();
            for (pi, chunk) in chunks.iter().enumerate() {
                p.page(w, h);
                let first = net.stop_name(&cols[0]);
                let last = net.stop_name(cols.last().unwrap());
                p.text_at(&format!("Horario usado · Línea {line} · {sentido}: {first} → {last}   (pág. {}/{n})", pi + 1), 11.0, Pdf::MARGIN, h - 12.0, true);
                p.text_at("Filas = trenes · columnas = estaciones · hora de salida (HH:MM). Horario GTFS consumido por el motor.", 7.0, Pdf::MARGIN, h - 17.0, false);
                // Cabecera ESCALONADA en dos alturas (pares arriba, impares abajo): cada nombre
                // dispone de 2 columnas de ancho sin pisar al vecino. Capacidad ≈ ancho / (0,55·pt).
                let top = h - 29.0;
                let hs = (fs - 0.3).max(3.8);
                let cap = ((2.0 * colw - 1.0) / (0.55 * hs * 0.3528)).floor().max(4.0) as usize;
                p.text_at("Tren", fs, Pdf::MARGIN, top, true);
                for (ci, sid) in cols.iter().enumerate() {
                    let name = net.stop_name(sid).trim_start_matches("Barcelona").trim_start_matches('-').trim().to_string();
                    let ab: String = name.chars().take(cap).collect();
                    let y = if ci % 2 == 0 { top + 3.2 } else { top };
                    p.text_at(&ab, hs, Pdf::MARGIN + x_first + ci as f32 * colw, y, true);
                }
                for (ri, t) in chunk.iter().enumerate() {
                    let y = top - (ri as f32 + 1.0) * 4.2;
                    p.text_at(&clip(&label(t), 11), fs, Pdf::MARGIN, y, false);
                    for st in t.stops.iter().filter(|s| !s.pass) {
                        if let Some(ci) = cols.iter().position(|c| c == &st.s) {
                            let hm = format!("{:02}:{:02}", st.d / 3600, (st.d % 3600) / 60);
                            p.text_at(&hm, fs, Pdf::MARGIN + x_first + ci as f32 * colw, y, false);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columnas_excel() {
        assert_eq!(col_ref(0), "A");
        assert_eq!(col_ref(25), "Z");
        assert_eq!(col_ref(26), "AA");
        assert_eq!(col_ref(27), "AB");
    }

    #[test]
    fn xlsx_es_un_zip_valido() {
        let req = ExportReq {
            runs: vec![RunRow { seed: 1.0, n_trains: 961.0, incidents: vec![RunInc { motivo: "A & <B>".into(), ..Default::default() }], ..Default::default() }],
            templates: vec![],
            speed_var_pct: Some(3.0),
        };
        let meta = Meta { generated: "x".into(), gtfs_date: "y".into(), service_id: "z".into() };
        let bytes = xlsx(&req, &meta, None).unwrap();
        assert_eq!(&bytes[..2], b"PK");
        let mut z = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut sh = String::new();
        std::io::Read::read_to_string(&mut z.by_name("xl/worksheets/sheet2.xml").unwrap(), &mut sh).unwrap();
        assert!(sh.contains("A &amp; &lt;B&gt;"));
    }

    #[test]
    fn pdf_translitera_simbolos() {
        assert_eq!(pdf_safe("Sants → Clot ≥ 1′"), "Sants -> Clot >= 1'");
        assert_eq!(pdf_safe("Estació · Lleida"), "Estació · Lleida");
    }
}
