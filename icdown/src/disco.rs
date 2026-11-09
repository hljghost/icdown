//! Espacio libre en el disco destino.
use std::path::Path;

/// Margen que siempre se deja libre: 1 GiB.
pub const MARGEN: u64 = 1 << 30;

pub fn espacio_libre(ruta: &Path) -> std::io::Result<u64> {
    fs2::available_space(ruta)
}

pub fn cabe(ruta: &Path, bytes: u64) -> bool {
    espacio_libre(ruta).map(|l| l >= bytes.saturating_add(MARGEN)).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    #[test]
    fn hay_espacio_en_temp() {
        let t = std::env::temp_dir();
        assert!(super::espacio_libre(&t).unwrap() > 0);
        assert!(super::cabe(&t, 1));
        assert!(!super::cabe(&t, u64::MAX / 2));
    }
}
