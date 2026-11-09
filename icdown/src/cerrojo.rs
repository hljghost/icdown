//! Garantiza que solo un proceso trabaje sobre un destino a la vez.
use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;

pub struct Cerrojo {
    _archivo: File,
}

fn abrir(dir_datos: &Path) -> std::io::Result<File> {
    std::fs::create_dir_all(dir_datos)?;
    OpenOptions::new().create(true).read(true).write(true).truncate(false).open(dir_datos.join("motor.lock"))
}

/// `Ok(None)` si otro proceso ya lo tiene.
pub fn intentar(dir_datos: &Path) -> std::io::Result<Option<Cerrojo>> {
    let f = abrir(dir_datos)?;
    match f.try_lock() {
        Ok(()) => Ok(Some(Cerrojo { _archivo: f })),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(e)) => Err(e),
    }
}

/// Espera hasta conseguirlo.
pub fn esperar(dir_datos: &Path) -> std::io::Result<Cerrojo> {
    let f = abrir(dir_datos)?;
    f.lock()?;
    Ok(Cerrojo { _archivo: f })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusivo_mientras_vive() {
        let d = tempfile::tempdir().unwrap();
        let a = intentar(d.path()).unwrap();
        assert!(a.is_some());
        assert!(intentar(d.path()).unwrap().is_none());
        drop(a);
        assert!(intentar(d.path()).unwrap().is_some());
    }
}
