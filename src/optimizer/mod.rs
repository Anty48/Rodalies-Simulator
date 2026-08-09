//! Motor de optimización de horarios: función de potencial + búsqueda iterativa.

pub mod potential;
pub mod search;

pub use potential::PotentialWeights;
pub use search::{optimize_line, optimize_line_cb, LineOptResult, SearchConfig};

use crate::gtfs_loader::Network;
use rayon::prelude::*;

/// Optimiza varias líneas (en paralelo) con la misma configuración.
pub fn optimize_lines(
    net: &Network,
    lines: &[String],
    service_id: &str,
    sc: SearchConfig,
    w: PotentialWeights,
) -> Vec<LineOptResult> {
    lines
        .par_iter()
        .filter_map(|l| optimize_line(net, l, service_id, sc, w))
        .collect()
}
