//! Sistema de señalización por cantones (block system) estricto.
//!
//! Modela los tres aspectos clásicos de la señalización ferroviaria y la regla de
//! capacidad 1 (un único tren por cantón y por andén):
//!
//! * **Verd (Green):** el cantó següent està lliure → el tren circula a la velocitat
//!   nominal del tram.
//! * **Groc (Yellow):** el recurs següent (l'andana d'arribada) està ocupat → el tren
//!   redueix la velocitat en el cantó actual per poder frenar a temps (temps de marxa
//!   multiplicat per `yellow_slowdown`).
//! * **Vermell (Red):** el cantó que vol ocupar està ocupat → el tren s'ha d'aturar
//!   abans d'entrar-hi; el temps d'espera s'acumula íntegrament com a retard.
//!
//! Este módulo contiene la lógica *pura* de decisión de aspecto y de tiempo de marxa;
//! el estado de ocupación lo mantiene el motor de simulación (`simulation_engine`),
//! que consulta estas funciones. Así la física de señalización queda aislada y testeable.

/// Aspecto de la señal a la entrada de un cantón.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aspect {
    /// Vía libre: velocidad nominal.
    Green,
    /// Anuncio de parada: reduce velocidad en el cantón actual.
    Yellow,
    /// Ocupado: parada completa antes del cantón.
    Red,
}

/// Parámetros de señalización.
#[derive(Debug, Clone, Copy)]
pub struct Signals {
    /// Factor multiplicador del tiempo de marcha bajo aspecto amarillo (>= 1.0).
    pub yellow_slowdown: f64,
}

impl Default for Signals {
    fn default() -> Self {
        Signals { yellow_slowdown: 1.4 }
    }
}

impl Signals {
    /// Decide el aspecto a la entrada de un cantón en función de:
    /// - `canton_free`: si el cantón que se quiere ocupar está libre (capacidad 1).
    /// - `platform_ahead_free`: si la andana de la estación de destino está libre.
    ///
    /// Prioridad: rojo (cantón ocupado) > amarillo (andana ocupada) > verde.
    pub fn aspect(canton_free: bool, platform_ahead_free: bool) -> Aspect {
        if !canton_free {
            Aspect::Red
        } else if !platform_ahead_free {
            Aspect::Yellow
        } else {
            Aspect::Green
        }
    }

    /// Tiempo de marcha efectivo por el cantón según el aspecto.
    /// En verde es el nominal; en amarillo se ralentiza; en rojo el tren no llega a
    /// entrar (el motor lo retiene), así que se devuelve el nominal por completitud.
    pub fn traversal_time(&self, nominal_secs: u32, aspect: Aspect) -> u32 {
        match aspect {
            Aspect::Green | Aspect::Red => nominal_secs,
            Aspect::Yellow => (nominal_secs as f64 * self.yellow_slowdown).round() as u32,
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aspect_priority() {
        assert_eq!(Signals::aspect(false, false), Aspect::Red);
        assert_eq!(Signals::aspect(false, true), Aspect::Red);
        assert_eq!(Signals::aspect(true, false), Aspect::Yellow);
        assert_eq!(Signals::aspect(true, true), Aspect::Green);
    }

    #[test]
    fn yellow_slows_down() {
        let s = Signals { yellow_slowdown: 1.5 };
        assert_eq!(s.traversal_time(120, Aspect::Green), 120);
        assert_eq!(s.traversal_time(120, Aspect::Yellow), 180);
    }
}
