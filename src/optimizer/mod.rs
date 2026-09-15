//! Motor de optimización de horarios: función de potencial + búsqueda del sistema entero.
//!
//! Un solo optimizador: `system::optimize_system` coordina TODAS las líneas a la vez (desfase de
//! fase por línea) sobre el **motor unificado** (`src/engine.rs`), que es el problema real
//! (conflictos entre líneas). El antiguo modo por línea (`search.rs`, sobre `simulation_engine`)
//! se retiró al cerrar la migración a Rust.

pub mod potential;
pub mod system;

pub use potential::PotentialWeights;
pub use system::{optimize_system, SystemResult, SystemSearch};
