//! Actualizaciones automáticas: desactivadas en Linux (stub). Se gestionarán con el gestor de
//! paquetes de la distribución o más adelante con un mecanismo propio.

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Siempre `false` en Linux.
pub fn permitidas() -> bool {
    false
}
