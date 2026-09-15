//! Pesos y componentes del potencial de coste V(H) de una propuesta de horario `H`.
//!
//! V(H) = w_delay · retard_ponderat_per_passatgers + w_conflict · conflictes_de_via
//!
//! Cuanto menor es V(H), mejor es el horario (menos retraso sufrido por los pasajeros y sin
//! cuellos de botella de señalización). Lo consume el optimizador del sistema
//! (`system::optimize_system`) sobre el **motor unificado** (`Engine::timeline` + `held_events`).

#[derive(Debug, Clone, Copy)]
pub struct PotentialWeights {
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
    pub fn factor(&self, t: u32) -> f64 {
        if self.is_peak(t) {
            self.peak_factor
        } else {
            1.0
        }
    }
}

/// Retraso ponderado por pasajeros a partir de la línea temporal del MOTOR UNIFICADO
/// (`Engine::timeline` = `(instante_s, retraso_total_s)` por minuto). Misma fórmula que
/// `passenger_weighted_delay` pero sobre el motor de la Fase B (la fuente de verdad). tren·min.
pub fn weighted_delay_from_timeline(timeline: &[(u32, f64)], w: &PotentialWeights) -> f64 {
    timeline.iter().map(|&(t, d)| (d / 60.0) * w.factor(t)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn punta_pesa_mas_que_valle() {
        let w = PotentialWeights::default();
        // Mismo retraso (600 s·tren) en hora punta vs. valle: la punta debe puntuar peor.
        let punta = weighted_delay_from_timeline(&[(8 * 3600, 600.0)], &w);
        let valle = weighted_delay_from_timeline(&[(13 * 3600, 600.0)], &w);
        assert!(punta > valle);
        assert!((punta - 10.0 * w.peak_factor).abs() < 1e-6); // 600 s = 10 min · factor punta
    }
}
