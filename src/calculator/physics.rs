//! Modelo físico del movimiento del tren (integración por pasos con **frenado
//! anticipado**). No calcula `t = d / v`: simula el movimiento paso a paso mirando
//! hacia delante para empezar a frenar a tiempo antes de cada reducción de velocidad
//! y de la estación destino (donde debe llegar a v = 0).
//!
//! ## Modelo de tracción (documentado)
//!
//! Renfe/Adif **no publican** curvas de esfuerzo tractor ni de aceleración por serie,
//! así que se usa un modelo de **potencia constante** con tope de arranque:
//!
//! ```text
//! a_tracc(v) = min( a_arranque , η · P / (m·(1+λ) · v) )
//! ```
//!
//! donde `P` = potencia (W), `m` = masa (kg), `η` = rendimiento, `λ` = factor de masa
//! rotativa y `a_arranque` = tope de aceleración a baja velocidad (adhesión/confort).
//! Es un modelo optimista: **no** incluye resistencia al avance (Davis) porque sus
//! coeficientes no están disponibles por serie y no se quieren inventar. Por eso el
//! resultado es un **tiempo mínimo teórico** (cota inferior), no un tiempo comercial.
//!
//! El frenado usa una deceleración de servicio constante `b` (parámetro del modelo).

/// Vehículo ya reducido a magnitudes SI para el integrador.
#[derive(Debug, Clone)]
pub struct Vehicle {
    pub vmax: f64,       // m/s
    pub power_w: f64,    // W
    pub mass_kg: f64,    // kg (masa en vacío)
    pub accel_start: f64, // m/s² (tope de arranque)
    pub decel: f64,      // m/s² (freno de servicio)
    pub efficiency: f64, // 0..1
    pub rotary: f64,     // factor de masa rotativa (p.ej. 0.10)
    /// Resistencia al avance tipo Davis: F_res = a + b·v + c·v² (N). 0 = desactivada.
    /// Los coeficientes por serie NO están publicados; cuando se usan, son ESTIMADOS.
    pub res_a: f64,
    pub res_b: f64,
    pub res_c: f64,
}

impl Vehicle {
    /// Masa efectiva (incluye inercia rotativa).
    fn mass_eff(&self) -> f64 {
        self.mass_kg * (1.0 + self.rotary)
    }

    /// Aceleración de tracción disponible a velocidad `v` (m/s²), modelo potencia
    /// constante con tope de arranque. `v` se satura por abajo para evitar dividir
    /// por cero en el arranque.
    pub fn accel_at(&self, v: f64) -> f64 {
        let v_eff = v.max(0.3);
        let a_power = self.efficiency * self.power_w / (self.mass_eff() * v_eff);
        a_power.min(self.accel_start)
    }

    /// Deceleración por resistencia al avance (m/s²) a velocidad `v`. 0 si desactivada.
    pub fn resist_accel(&self, v: f64) -> f64 {
        if self.res_a == 0.0 && self.res_b == 0.0 && self.res_c == 0.0 {
            return 0.0;
        }
        (self.res_a + self.res_b * v + self.res_c * v * v) / self.mass_eff()
    }
}

/// Construye la lista de restricciones a partir de un perfil de zonas: el inicio de cada
/// zona actúa como punto de reducción (para frenar a tiempo) y el destino con v=0.
pub fn restrictions_from_zones(zones: &[SpeedZone], length_m: f64) -> Vec<Restriction> {
    let mut r: Vec<Restriction> = zones.iter().map(|z| Restriction { x: z.from, v: z.vmax }).collect();
    r.push(Restriction { x: length_m, v: 0.0 });
    r
}

/// Una restricción de velocidad puntual: en la posición `x` (m) la velocidad no puede
/// superar `v` (m/s). El destino se modela como restricción con `v = 0`.
#[derive(Debug, Clone, Copy)]
pub struct Restriction {
    pub x: f64,
    pub v: f64,
}

/// Muestra de la trayectoria (para gráficas y análisis de fases).
#[derive(Debug, Clone, Copy)]
pub struct Trace {
    pub t: f64, // s
    pub x: f64, // m
    pub v: f64, // m/s
    pub a: f64, // m/s² (aceleración aplicada en el paso)
}

/// Resultado de la simulación de un recorrido.
#[derive(Debug, Clone)]
pub struct SimResult {
    pub time_s: f64,
    pub vmax_reached: f64, // m/s
    pub trace: Vec<Trace>,
    pub reached_end: bool,
}

/// Desglose del movimiento por fases (tiempos en s, distancias en m).
#[derive(Debug, Clone, Copy, Default)]
pub struct Breakdown {
    pub t_accel: f64,
    pub t_cruise: f64,
    pub t_brake: f64,
    pub d_accel: f64,
    pub d_cruise: f64,
    pub d_brake: f64,
}

/// Umbral de aceleración (m/s²) para clasificar una fase.
const A_PHASE: f64 = 0.06;

impl SimResult {
    /// Reparte el recorrido en aceleración / crucero / frenada sumando el tiempo y la
    /// distancia de cada paso según el signo de la aceleración aplicada.
    pub fn breakdown(&self) -> Breakdown {
        let mut b = Breakdown::default();
        let mut prev = Trace { t: 0.0, x: 0.0, v: 0.0, a: 0.0 };
        for s in &self.trace {
            let dt = (s.t - prev.t).max(0.0);
            let dx = (s.x - prev.x).max(0.0);
            if s.a > A_PHASE {
                b.t_accel += dt;
                b.d_accel += dx;
            } else if s.a < -A_PHASE {
                b.t_brake += dt;
                b.d_brake += dx;
            } else {
                b.t_cruise += dt;
                b.d_cruise += dx;
            }
            prev = *s;
        }
        b
    }
}

/// Perfil de velocidad de la infraestructura: límite (m/s) en función de la
/// posición `x` (m). Se representa por zonas `[from, to)` ordenadas por `from`.
#[derive(Debug, Clone, Copy)]
pub struct SpeedZone {
    pub from: f64, // m
    pub to: f64,   // m
    pub vmax: f64, // m/s
}

/// Límite de infraestructura en la posición `x`. Si no hay zona que la cubra,
/// devuelve `f64::INFINITY` (sin límite de vía conocido → manda el tren).
fn infra_limit(zones: &[SpeedZone], x: f64) -> f64 {
    for z in zones {
        if x >= z.from && x < z.to {
            return z.vmax;
        }
    }
    f64::INFINITY
}

/// Velocidad máxima permitida en `x` por el **frenado anticipado**: para cada
/// restricción futura (incluida la parada final) se calcula la velocidad desde la que
/// aún se puede frenar a tiempo, `sqrt(v_r² + 2·b·(x_r − x))`, y se toma la menor.
fn braking_cap(restrictions: &[Restriction], x: f64, decel: f64) -> f64 {
    let mut cap = f64::INFINITY;
    for r in restrictions {
        if r.x >= x {
            let c = (r.v * r.v + 2.0 * decel * (r.x - x)).max(0.0).sqrt();
            if c < cap {
                cap = c;
            }
        }
    }
    cap
}

/// Simula el recorrido `[0, length_m]` con el vehículo dado, el perfil de vía `zones`
/// y las `restrictions` (deben incluir el destino en `x = length_m, v = 0`). Integra
/// con paso temporal `dt` (s) usando velocidad media por paso (Heun/trapezoidal).
pub fn simulate(
    length_m: f64,
    veh: &Vehicle,
    zones: &[SpeedZone],
    restrictions: &[Restriction],
    dt: f64,
) -> SimResult {
    let mut x = 0.0f64;
    let mut v = 0.0f64;
    let mut t = 0.0f64;
    let mut vmax_reached = 0.0f64;
    let mut trace: Vec<Trace> = Vec::new();

    // Cota de seguridad para no colgarse si algo va mal (p.ej. potencia 0).
    let t_max = 6.0 * 3600.0;
    let eps = 1e-6;

    // Velocidad permitida en una posición dada (mínimo de: Vmax tren, vía, frenado).
    let allowed = |x: f64| -> f64 {
        veh.vmax
            .min(infra_limit(zones, x))
            .min(braking_cap(restrictions, x, veh.decel))
    };

    while x < length_m - eps && t < t_max {
        let v_allow = allowed(x);
        let a;
        let v_next;
        if v < v_allow - eps {
            // Acelerar, sin pasarse del límite permitido en este punto. La resistencia al
            // avance (si está activada) resta aceleración y limita la velocidad de equilibrio.
            a = veh.accel_at(v) - veh.resist_accel(v);
            v_next = (v + a * dt).max(0.0).min(v_allow);
        } else {
            // Seguir la envolvente de velocidad permitida. Como `v_allow` incluye la
            // curva de frenado `sqrt(v_r² + 2·b·(x_r−x))`, al avanzar x el límite baja
            // de forma continua y la velocidad lo sigue exactamente: esto reproduce el
            // frenado de servicio a −b (crucero cuando el límite es plano) sin la deriva
            // numérica de un esquema puramente reactivo.
            v_next = v_allow;
            a = (v_next - v) / dt;
        }
        // Avance con velocidad media del paso (trapezoidal).
        let dx = 0.5 * (v + v_next) * dt;

        // Si este paso rebasa el destino, hacemos un sub-paso exacto hasta `length_m`.
        // El frenado anticipado garantiza que venimos sobre la curva de frenado, así que
        // la velocidad de llegada calculada cinemáticamente es ~0 (evita el artefacto de
        // discretización de terminar con velocidad residual).
        if x + dx >= length_m {
            let rem = (length_m - x).max(0.0);
            let v_end = (v * v - 2.0 * veh.decel * rem).max(0.0).sqrt();
            let v_avg = 0.5 * (v + v_end);
            let dt_p = if v_avg > 1e-6 { rem / v_avg } else { dt };
            t += dt_p;
            x = length_m;
            v = v_end;
            if v > vmax_reached {
                vmax_reached = v;
            }
            trace.push(Trace { t, x, v, a: -veh.decel });
            break;
        }

        x += dx;
        v = v_next;
        t += dt;
        if v > vmax_reached {
            vmax_reached = v;
        }
        trace.push(Trace { t, x: x.min(length_m), v, a });

        // Si nos hemos parado sin llegar (no debería con buen frenado) y no podemos
        // acelerar, salir para no bucle infinito.
        if v <= eps && allowed(x) <= eps && x < length_m - eps {
            break;
        }
    }

    let reached_end = x >= length_m - 1.0;
    SimResult { time_s: t, vmax_reached, trace, reached_end }
}

// --------------------------------------------------------------------------
// Tests del modelo físico
// --------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    fn veh_447() -> Vehicle {
        Vehicle {
            vmax: 120.0 / 3.6,
            power_w: 2_400_000.0,
            mass_kg: 162_500.0,
            accel_start: 1.0,
            decel: 0.9,
            efficiency: 0.85,
            rotary: 0.10,
            res_a: 0.0,
            res_b: 0.0,
            res_c: 0.0,
        }
    }

    /// Restricción única: destino a `len` con v=0.
    fn dest_only(len: f64) -> Vec<Restriction> {
        vec![Restriction { x: len, v: 0.0 }]
    }

    #[test]
    fn nunca_supera_vmax_del_tren() {
        let veh = veh_447();
        let len = 30_000.0;
        let r = simulate(len, &veh, &[], &dest_only(len), 0.1);
        for s in &r.trace {
            assert!(s.v <= veh.vmax + 1e-3, "v={} > vmax={}", s.v, veh.vmax);
        }
    }

    #[test]
    fn nunca_supera_el_limite_de_infraestructura() {
        let veh = veh_447();
        let len = 20_000.0;
        // Zona central lenta a 60 km/h entre 8 y 12 km.
        let zones = vec![
            SpeedZone { from: 0.0, to: 8_000.0, vmax: veh.vmax },
            SpeedZone { from: 8_000.0, to: 12_000.0, vmax: 60.0 / 3.6 },
            SpeedZone { from: 12_000.0, to: len, vmax: veh.vmax },
        ];
        let mut r = dest_only(len);
        // Restricción de entrada a la zona lenta (para forzar el frenado anticipado).
        r.push(Restriction { x: 8_000.0, v: 60.0 / 3.6 });
        let res = simulate(len, &veh, &zones, &r, 0.1);
        for s in &res.trace {
            let lim = super::infra_limit(&zones, s.x);
            assert!(s.v <= lim + 0.2, "en x={} v={} supera límite {}", s.x, s.v, lim);
        }
    }

    #[test]
    fn llega_a_velocidad_cero() {
        let veh = veh_447();
        let len = 15_000.0;
        let r = simulate(len, &veh, &[], &dest_only(len), 0.1);
        assert!(r.reached_end, "no llegó al destino");
        assert!(r.trace.last().unwrap().v < 0.5, "llega con v={}", r.trace.last().unwrap().v);
    }

    #[test]
    fn frena_antes_de_una_reduccion() {
        let veh = veh_447();
        let len = 20_000.0;
        let vlim = 40.0 / 3.6;
        let restr = vec![
            Restriction { x: 10_000.0, v: vlim },
            Restriction { x: len, v: 0.0 },
        ];
        let res = simulate(len, &veh, &[], &restr, 0.1);
        // En el punto de la reducción, la velocidad no debe superar el límite.
        let near = res
            .trace
            .iter()
            .min_by(|a, b| (a.x - 10_000.0).abs().partial_cmp(&(b.x - 10_000.0).abs()).unwrap())
            .unwrap();
        assert!(near.v <= vlim + 0.3, "al llegar a la reducción v={} > {}", near.v, vlim);
    }

    #[test]
    fn recorrido_mas_largo_no_es_mas_rapido() {
        let veh = veh_447();
        let t1 = simulate(10_000.0, &veh, &[], &dest_only(10_000.0), 0.1).time_s;
        let t2 = simulate(20_000.0, &veh, &[], &dest_only(20_000.0), 0.1).time_s;
        assert!(t2 > t1, "20 km ({}) no tardó más que 10 km ({})", t2, t1);
    }

    #[test]
    fn estable_al_cambiar_dt() {
        let veh = veh_447();
        let len = 25_000.0;
        let t_a = simulate(len, &veh, &[], &dest_only(len), 0.1).time_s;
        let t_b = simulate(len, &veh, &[], &dest_only(len), 0.05).time_s;
        let rel = (t_a - t_b).abs() / t_b;
        assert!(rel < 0.02, "dt inestable: 0.1s={} vs 0.05s={} (rel={})", t_a, t_b, rel);
    }

    #[test]
    fn simetrico_al_invertir_restricciones_simetricas() {
        let veh = veh_447();
        let len = 18_000.0;
        // Restricción simétrica respecto al centro.
        let a = vec![
            Restriction { x: 6_000.0, v: 50.0 / 3.6 },
            Restriction { x: len, v: 0.0 },
        ];
        let b = vec![
            Restriction { x: len - 6_000.0, v: 50.0 / 3.6 },
            Restriction { x: len, v: 0.0 },
        ];
        let ta = simulate(len, &veh, &[], &a, 0.1).time_s;
        let tb = simulate(len, &veh, &[], &b, 0.1).time_s;
        assert!((ta - tb).abs() < 3.0, "no simétrico: {} vs {}", ta, tb);
    }

    #[test]
    fn mas_potencia_es_mas_rapido() {
        let len = 30_000.0;
        let base = veh_447();
        let mut potente = base.clone();
        potente.power_w *= 2.0;
        let t_base = simulate(len, &base, &[], &dest_only(len), 0.1).time_s;
        let t_pot = simulate(len, &potente, &[], &dest_only(len), 0.1).time_s;
        assert!(t_pot < t_base, "más potencia no fue más rápido");
    }

    #[test]
    fn la_resistencia_frena_al_tren() {
        let len = 25_000.0;
        let base = veh_447();
        let mut con_res = base.clone();
        con_res.res_a = 3000.0;
        con_res.res_b = 60.0;
        con_res.res_c = 6.0;
        let t_base = simulate(len, &base, &[], &dest_only(len), 0.1);
        let t_res = simulate(len, &con_res, &[], &dest_only(len), 0.1);
        assert!(t_res.time_s > t_base.time_s, "la resistencia debería aumentar el tiempo");
        // La velocidad de equilibrio con resistencia no supera la del tren.
        assert!(t_res.vmax_reached <= base.vmax + 1e-3);
    }

    #[test]
    fn zonas_a_restricciones_y_envolvente() {
        let veh = veh_447();
        let len = 20_000.0;
        // Perfil: 120 → 60 (8-12 km) → 120. Zonas + restricciones derivadas.
        let zones = vec![
            SpeedZone { from: 0.0, to: 8_000.0, vmax: veh.vmax },
            SpeedZone { from: 8_000.0, to: 12_000.0, vmax: 60.0 / 3.6 },
            SpeedZone { from: 12_000.0, to: len, vmax: veh.vmax },
        ];
        let restr = restrictions_from_zones(&zones, len);
        // El destino y el inicio de cada zona están presentes.
        assert!(restr.iter().any(|r| (r.x - len).abs() < 1.0 && r.v == 0.0));
        assert!(restr.iter().any(|r| (r.x - 8_000.0).abs() < 1.0 && (r.v - 60.0 / 3.6).abs() < 1e-6));
        let res = simulate(len, &veh, &zones, &restr, 0.1);
        // Nunca supera el límite de la zona lenta.
        for s in &res.trace {
            if s.x >= 8_000.0 && s.x <= 12_000.0 {
                assert!(s.v <= 60.0 / 3.6 + 0.2, "supera 60 km/h en zona lenta: {}", s.v * 3.6);
            }
        }
    }
}
