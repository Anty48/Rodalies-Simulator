//! Seguimiento de retrasos y análisis de la RECUPERACIÓN del sistema tras un pico.
//!
//! A partir de la línea temporal del motor (retraso total de la red cada minuto + trenes en
//! servicio) construye dos series —retraso medio por tren y retraso acumulado de la red— y, sobre
//! cualquiera de ellas, detecta los picos destacables, recorta el tramo de bajada de cada uno y
//! ajusta tres modelos de recomposición comparando su R²:
//!
//! * **exponencial negativa** `y = b + A·e^(−t/τ)` — un sistema que drena su cola a un ritmo
//!   proporcional a lo que queda (típico de una red con holgura: cada tren recupera en sus
//!   márgenes). `τ = 1/k` es la **constante de tiempo**: en τ se ha recuperado el 63 %, en 3τ el 95 %.
//! * **parábola** `y = a·t² + b·t + c` — bajada que se acelera o se frena (p. ej. una cola que se
//!   vacía cuando la incidencia se levanta y los trenes se van desembalsando).
//! * **lineal** `y = m·t + c` — drenaje a ritmo constante (capacidad saturada: sale un tren por
//!   intervalo de bloqueo, pase lo que pase).
//!
//! Todo son funciones puras sobre `&[f64]` (sin motor ni red) → testeables aisladamente.

use serde::Serialize;

/// Fracción residual de la bajada que se considera "estable": al modelo ya solo le queda por
/// recorrer el 5 % de su caída total (≈ 3τ en la exponencial pura).
const RESIDUAL_FRAC: f64 = 0.05;
/// Ventana de la media móvil centrada para DETECTAR picos (la serie de 1 min es ruidosa por las
/// salidas/llegadas individuales; los ajustes usan los datos crudos).
const SMOOTH_HALF: usize = 2;
/// Tope de picos por serie (los más prominentes): más allá es ruido estructural del horario.
const MAX_PEAKS: usize = 6;
/// Longitud máxima del tramo de bajada analizado (min).
const MAX_SEGMENT_MIN: usize = 180;
/// Mínimo de puntos para que un ajuste de 3 parámetros sea informativo.
const MIN_POINTS: usize = 6;
/// Cuánto se mira hacia atrás (min) para buscar dónde empieza la subida del pico.
const PRE_WINDOW_MIN: usize = 60;
/// Ventana (min) antes de la subida sobre la que se promedia la "media previa" (normalidad de antes).
const PRE_MEAN_MIN: usize = 30;
/// Fracción del máximo de trenes en servicio por debajo de la cual la serie no se analiza (arranque
/// y cierre de la jornada: con 3 trenes en servicio la media salta con cualquier llegada).
const SERVICE_FRAC: f64 = 0.25;
/// Dos picos separados por ≤ este hueco (min) son el MISMO episodio (una incidencia que da un
/// doble golpe: se retiene, se suelta un poco y se vuelve a retener).
const MERGE_GAP_MIN: usize = 15;
/// … o si entre ellos la red no llegó a recuperar ni la mitad de lo subido (fracción del valle).
const MERGE_RECOVERY_FRAC: f64 = 0.5;
/// Tras la ventana de una incidencia se sigue mirando este margen (s) para medir su impacto: el
/// retraso sigue subiendo un rato después de levantarla (trenes embalsados).
const IMPACT_TAIL_S: f64 = 1800.0;

/// Series por minuto para el seguimiento de retrasos.
#[derive(Serialize, Clone, Debug)]
pub struct DelayTrack {
    /// Instante de la primera muestra (s desde medianoche) y paso entre muestras (s).
    pub t0: u32,
    pub dt: u32,
    /// Retraso medio por tren EN SERVICIO (min).
    pub mean: Vec<f64>,
    /// Retraso acumulado de la red = Σ retrasos vivos de todos los trenes (min).
    pub total: Vec<f64>,
    /// Trenes en servicio en cada muestra.
    pub active: Vec<u32>,
}

/// Construye las series del seguimiento a partir de la línea temporal del motor.
pub fn delay_track(timeline: &[(u32, f64)], active: &[u32]) -> DelayTrack {
    let r2 = |x: f64| (x * 100.0).round() / 100.0; // JSON compacto: 0,01 min basta
    let t0 = timeline.first().map(|p| p.0).unwrap_or(0);
    let dt = if timeline.len() > 1 { timeline[1].0 - timeline[0].0 } else { 60 };
    let mut mean = Vec::with_capacity(timeline.len());
    let mut total = Vec::with_capacity(timeline.len());
    for (i, &(_, d)) in timeline.iter().enumerate() {
        let n = active.get(i).copied().unwrap_or(0);
        total.push(r2(d / 60.0));
        mean.push(if n > 0 { r2(d / 60.0 / n as f64) } else { 0.0 });
    }
    DelayTrack { t0, dt, mean, total, active: active.to_vec() }
}

/// Seguimiento de UNA línea (mismo eje temporal que [`DelayTrack`]): retraso medio por tren de la
/// línea en servicio y retraso acumulado de sus trenes, por minuto. Sirve para separar qué línea
/// carga el retraso de la red y superponerlas para ver cómo se contagian.
#[derive(Serialize, Clone, Debug)]
pub struct LineTrack {
    pub line: String,
    /// Color oficial de la línea (`route_color` del GTFS).
    pub color: String,
    pub mean: Vec<f64>,
    pub total: Vec<f64>,
    pub active: Vec<u32>,
}

/// Series de una línea a partir de su retraso acumulado (s) y sus trenes en servicio por muestra.
pub fn line_track(line: &str, color: String, total_s: &[f32], active: &[u16]) -> LineTrack {
    let r2 = |x: f64| (x * 100.0).round() / 100.0;
    let total: Vec<f64> = total_s.iter().map(|&d| r2(d as f64 / 60.0)).collect();
    let mean = total
        .iter()
        .zip(active)
        .map(|(&t, &n)| if n > 0 { r2(t / n as f64) } else { 0.0 })
        .collect();
    LineTrack { line: line.to_string(), color, mean, total, active: active.iter().map(|&n| n as u32).collect() }
}

/// Impacto de una incidencia sobre la red: cuánto SUBE el retraso acumulado (min) desde que empieza
/// hasta su máximo dentro de `[t0, t1 + IMPACT_TAIL_S]`. Es la medida de gravedad REAL (lo que le
/// costó a la red), no la nominal (efecto × duración). Con incidencias solapadas ambas comparten
/// la subida: no se puede atribuir sin simular cada una por separado.
pub fn incident_impact_min(timeline: &[(u32, f64)], t0: f64, t1: f64) -> f64 {
    let end = t1 + IMPACT_TAIL_S;
    let mut base: Option<f64> = None;
    let mut peak = f64::MIN;
    for &(t, d) in timeline {
        let t = t as f64;
        if t + 60.0 <= t0 {
            base = Some(d); // última muestra antes del inicio (nivel previo)
            continue;
        }
        if t > end {
            break;
        }
        base.get_or_insert(d);
        peak = peak.max(d);
    }
    match base {
        Some(b) if peak > f64::MIN => ((peak - b) / 60.0).max(0.0),
        _ => 0.0,
    }
}

/// Análisis de recuperación de una jornada: sobre el RETRASO MEDIO por tren en servicio (el
/// retraso acumulado de la red = N(t)·medio(t) no se ajusta por separado: se usa para contrastar
/// las integrales). Solo dentro de la ventana de SERVICIO (trenes en servicio ≥ 25 % del máximo del
/// día): fuera, con 3 trenes, la media da picos espurios de arranque/cierre. Índices sobre la serie
/// completa; una muestra = 1 min (`SAMPLE_STEP_S` del motor).
pub fn analyze_track(timeline: &[(u32, f64)], active: &[u32]) -> Vec<PeakFit> {
    let n = timeline.len().min(active.len());
    let total: Vec<f64> = timeline[..n].iter().map(|p| p.1 / 60.0).collect();
    let w: Vec<f64> = active[..n].iter().map(|&a| a as f64).collect();
    let mean: Vec<f64> = (0..n).map(|i| if w[i] > 0.0 { total[i] / w[i] } else { 0.0 }).collect();
    let max = w.iter().copied().fold(0.0, f64::max);
    let ok = |i: &usize| max > 0.0 && w[*i] >= SERVICE_FRAC * max;
    let (Some(lo), Some(hi)) = ((0..n).find(ok), (0..n).rev().find(ok)) else {
        return Vec::new();
    };
    let mut v = analyze_weighted(&mean[lo..=hi], &w[lo..=hi], &total[lo..=hi]);
    for p in &mut v {
        p.i_start += lo;
        p.i_peak += lo;
        p.i_end += lo;
    }
    v
}

/// Un modelo ajustado sobre el tramo de bajada (t en minutos desde el pico).
#[derive(Serialize, Clone, Debug)]
pub struct Fit {
    /// "exp" | "parab" | "lin".
    pub model: &'static str,
    pub r2: f64,
    /// R² ajustado por nº de parámetros (lineal 2, los otros 3): criterio para elegir el mejor.
    pub r2_adj: f64,
    /// exp: [b, A, τ] · parab: [a, b, c] · lin: [m, c, 0].
    pub p: [f64; 3],
    /// TIEMPO HASTA ESTABILIDAD (min desde el pico): cuando al modelo ya solo le queda ≤ 5 % de su
    /// bajada total por recorrer → el retraso por minuto es prácticamente constante (en la
    /// exponencial, 3τ exactos). `None` si el modelo no baja.
    pub t_stab: Option<f64>,
    /// NUEVA NORMALIDAD (min): nivel en que se estabiliza el sistema tras el pico.
    pub new_level: Option<f64>,
    /// De dónde sale ese nivel: "model" (asíntota de la exponencial, vértice de la parábola) o
    /// "data" (meseta final observada: la recta y la parábola cóncava no tienen suelo propio).
    pub level_src: &'static str,
    /// HUELLA = nueva normalidad − media previa al pico (min por tren). > 0: el sistema queda peor
    /// que antes; ≈ 0: se recompone del todo; < 0: queda mejor (p. ej. baja la demanda).
    pub footprint: Option<f64>,
    /// ∫ modelo(t)·N(t) dt sobre el tramo (tren·min): el retraso acumulado que PREDICE el modelo.
    pub area: f64,
}

impl Fit {
    pub fn eval(&self, t: f64) -> f64 {
        let p = self.p;
        match self.model {
            "exp" => p[0] + p[1] * (-t / p[2]).exp(),
            "parab" => p[0] * t * t + p[1] * t + p[2],
            _ => p[0] * t + p[1],
        }
    }
}

/// Un pico destacable y su análisis de recuperación. Índices sobre la serie analizada.
#[derive(Serialize, Clone, Debug)]
pub struct PeakFit {
    /// Inicio de la subida (valle previo), pico y fin del tramo de bajada recortado.
    pub i_start: usize,
    pub i_peak: usize,
    pub i_end: usize,
    pub y_peak: f64,
    /// MEDIA PREVIA: media de la serie en los `PRE_MEAN_MIN` min anteriores a la subida (la
    /// normalidad de antes del pico, contra la que se mide la huella).
    pub pre_mean: f64,
    /// Meseta final OBSERVADA: media del último cuarto del tramo (suelo de la recta/parábola cóncava).
    pub tail_level: f64,
    pub prominence: f64,
    /// Nº de picos crudos agrupados en este episodio (1 = pico simple; 0 = tramo manual).
    pub merged: usize,
    /// ∫ retraso acumulado de la red dt sobre el tramo (tren·min): lo que pasó DE VERDAD.
    pub area_data: f64,
    /// ∫ (retraso acumulado − media previa·N) dt (tren·min): exceso de retraso sobre la normalidad
    /// previa durante la bajada = coste del pico para la red.
    pub area_excess: f64,
    pub fits: Vec<Fit>,
    /// Índice en `fits` del mejor modelo (mayor R² ajustado).
    pub best: usize,
}

/// Detecta los picos destacables de `y` y ajusta los tres modelos a la bajada de cada uno (sin
/// pesos: N = 1 y acumulado = y). Para tests y series sueltas.
#[cfg(test)]
pub fn analyze(y: &[f64]) -> Vec<PeakFit> {
    analyze_weighted(y, &vec![1.0; y.len()], y)
}

/// Núcleo: `y` = retraso medio (min), `w` = trenes en servicio, `total` = retraso acumulado (min)
/// en cada muestra (1 min). `total ≈ y·w`, pero se pasa aparte para no arrastrar redondeos.
fn analyze_weighted(y: &[f64], w: &[f64], total: &[f64]) -> Vec<PeakFit> {
    let n = y.len();
    if n < MIN_POINTS * 2 {
        return Vec::new();
    }
    let s = smooth(y, SMOOTH_HALF);
    let peaks = merge_episodes(&s, y, detect_peaks(&s, y));
    let mut out = Vec::new();
    for (k, pk) in peaks.iter().enumerate() {
        // El tramo termina en el valle antes del siguiente episodio (o del siguiente punto más
        // alto): así se recorta la recuperación sin mezclarla con la subida siguiente.
        let limit = peaks
            .get(k + 1)
            .map(|q| q.i_first)
            .unwrap_or(n - 1)
            .min(pk.right_higher)
            .min(pk.i_peak + MAX_SEGMENT_MIN);
        if limit <= pk.i_peak {
            continue;
        }
        let valley = argmin(&s, pk.i_peak + 1, limit);
        let y_peak = y[pk.i_peak];
        // Inicio de la subida: el valle LOCAL previo dentro de la última hora (sin cruzar el tramo
        // del pico anterior); el mínimo global del día (madrugada) no sirve, a mediodía la red nunca
        // vuelve a 0.
        let prev_end = out.last().map(|p: &PeakFit| p.i_end).unwrap_or(0);
        let lo = pk.i_first.saturating_sub(PRE_WINDOW_MIN).max(prev_end);
        let i_start = argmin(&s, lo, pk.i_first);
        let y_ref = s[i_start].min(y_peak);
        // Media previa: los `PRE_MEAN_MIN` minutos antes de que empiece a subir.
        let m0 = i_start.saturating_sub(PRE_MEAN_MIN).max(prev_end.min(i_start));
        let pre = &y[m0..=i_start];
        let pre_mean = pre.iter().sum::<f64>() / pre.len() as f64;
        // Fin del tramo: primer instante en que la serie suavizada vuelve al 5 % residual sobre el
        // valle previo, más una cola (½ de lo transcurrido) para que el ajuste vea la meseta; si no
        // vuelve antes del valle, el valle.
        let back_lvl = y_ref + RESIDUAL_FRAC * (y_peak - y_ref);
        let back = (pk.i_peak + 1..=valley).find(|&i| s[i] <= back_lvl);
        let i_end = match back {
            Some(i) => (i + ((i - pk.i_peak) / 2).max(MIN_POINTS)).min(valley.max(i)),
            None => valley,
        };
        if i_end < pk.i_peak + MIN_POINTS - 1 {
            continue;
        }
        if let Some(pf) = fit_segment(y, w, total, i_start, pk.i_peak, i_end, pre_mean, pk.prominence, pk.n) {
            out.push(pf);
        }
    }
    out
}

/// Ajusta la bajada `[i_peak, i_end]` y calcula integrales/exceso. Compartido por la detección
/// automática y el tramo MANUAL. `None` si el tramo es demasiado corto o plano.
#[allow(clippy::too_many_arguments)]
fn fit_segment(
    y: &[f64],
    w: &[f64],
    total: &[f64],
    i_start: usize,
    i_peak: usize,
    i_end: usize,
    pre_mean: f64,
    prominence: f64,
    merged: usize,
) -> Option<PeakFit> {
    if i_end < i_peak + MIN_POINTS - 1 {
        return None;
    }
    let seg = i_peak..=i_end;
    let ys = &y[seg.clone()];
    let ws = &w[seg.clone()];
    let tail = &ys[ys.len() - (ys.len() / 4).max(MIN_POINTS - 1).min(ys.len())..];
    let tail_level = tail.iter().sum::<f64>() / tail.len() as f64;
    let fits = fit_all(ys, ws, tail_level, pre_mean);
    if fits.is_empty() {
        return None;
    }
    let area_data = trapz(&total[seg.clone()]);
    let excess: Vec<f64> = total[seg].iter().zip(ws).map(|(t, n)| t - pre_mean * n).collect();
    let best = fits
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.r2_adj.total_cmp(&b.1.r2_adj))
        .map(|(i, _)| i)
        .unwrap_or(0);
    Some(PeakFit {
        i_start,
        i_peak,
        i_end,
        y_peak: y[i_peak],
        pre_mean,
        tail_level,
        prominence,
        merged,
        area_data,
        area_excess: trapz(&excess),
        fits,
        best,
    })
}

/// Tramo MANUAL marcado por el usuario: `[i0, i1]` = episodio completo (desde antes de que suba
/// hasta donde considera que ha acabado). El pico es el máximo del retraso medio dentro del tramo;
/// se ajusta la bajada pico → `i1`, y la media previa son los `PRE_MEAN_MIN` min antes de `i0`.
/// Series completas de la jornada (índices sobre ellas); `total` en min, `active` = trenes.
pub fn analyze_manual(total: &[f64], active: &[u32], i0: usize, i1: usize) -> Result<PeakFit, String> {
    let n = total.len().min(active.len());
    if n == 0 || i0 >= i1 || i1 >= n {
        return Err("Tramo fuera de la jornada: marca un inicio y un final dentro del gráfico.".into());
    }
    let w: Vec<f64> = active[..n].iter().map(|&a| a as f64).collect();
    let y: Vec<f64> = (0..n).map(|i| if w[i] > 0.0 { total[i] / w[i] } else { 0.0 }).collect();
    let i_peak = (i0..=i1).fold(i0, |m, i| if y[i] > y[m] { i } else { m });
    if i1 < i_peak + MIN_POINTS - 1 {
        return Err(format!(
            "Tras el pico ({} min dentro del tramo) quedan menos de {} min de bajada: alarga el final.",
            i_peak - i0,
            MIN_POINTS
        ));
    }
    let pre = &y[i0.saturating_sub(PRE_MEAN_MIN)..=i0];
    let pre_mean = pre.iter().sum::<f64>() / pre.len() as f64;
    fit_segment(&y, &w, &total[..n], i0, i_peak, i1, pre_mean, y[i_peak] - pre_mean, 0)
        .ok_or_else(|| "La bajada del tramo es plana: no hay nada que ajustar.".into())
}

/// Integral por trapecios con paso 1 (min).
fn trapz(v: &[f64]) -> f64 {
    v.windows(2).map(|p| 0.5 * (p[0] + p[1])).sum()
}

/// Ajusta exp / parábola / lineal a un tramo (t = 0,1,2… min) y deriva de cada modelo la
/// estabilidad, la nueva normalidad, la huella y la integral ponderada por `ws` (trenes).
/// `tail_level` = meseta observada (suelo para los modelos sin suelo propio). Vacío si es plano.
pub fn fit_all(ys: &[f64], ws: &[f64], tail_level: f64, pre_mean: f64) -> Vec<Fit> {
    let n = ys.len();
    let ts: Vec<f64> = (0..n).map(|i| i as f64).collect();
    let mean = ys.iter().sum::<f64>() / n as f64;
    let sst: f64 = ys.iter().map(|v| (v - mean).powi(2)).sum();
    if n < MIN_POINTS || sst <= 1e-12 {
        return Vec::new();
    }
    let span = (n - 1) as f64;
    let new_fit = |model, p, k: usize| {
        let mut f = Fit { model, r2: 0.0, r2_adj: 0.0, p, t_stab: None, new_level: None, level_src: "data", footprint: None, area: 0.0 };
        let sse: f64 = ts.iter().zip(ys).map(|(&t, &v)| (v - f.eval(t)).powi(2)).sum();
        f.r2 = 1.0 - sse / sst;
        f.r2_adj = 1.0 - (1.0 - f.r2) * (n as f64 - 1.0) / (n as f64 - k as f64 - 1.0).max(1.0);
        let wm: Vec<f64> = ts.iter().zip(ws).map(|(&t, &w)| f.eval(t) * w).collect();
        f.area = trapz(&wm);
        f
    };
    // Suelo tomado de los DATOS (recta, parábola cóncava o con vértice fuera de rango): el modelo
    // baja hasta la meseta observada; "estable" al quedar a ≤ 5 % de esa bajada.
    let data_floor = |f: &mut Fit| {
        let y0 = f.eval(0.0);
        if y0 > tail_level {
            let lvl = tail_level + RESIDUAL_FRAC * (y0 - tail_level);
            f.t_stab = match f.model {
                "lin" if f.p[0] < 0.0 => Some(((lvl - f.p[1]) / f.p[0]).max(0.0)),
                "parab" => first_positive_root(f.p[0], f.p[1], f.p[2] - lvl),
                _ => None,
            };
            if f.t_stab.is_some() {
                f.new_level = Some(tail_level);
                f.level_src = "data";
            }
        }
    };

    let mut out = Vec::with_capacity(3);
    if let Some(p) = fit_exp(&ts, ys) {
        let mut f = new_fit("exp", p, 3);
        // Asíntota propia b; le queda el 5 % de la bajada cuando e^(−t/τ) = 0,05 → t = τ·ln 20 ≈ 3τ.
        if p[1] > 0.0 {
            f.t_stab = Some(p[2] * (1.0 / RESIDUAL_FRAC).ln());
            f.new_level = Some(p[0]);
            f.level_src = "model";
        }
        out.push(f);
    }
    if let Some(p) = fit_poly2(&ts, ys) {
        let mut f = new_fit("parab", p, 3);
        // Parábola convexa (a > 0): la bajada se frena y se aplana en el vértice t_v = −b/2a; suelo
        // propio y(t_v). Resta 5 % cuando a(t−t_v)² = 5 %·a·t_v² → t = t_v·(1 − √0,05). Si el vértice
        // cae lejos del tramo (> 3× su longitud) la curvatura no es fiable → suelo de datos.
        let tv = if p[0] > 0.0 { -p[1] / (2.0 * p[0]) } else { f64::NAN };
        if tv > 0.0 && tv <= 3.0 * span {
            f.t_stab = Some(tv * (1.0 - RESIDUAL_FRAC.sqrt()));
            f.new_level = Some(f.eval(tv));
            f.level_src = "model";
        } else {
            data_floor(&mut f);
        }
        out.push(f);
    }
    if let Some((m, c)) = fit_linear(&ts, ys) {
        let mut f = new_fit("lin", [m, c, 0.0], 2);
        data_floor(&mut f);
        out.push(f);
    }
    for f in &mut out {
        f.footprint = f.new_level.map(|l| l - pre_mean);
    }
    out
}

// ---- detección de picos --------------------------------------------------------------------

/// Episodio de retraso: uno o varios picos crudos PEGADOS agrupados. `i_first` = primer pico (de
/// ahí se busca la subida y la media previa), `i_peak` = el más alto (de ahí arranca la bajada).
struct Episode {
    i_first: usize,
    i_last: usize,
    i_peak: usize,
    right_higher: usize,
    prominence: f64,
    n: usize,
}

/// Agrupa picos consecutivos que son el MISMO episodio: una incidencia a menudo da un doble golpe
/// (la red retiene, suelta un poco y vuelve a retener) y separarlos parte una sola recuperación en
/// dos tramos cortos y mal ajustados. Se fusionan si están a ≤ `MERGE_GAP_MIN` min o si en el valle
/// entre ellos la red no recuperó ni la mitad (`MERGE_RECOVERY_FRAC`) de lo subido desde el nivel
/// previo al episodio.
fn merge_episodes(s: &[f64], y: &[f64], peaks: Vec<RawPeak>) -> Vec<Episode> {
    let mut out: Vec<Episode> = Vec::new();
    for p in peaks {
        if let Some(e) = out.last_mut() {
            let base = s[argmin(s, e.i_first.saturating_sub(PRE_WINDOW_MIN), e.i_first)];
            let valley = s[argmin(s, e.i_last, p.i_peak)];
            let low = s[e.i_last].min(s[p.i_peak]);
            let recovered = if low - base > 1e-12 { (low - valley) / (low - base) } else { 1.0 };
            if p.i_peak.saturating_sub(e.i_last) <= MERGE_GAP_MIN || recovered < MERGE_RECOVERY_FRAC {
                if y[p.i_peak] > y[e.i_peak] {
                    e.i_peak = p.i_peak;
                }
                e.i_last = p.i_peak;
                e.right_higher = e.right_higher.max(p.right_higher);
                e.prominence = e.prominence.max(p.prominence);
                e.n += 1;
                continue;
            }
        }
        out.push(Episode { i_first: p.i_peak, i_last: p.i_peak, i_peak: p.i_peak, right_higher: p.right_higher, prominence: p.prominence, n: 1 });
    }
    out
}

struct RawPeak {
    i_peak: usize,
    right_higher: usize,
    prominence: f64,
}

/// Media móvil centrada de semiancho `h` (los bordes usan la ventana disponible).
fn smooth(y: &[f64], h: usize) -> Vec<f64> {
    let n = y.len();
    (0..n)
        .map(|i| {
            let (a, b) = (i.saturating_sub(h), (i + h).min(n - 1));
            y[a..=b].iter().sum::<f64>() / (b - a + 1) as f64
        })
        .collect()
}

fn argmin(s: &[f64], a: usize, b: usize) -> usize {
    let mut best = a.min(s.len() - 1);
    for i in a..=b.min(s.len() - 1) {
        if s[i] < s[best] {
            best = i;
        }
    }
    best
}

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    v[v.len() / 2]
}

/// Picos DESTACABLES por prominencia topográfica (lo que sobresale respecto al collado más alto
/// que lo separa de un punto más alto). Umbral = max(20 % del rango del día, 6σ del ruido), con
/// σ robusta (MAD) del residuo crudo − suavizado: así un día sin incidencias no inventa picos del
/// vaivén de salidas, y un día con varias no esconde las pequeñas tras la grande.
fn detect_peaks(s: &[f64], y: &[f64]) -> Vec<RawPeak> {
    let n = s.len();
    let (lo, hi) = s.iter().fold((f64::MAX, f64::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    let sigma = 1.4826 * median(y.iter().zip(s).map(|(a, b)| (a - b).abs()).collect());
    let thr = (0.2 * (hi - lo)).max(6.0 * sigma);
    if hi - lo <= 1e-9 {
        return Vec::new();
    }
    let mut peaks = Vec::new();
    let mut i = 1;
    while i + 1 < n {
        // Máximo local (una meseta cuenta una vez, en su primer punto).
        if s[i] > s[i - 1] && s[i] >= s[i + 1] {
            let mut j = i + 1;
            while j < n && s[j] == s[i] {
                j += 1;
            }
            if j < n && s[j] > s[i] {
                i = j;
                continue; // era un escalón de subida, no un pico
            }
            let (mut lmin, mut k) = (s[i], i);
            while k > 0 && s[k - 1] <= s[i] {
                k -= 1;
                lmin = lmin.min(s[k]);
            }
            let (mut rmin, mut k) = (s[i], i);
            while k + 1 < n && s[k + 1] <= s[i] {
                k += 1;
                rmin = rmin.min(s[k]);
            }
            let right_higher = k;
            let col = lmin.max(rmin);
            let prominence = s[i] - col;
            if prominence >= thr {
                // El pico "real" es el máximo crudo cercano (el suavizado lo desplaza y lo rebaja).
                let (a, b) = (i.saturating_sub(SMOOTH_HALF + 1), (i + SMOOTH_HALF + 1).min(n - 1));
                let ip = (a..=b).fold(a, |m, q| if y[q] > y[m] { q } else { m });
                peaks.push(RawPeak { i_peak: ip, right_higher, prominence });
            }
            i = j;
        } else {
            i += 1;
        }
    }
    peaks.sort_by(|a, b| b.prominence.total_cmp(&a.prominence));
    peaks.truncate(MAX_PEAKS);
    peaks.sort_by_key(|p| p.i_peak);
    peaks.dedup_by_key(|p| p.i_peak);
    peaks
}

// ---- ajustes por mínimos cuadrados ---------------------------------------------------------

fn fit_linear(ts: &[f64], ys: &[f64]) -> Option<(f64, f64)> {
    let n = ts.len() as f64;
    let (sx, sy) = (ts.iter().sum::<f64>(), ys.iter().sum::<f64>());
    let sxx: f64 = ts.iter().map(|t| t * t).sum();
    let sxy: f64 = ts.iter().zip(ys).map(|(t, y)| t * y).sum();
    let den = n * sxx - sx * sx;
    if den.abs() < 1e-12 {
        return None;
    }
    let m = (n * sxy - sx * sy) / den;
    Some((m, (sy - m * sx) / n))
}

/// Parábola por ecuaciones normales (3×3, eliminación de Gauss con pivoteo parcial).
fn fit_poly2(ts: &[f64], ys: &[f64]) -> Option<[f64; 3]> {
    let mut s = [0.0; 5]; // Σ t^k, k = 0..4
    let mut r = [0.0; 3]; // Σ y·t^k, k = 0..2
    for (&t, &y) in ts.iter().zip(ys) {
        let mut tk = 1.0;
        for k in 0..5 {
            s[k] += tk;
            if k < 3 {
                r[k] += y * tk;
            }
            tk *= t;
        }
    }
    // Incógnitas en orden [c, b, a] (potencias 0, 1, 2).
    let m = [[s[0], s[1], s[2]], [s[1], s[2], s[3]], [s[2], s[3], s[4]]];
    let x = solve3(m, r)?;
    Some([x[2], x[1], x[0]])
}

fn solve3(mut a: [[f64; 3]; 3], mut b: [f64; 3]) -> Option<[f64; 3]> {
    for col in 0..3 {
        let piv = (col..3).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[piv][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, piv);
        b.swap(col, piv);
        for row in col + 1..3 {
            let f = a[row][col] / a[col][col];
            for k in col..3 {
                a[row][k] -= f * a[col][k];
            }
            b[row] -= f * b[col];
        }
    }
    let mut x = [0.0; 3];
    for i in (0..3).rev() {
        let s: f64 = (i + 1..3).map(|k| a[i][k] * x[k]).sum();
        x[i] = (b[i] - s) / a[i][i];
    }
    Some(x)
}

/// Para τ fijo el modelo `b + A·e^(−t/τ)` es LINEAL en (b, A) → mínimos cuadrados cerrados. Se
/// barre τ en rejilla logarítmica y se refina con sección áurea sobre ln τ (la SSE es unimodal en la
/// práctica en torno al óptimo). Evita Gauss-Newton y sus problemas de arranque.
fn fit_exp(ts: &[f64], ys: &[f64]) -> Option<[f64; 3]> {
    let span = ts.last().copied().unwrap_or(1.0).max(1.0);
    let sse_at = |tau: f64| -> Option<(f64, f64, f64)> {
        let e: Vec<f64> = ts.iter().map(|t| (-t / tau).exp()).collect();
        let (mut a, mut b) = fit_linear(&e, ys)?; // y = A·e + b
        if b < 0.0 {
            // Un retraso no puede tender a un valor negativo: restricción física b ≥ 0 → con b = 0
            // el óptimo de A es cerrado (proyección de y sobre e).
            let ee: f64 = e.iter().map(|x| x * x).sum();
            if ee <= 1e-12 {
                return None;
            }
            b = 0.0;
            a = e.iter().zip(ys).map(|(x, y)| x * y).sum::<f64>() / ee;
        }
        let sse = e.iter().zip(ys).map(|(x, y)| (y - (a * x + b)).powi(2)).sum();
        Some((sse, b, a))
    };
    let (lo, hi) = ((0.2f64).ln(), (20.0 * span).ln());
    const GRID: usize = 120;
    let mut best: Option<(f64, usize)> = None;
    for g in 0..=GRID {
        let lt = lo + (hi - lo) * g as f64 / GRID as f64;
        if let Some((sse, _, _)) = sse_at(lt.exp()) {
            if best.map_or(true, |(b, _)| sse < b) {
                best = Some((sse, g));
            }
        }
    }
    let (_, g) = best?;
    let step = (hi - lo) / GRID as f64;
    let (mut a, mut b) = (lo + step * g.saturating_sub(1) as f64, lo + step * (g + 1).min(GRID) as f64);
    let phi = (5f64.sqrt() - 1.0) / 2.0;
    let f = |lt: f64| sse_at(lt.exp()).map_or(f64::MAX, |r| r.0);
    for _ in 0..60 {
        let (c, d) = (b - phi * (b - a), a + phi * (b - a));
        if f(c) < f(d) {
            b = d;
        } else {
            a = c;
        }
    }
    let tau = ((a + b) / 2.0).exp();
    let (_, bb, aa) = sse_at(tau)?;
    Some([bb, aa, tau])
}

/// Primera raíz t > 0 de `a·t² + b·t + c = 0` (o de la recta si a≈0).
fn first_positive_root(a: f64, b: f64, c: f64) -> Option<f64> {
    if c <= 0.0 {
        return Some(0.0); // ya está por debajo del nivel en el pico (tramo casi plano)
    }
    if a.abs() < 1e-12 {
        return if b < 0.0 { Some(-c / b) } else { None };
    }
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return None; // la parábola rebota antes de llegar al nivel
    }
    let sq = disc.sqrt();
    let mut roots = [(-b - sq) / (2.0 * a), (-b + sq) / (2.0 * a)];
    roots.sort_by(|x, y| x.total_cmp(y));
    roots.into_iter().find(|&t| t > 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn best_model(f: &[Fit]) -> &'static str {
        f.iter().max_by(|a, b| a.r2_adj.total_cmp(&b.r2_adj)).unwrap().model
    }

    #[test]
    fn exp_decay_recovers_tau_level_and_area() {
        let ys: Vec<f64> = (0..60).map(|t| 2.0 + 30.0 * (-(t as f64) / 12.0).exp()).collect();
        let ws = vec![10.0; ys.len()];
        let fits = fit_all(&ys, &ws, 2.1, 1.5);
        assert_eq!(best_model(&fits), "exp");
        let e = fits.iter().find(|f| f.model == "exp").unwrap();
        assert!((e.p[2] - 12.0).abs() < 0.1, "tau = {}", e.p[2]);
        assert!(e.r2 > 0.9999);
        // Estabilidad = τ·ln 20 ≈ 3τ; nueva normalidad = asíntota 2; huella = 2 − 1,5.
        assert!((e.t_stab.unwrap() - 12.0 * 20f64.ln()).abs() < 0.5);
        assert!((e.new_level.unwrap() - 2.0).abs() < 0.01);
        assert!((e.footprint.unwrap() - 0.5).abs() < 0.01);
        // La integral del modelo × N coincide con la de los datos × N (ajuste exacto).
        let data: f64 = trapz(&ys.iter().map(|y| y * 10.0).collect::<Vec<_>>());
        assert!((e.area - data).abs() / data < 1e-3);
    }

    #[test]
    fn linear_drain_uses_observed_floor() {
        let ys: Vec<f64> = (0..40).map(|t| 50.0 - 1.2 * t as f64).collect();
        let lin = fit_all(&ys, &vec![1.0; 40], 2.0, 2.0).into_iter().find(|f| f.model == "lin").unwrap();
        assert!((lin.p[0] + 1.2).abs() < 0.05);
        assert_eq!(lin.level_src, "data");
        // 5 % residual sobre la meseta 2: 2 + 0,05·48 = 4,4 → t = (4,4 − 50)/(−1,2) = 38.
        assert!((lin.t_stab.unwrap() - 38.0).abs() < 1.0);
    }

    #[test]
    fn convex_parabola_floor_is_vertex() {
        let ts: Vec<f64> = (0..30).map(|t| t as f64).collect();
        let ys: Vec<f64> = ts.iter().map(|t| 0.05 * t * t - 3.0 * t + 60.0).collect();
        let p = fit_poly2(&ts, &ys).unwrap();
        assert!((p[0] - 0.05).abs() < 1e-6 && (p[1] + 3.0).abs() < 1e-6 && (p[2] - 60.0).abs() < 1e-6);
        let f = fit_all(&ys, &vec![1.0; 30], 20.0, 10.0).into_iter().find(|f| f.model == "parab").unwrap();
        // Vértice en t = 30 → y = 15; estable en 30·(1 − √0,05) ≈ 23,3.
        assert_eq!(f.level_src, "model");
        assert!((f.new_level.unwrap() - 15.0).abs() < 1e-6);
        assert!((f.t_stab.unwrap() - 30.0 * (1.0 - 0.05f64.sqrt())).abs() < 1e-6);
    }

    #[test]
    fn detects_incident_peak_and_ignores_noise() {
        // Día plano con ruido de ±0,2 y una incidencia a los 300 min que sube a +20 y drena en exp.
        let y: Vec<f64> = (0..900)
            .map(|i| {
                let noise = ((i * 37 % 11) as f64 - 5.0) * 0.04;
                let inc = if (300..330).contains(&i) {
                    20.0 * (i - 300) as f64 / 30.0
                } else if i >= 330 {
                    20.0 * (-((i - 330) as f64) / 15.0).exp()
                } else {
                    0.0
                };
                3.0 + noise + inc
            })
            .collect();
        let peaks = analyze(&y);
        assert_eq!(peaks.len(), 1, "picos: {:?}", peaks.iter().map(|p| p.i_peak).collect::<Vec<_>>());
        let p = &peaks[0];
        assert!((328..=332).contains(&p.i_peak));
        assert!((p.pre_mean - 3.0).abs() < 0.1, "media previa {}", p.pre_mean);
        assert_eq!(p.fits[p.best].model, "exp");
        let e = &p.fits[p.best];
        assert!((e.p[2] - 15.0).abs() < 1.5, "tau = {}", e.p[2]);
        // Se recompone del todo: huella ≈ 0.
        assert!(e.footprint.unwrap().abs() < 0.3, "huella {:?}", e.footprint);
    }

    #[test]
    fn double_hit_is_one_episode() {
        // Una incidencia con doble golpe: sube a +12, baja a +8 (no recupera ni la mitad) y vuelve
        // a subir a +15 a los 25 min; luego drena. Debe salir UN episodio con la bajada del mayor.
        let y: Vec<f64> = (0..900)
            .map(|i| {
                let noise = ((i * 37 % 11) as f64 - 5.0) * 0.04;
                let x = i as f64;
                // Tramos lineales: 280→300 sube a +12, 300→312 baja a +8, 312→325 sube a +15,
                // después drena en exponencial (τ = 15 min).
                let inc = if x < 280.0 {
                    0.0
                } else if x < 300.0 {
                    12.0 * (x - 280.0) / 20.0
                } else if x < 312.0 {
                    12.0 - 4.0 * (x - 300.0) / 12.0
                } else if x < 325.0 {
                    8.0 + 7.0 * (x - 312.0) / 13.0
                } else {
                    15.0 * (-(x - 325.0) / 15.0).exp()
                };
                3.0 + noise + inc
            })
            .collect();
        let peaks = analyze(&y);
        assert_eq!(peaks.len(), 1, "picos: {:?}", peaks.iter().map(|p| p.i_peak).collect::<Vec<_>>());
        assert!(peaks[0].merged >= 2);
        assert!((322..=328).contains(&peaks[0].i_peak));
    }

    #[test]
    fn manual_segment_fits_decay_from_max() {
        // Retraso medio 0,3 + exp(−t/10)·2 a partir del minuto 100; 50 trenes.
        let active = vec![50u32; 300];
        let total: Vec<f64> = (0..300)
            .map(|i| 50.0 * (0.3 + if i >= 100 { 2.0 * (-((i - 100) as f64) / 10.0).exp() } else { 0.0 }))
            .collect();
        let p = analyze_manual(&total, &active, 90, 160).unwrap();
        assert_eq!(p.i_peak, 100);
        assert!((p.pre_mean - 0.3).abs() < 1e-9);
        let e = p.fits.iter().find(|f| f.model == "exp").unwrap();
        assert!((e.p[2] - 10.0).abs() < 0.2, "tau {}", e.p[2]);
        assert!(e.footprint.unwrap().abs() < 0.01);
        assert!(analyze_manual(&total, &active, 150, 152).is_err());
    }

    #[test]
    fn impact_is_rise_during_incident() {
        let tl: Vec<(u32, f64)> = (0..100).map(|i| (i * 60, if (40..60).contains(&i) { 600.0 } else { 60.0 })).collect();
        let imp = incident_impact_min(&tl, 40.0 * 60.0, 50.0 * 60.0);
        assert!((imp - 9.0).abs() < 1e-9);
    }
}
