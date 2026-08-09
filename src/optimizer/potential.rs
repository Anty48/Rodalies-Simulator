//! Función de potencial de coste V(H) de una propuesta de horario `H`.
//!
//! V(H) = w_reg · irregularitat_de_freqüències
//!      + w_delay · retard_ponderat_per_passatgers
//!      + w_conflict · conflictes_de_via
//!
//! Cuanto menor es V(H), mejor es el horario (más regular, menos retraso sufrido por
//! los pasajeros y sin cuellos de botella de señalización).

use crate::simulation_engine::SimResult;

#[derive(Debug, Clone, Copy)]
pub struct PotentialWeights {
    /// Peso de la irregularidad de intervalos (por minuto de desviación).
    pub w_regularity: f64,
    /// Peso del retraso ponderado por pasajeros (por tren·minuto).
    pub w_delay: f64,
    /// Peso (severo) de cada conflicto de vía / retención por señal roja.
    pub w_conflict: f64,
    /// Franja punta [inicio, fin) en segundos desde medianoche.
    pub peak_start: u32,
    pub peak_end: u32,
    /// Factor multiplicador en hora punta (p.ej. 3.0 = penalización triple).
    pub peak_factor: f64,
}

impl Default for PotentialWeights {
    fn default() -> Self {
        PotentialWeights {
            w_regularity: 1.0,
            w_delay: 0.02,
            w_conflict: 6.0,
            peak_start: 7 * 3600,
            peak_end: 9 * 3600 + 30 * 60, // 09:30
            peak_factor: 3.0,
        }
    }
}

impl PotentialWeights {
    fn is_peak(&self, t: u32) -> bool {
        t >= self.peak_start && t < self.peak_end
    }
    fn factor(&self, t: u32) -> f64 {
        if self.is_peak(t) {
            self.peak_factor
        } else {
            1.0
        }
    }
}

/// 1) Regularidad de frecuencias: desviación (ponderada en punta) de los intervalos
/// entre salidas consecutivas de la línea, en minutos. Los intervalos cuyo punto medio
/// cae en hora punta pesan `peak_factor` veces más.
pub fn regularity_cost(origin_times_sorted: &[u32], w: &PotentialWeights) -> f64 {
    if origin_times_sorted.len() < 3 {
        return 0.0;
    }
    let headways: Vec<f64> = origin_times_sorted
        .windows(2)
        .map(|p| (p[1] - p[0]) as f64)
        .collect();
    let mean = headways.iter().sum::<f64>() / headways.len() as f64;
    let mut num = 0.0;
    let mut den = 0.0;
    for (i, &h) in headways.iter().enumerate() {
        let mid = origin_times_sorted[i] + (h as u32) / 2;
        let wt = w.factor(mid);
        num += wt * (h - mean).powi(2);
        den += wt;
    }
    if den == 0.0 {
        return 0.0;
    }
    (num / den).sqrt() / 60.0 // en minutos
}

/// 3) Retraso ponderado por pasajeros: integral del retraso acumulado de la red en el
/// tiempo, multiplicada por el factor de intensidad de pasajeros (mayor en punta).
/// Devuelve tren·minutos ponderados.
pub fn passenger_weighted_delay(sim: &SimResult, w: &PotentialWeights) -> f64 {
    let mut cost = 0.0;
    for s in &sim.timeline {
        // Cada muestra representa 60 s; total_delay está en segundos-tren.
        cost += (s.total_delay as f64 / 60.0) * w.factor(s.time);
    }
    cost
}

/// V(H) completo a partir de las salidas de origen y de un resultado de simulación
/// (que incluye la penalización por conflicto vía `held_events`).
pub fn potential(origin_times_sorted: &[u32], sim: &SimResult, w: &PotentialWeights) -> f64 {
    let reg = regularity_cost(origin_times_sorted, w);
    let delay = passenger_weighted_delay(sim, w);
    // 4) Penalización por conflicto: cada retención por señal (competencia por vía).
    let conflict = sim.held_events as f64;
    w.w_regularity * reg + w.w_delay * delay + w.w_conflict * conflict
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regular_schedule_beats_irregular() {
        let w = PotentialWeights::default();
        // Intervalos perfectos de 10 min.
        let regular: Vec<u32> = (0..6).map(|i| 7 * 3600 + i * 600).collect();
        // Intervalos irregulares.
        let irregular: Vec<u32> = vec![25200, 25260, 27000, 27060, 28800, 30600];
        assert!(regularity_cost(&regular, &w) < regularity_cost(&irregular, &w));
    }
}
