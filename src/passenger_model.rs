//! Modelo dinámico de pasajeros y cálculo de *dwell time* (tiempo de parada).
//!
//! El horario ya incluye el tiempo de parada teórico para el trasiego normal de
//! viajeros. Lo que amplifica el retraso es el EXCESO de pasajeros que se acumula
//! en el andén cuando el tren llega tarde: cuanto más tarde, más gente esperando y
//! más tiempo de subida/bajada por coche, de forma NO lineal (aglomeración).
//!
//! ```text
//! Dwell Real = Dwell Teòric + Δt_passatgers
//! passatgers_extra = tassa_base · retard_min           (flux base d'acumulació)
//! Δt_passatgers    = t_pax · (passatgers_extra / cotxes) · e^(k · retard_min)
//! ```
//!
//! Con retraso 0 el Δt es 0 ⇒ un tren puntual circula conforme al horario, y solo
//! las incidencias (o los conflictos de señalización) introducen retraso; así la
//! métrica de "retorno al equilibrio" tiene sentido.

use rand::Rng;

#[derive(Debug, Clone)]
pub struct PassengerModel {
    /// Flujo base de acumulación de pasajeros en el andén (pax/minuto de retraso).
    pub base_arrival_rate: f64,
    /// Segundos que tarda de media un pasajero en subir/bajar por coche.
    pub board_time_per_pax: f64,
    /// Número de coches del tren (reparte la carga entre puertas).
    pub cars: f64,
    /// Exponente de aglomeración: cuánto se dispara el dwell con el retraso.
    pub congestion_k: f64,
    /// Tope de seguridad para el dwell (segundos).
    pub max_dwell: u32,
}

impl Default for PassengerModel {
    fn default() -> Self {
        // Valores calibrados de forma plausible para cercanías densas.
        PassengerModel {
            base_arrival_rate: 4.0, // pax extra por minuto de retraso en un andén tipo
            board_time_per_pax: 0.8,
            cars: 4.0,
            congestion_k: 0.10,
            max_dwell: 600, // 10 min como techo
        }
    }
}

impl PassengerModel {
    /// Pasajeros EXTRA acumulados en el andén por culpa del retraso de llegada.
    /// A mayor retraso, más gente esperando.
    pub fn extra_passengers(&self, arrival_delay_secs: i64) -> f64 {
        let delay_min = arrival_delay_secs.max(0) as f64 / 60.0;
        (self.base_arrival_rate * delay_min).max(0.0)
    }

    /// Δt debido a los pasajeros extra. Crece exponencialmente con el retraso.
    pub fn passenger_delta(&self, arrival_delay_secs: i64) -> f64 {
        let extra = self.extra_passengers(arrival_delay_secs);
        let per_car = extra / self.cars.max(1.0);
        let delay_min = arrival_delay_secs.max(0) as f64 / 60.0;
        let congestion = (self.congestion_k * delay_min).exp();
        self.board_time_per_pax * per_car * congestion
    }

    /// Dwell real (determinista) = teórico + Δt_pasajeros, acotado a [0, max].
    pub fn dwell_time(&self, theoretical_dwell: u32, arrival_delay_secs: i64) -> u32 {
        let real = theoretical_dwell as f64 + self.passenger_delta(arrival_delay_secs);
        (real.round() as i64).clamp(0, self.max_dwell as i64) as u32
    }

    /// Variante estocástica: introduce ruido en la generación de pasajeros con `rand`.
    /// Útil para las iteraciones Monte Carlo de resiliencia.
    pub fn dwell_time_random<R: Rng + ?Sized>(
        &self,
        theoretical_dwell: u32,
        arrival_delay_secs: i64,
        rng: &mut R,
    ) -> u32 {
        // Ruido multiplicativo ±25% sobre la demanda extra de pasajeros.
        let noise = rng.gen_range(0.75..1.25);
        let extra = self.extra_passengers(arrival_delay_secs) * noise;
        let per_car = extra / self.cars.max(1.0);
        let delay_min = arrival_delay_secs.max(0) as f64 / 60.0;
        let congestion = (self.congestion_k * delay_min).exp();
        let delta = self.board_time_per_pax * per_car * congestion;
        let real = theoretical_dwell as f64 + delta;
        (real.round() as i64).clamp(0, self.max_dwell as i64) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_time_train_has_no_passenger_penalty() {
        let m = PassengerModel::default();
        // Sin retraso, el dwell real == teórico (no hay pasajeros extra).
        assert_eq!(m.dwell_time(30, 0), 30);
        assert_eq!(m.dwell_time(0, 0), 0);
    }

    #[test]
    fn dwell_grows_with_delay() {
        let m = PassengerModel::default();
        let d0 = m.dwell_time(30, 0);
        let d5 = m.dwell_time(30, 300);
        let d10 = m.dwell_time(30, 600);
        // A más retraso, más dwell (aglomeración creciente y exponencial).
        assert!(d0 < d5, "d0={} d5={}", d0, d5);
        assert!(d5 < d10, "d5={} d10={}", d5, d10);
        // La amplificación es superlineal: el salto 5→10 supera al 0→5.
        assert!((d10 - d5) > (d5 - d0), "no és superlineal: {} {} {}", d0, d5, d10);
    }

    #[test]
    fn dwell_respects_bounds() {
        let m = PassengerModel::default();
        assert!(m.dwell_time(30, 100_000) <= m.max_dwell);
    }

    #[test]
    fn random_dwell_stays_within_bounds() {
        use rand::rngs::StdRng;
        use rand::SeedableRng;
        let m = PassengerModel::default();
        let mut rng = StdRng::seed_from_u64(42);
        for _ in 0..1000 {
            let d = m.dwell_time_random(30, 300, &mut rng);
            assert!(d <= m.max_dwell);
        }
    }
}
