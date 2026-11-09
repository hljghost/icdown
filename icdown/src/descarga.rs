//! Descarga a <archivo>.part reanudando lo que ya hubiera (Range).
use reqwest::blocking::Client;
use reqwest::StatusCode;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub enum ErrorDescarga {
    /// 401/403/404/410: hay que pedir enlaces nuevos a iCloud.
    UrlCaducada(u16),
    Transitorio(String),
    Disco(String),
    Cancelada,
}

impl std::fmt::Display for ErrorDescarga {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErrorDescarga::UrlCaducada(c) => write!(f, "enlace de descarga caducado (HTTP {c})"),
            ErrorDescarga::Transitorio(m) => write!(f, "fallo de red: {m}"),
            ErrorDescarga::Disco(m) => write!(f, "fallo de disco: {m}"),
            ErrorDescarga::Cancelada => write!(f, "cancelada"),
        }
    }
}

/// Bytes recibidos por este proceso desde que arrancó (la bandeja calcula la velocidad con él).
pub static BYTES_RECIBIDOS: AtomicU64 = AtomicU64::new(0);

fn disco(e: std::io::Error) -> ErrorDescarga {
    ErrorDescarga::Disco(e.to_string())
}

pub fn cliente() -> Client {
    Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .user_agent("icdown/0.1")
        .build()
        .expect("no pude crear el cliente HTTP")
}

pub fn ruta_part(final_: &Path) -> PathBuf {
    let mut s = final_.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

/// Cada cuánto se fuerza a disco lo descargado y se anota en el `.meta`.
pub const SINCRONIZAR_CADA: u64 = 16 * 1024 * 1024;

/// `<archivo>.part.meta`: 1.ª línea, identidad del contenido que se está bajando ("tamaño:checksum");
/// 2.ª línea, cuántos bytes del `.part` se sabe que están ya en disco (`sync_all`).
pub fn ruta_meta(part: &Path) -> PathBuf {
    let mut s = part.as_os_str().to_owned();
    s.push(".meta");
    PathBuf::from(s)
}

/// Borra el `.part` y su `.meta` (lo que no exista se ignora).
pub fn borrar_part(part: &Path) {
    let _ = std::fs::remove_file(part);
    let _ = std::fs::remove_file(ruta_meta(part));
}

/// (identidad, bytes sincronizados). Sin segunda línea válida se asume que no hay nada sincronizado.
fn leer_meta(part: &Path) -> Option<(String, u64)> {
    let texto = std::fs::read_to_string(ruta_meta(part)).ok()?;
    let mut lineas = texto.lines();
    let identidad = lineas.next()?.to_string();
    let sincronizado = lineas.next().and_then(|l| l.trim().parse().ok()).unwrap_or(0);
    Some((identidad, sincronizado))
}

fn escribir_meta(part: &Path, identidad: &str, sincronizado: u64) -> Result<(), ErrorDescarga> {
    let mut f = std::fs::File::create(ruta_meta(part)).map_err(disco)?;
    writeln!(f, "{identidad}\n{sincronizado}").map_err(disco)?;
    f.sync_all().map_err(disco)
}

/// Fuerza a disco el `.part` y anota cuánto hay; si falla, el `.meta` conserva un valor menor (seguro).
fn sincronizar(archivo: &std::fs::File, part: &Path, identidad: &str, escrito: u64) -> Result<(), ErrorDescarga> {
    archivo.sync_all().map_err(disco)?;
    escribir_meta(part, identidad, escrito)
}

fn inicio_content_range(v: &str) -> Option<u64> {
    v.strip_prefix("bytes ")?.split('-').next()?.parse().ok()
}

/// Deja en `part` exactamente `tamano` bytes descargados de `url`, reanudando si ya había algo
/// del mismo contenido (`identidad`, anotada en `<part>.meta`); si no, empieza de cero.
pub fn descargar(cliente: &Client, url: &str, part: &Path, tamano: u64, identidad: &str, cancelar: &AtomicBool) -> Result<(), ErrorDescarga> {
    let mut tengo = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);
    match leer_meta(part) {
        Some((id, sincronizado)) if id == identidad && tengo <= tamano => {
            // Tras un corte de luz, lo no sincronizado puede ser basura (p. ej. ceros): se descarta.
            if tengo > sincronizado {
                let f = OpenOptions::new().write(true).open(part).map_err(disco)?;
                f.set_len(sincronizado).map_err(disco)?;
                f.sync_all().map_err(disco)?;
                tengo = sincronizado;
            }
        }
        _ => {
            if part.exists() {
                std::fs::remove_file(part).map_err(disco)?;
            }
            tengo = 0;
        }
    }
    if tengo == 0 {
        escribir_meta(part, identidad, 0)?;
    }
    if tengo == tamano && tamano > 0 {
        return Ok(());
    }
    let segundos = 120 + tamano.saturating_sub(tengo) / (256 * 1024);
    let mut pet = cliente.get(url).timeout(Duration::from_secs(segundos));
    if tengo > 0 {
        pet = pet.header(reqwest::header::RANGE, format!("bytes={tengo}-"));
    }
    let mut resp = pet.send().map_err(|e| ErrorDescarga::Transitorio(format!("conexión: {e}")))?;
    let anexar = match resp.status() {
        StatusCode::PARTIAL_CONTENT => {
            let inicio = resp
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(inicio_content_range);
            if inicio != Some(tengo) {
                borrar_part(part);
                return Err(ErrorDescarga::Transitorio("el servidor devolvió un rango inesperado; se reinicia".into()));
            }
            true
        }
        StatusCode::OK => false,
        StatusCode::RANGE_NOT_SATISFIABLE => {
            borrar_part(part);
            return Err(ErrorDescarga::Transitorio("rango no satisfacible; se reinicia".into()));
        }
        s if s == StatusCode::UNAUTHORIZED || s == StatusCode::FORBIDDEN || s == StatusCode::NOT_FOUND || s == StatusCode::GONE => {
            return Err(ErrorDescarga::UrlCaducada(s.as_u16()));
        }
        s => return Err(ErrorDescarga::Transitorio(format!("HTTP {}", s.as_u16()))),
    };
    if !anexar {
        escribir_meta(part, identidad, 0)?;
    }
    let mut archivo = if anexar {
        OpenOptions::new().append(true).open(part)
    } else {
        OpenOptions::new().create(true).write(true).truncate(true).open(part)
    }
    .map_err(disco)?;
    let mut escrito = if anexar { tengo } else { 0 };
    let mut sin_sincronizar = 0u64;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if cancelar.load(Ordering::Relaxed) {
            let _ = sincronizar(&archivo, part, identidad, escrito);
            return Err(ErrorDescarga::Cancelada);
        }
        let n = match resp.read(&mut buf) {
            Ok(n) => n,
            Err(e) => {
                let _ = sincronizar(&archivo, part, identidad, escrito);
                return Err(ErrorDescarga::Transitorio(format!("lectura: {e}")));
            }
        };
        if n == 0 {
            break;
        }
        archivo.write_all(&buf[..n]).map_err(disco)?;
        escrito += n as u64;
        BYTES_RECIBIDOS.fetch_add(n as u64, Ordering::Relaxed);
        sin_sincronizar += n as u64;
        if sin_sincronizar >= SINCRONIZAR_CADA {
            sincronizar(&archivo, part, identidad, escrito)?;
            sin_sincronizar = 0;
        }
    }
    sincronizar(&archivo, part, identidad, escrito)?;
    let final_len = std::fs::metadata(part).map_err(disco)?.len();
    if final_len != tamano {
        return Err(ErrorDescarga::Transitorio(format!("descarga incompleta: {final_len} de {tamano} bytes")));
    }
    Ok(())
}
