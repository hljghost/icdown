//! "Verificar todo": re-comprueba lo copiado y reprograma lo dañado o ausente.
use crate::layout;
use crate::registro::Registro;
use crate::verificar;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct ResultadoVerificacion {
    pub revisadas: usize,
    /// (ruta relativa, motivo)
    pub malas: Vec<(String, String)>,
}

pub fn verificar_todo(
    destino: &Path,
    registro: &Mutex<Registro>,
    cancelar: &AtomicBool,
    avance: &dyn Fn(usize, usize),
) -> anyhow::Result<ResultadoVerificacion> {
    let partes = registro.lock().unwrap().partes_verificadas()?;
    let total = partes.len();
    let mut r = ResultadoVerificacion::default();
    for p in &partes {
        if cancelar.load(Ordering::Relaxed) {
            break;
        }
        let motivo = match verificar::reverificar(&layout::en_disco(destino, &p.ruta), p.tamano, &p.extension, p.estructura_atipica) {
            Ok(sha) if sha == p.sha256 => None,
            Ok(_) => Some("el contenido cambió (SHA-256 distinto)".to_string()),
            Err(f) => Some(f.to_string()),
        };
        if let Some(mut m) = motivo {
            registro.lock().unwrap().marcar_para_redescarga(&p.asset_id, p.tipo, &m)?;
            if let Err(e) = apartar_danado(destino, &p.ruta) {
                m = format!("{m} (no se pudo apartar: {e})");
            }
            r.malas.push((p.ruta.clone(), m));
        }
        r.revisadas += 1;
        avance(r.revisadas, total);
    }
    Ok(r)
}

/// Mueve un archivo dañado a `<destino>\.icdown\danados\<unix>_<nombre>` para que nunca se
/// vuelva a adoptar como bueno. Si ya no existe, no hace nada.
fn apartar_danado(destino: &Path, ruta_rel: &str) -> std::io::Result<()> {
    let origen = layout::en_disco(destino, ruta_rel);
    if !origen.exists() {
        return Ok(());
    }
    let dir = destino.join(".icdown").join("danados");
    std::fs::create_dir_all(&dir)?;
    let nombre = origen.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "archivo".into());
    let unix = chrono::Utc::now().timestamp();
    let mut destino_final = dir.join(format!("{unix}_{nombre}"));
    let mut n = 2;
    while destino_final.exists() {
        destino_final = dir.join(format!("{unix}_{n}_{nombre}"));
        n += 1;
    }
    std::fs::rename(&origen, &destino_final)
}
