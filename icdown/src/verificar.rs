//! Verificación: tamaño exacto + estructura completa + SHA-256 releído del disco.
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs::File;
use std::io::{BufRead, Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub enum FalloVerificacion {
    NoExiste,
    Tamano { esperado: u64, real: u64 },
    Estructura(String),
    Lectura(String),
}

impl fmt::Display for FalloVerificacion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FalloVerificacion::NoExiste => write!(f, "el archivo no existe"),
            FalloVerificacion::Tamano { esperado, real } => write!(f, "tamaño {real} bytes, se esperaban {esperado}"),
            FalloVerificacion::Estructura(m) => write!(f, "archivo incompleto o dañado: {m}"),
            FalloVerificacion::Lectura(m) => write!(f, "error de lectura: {m}"),
        }
    }
}

fn lectura(e: std::io::Error) -> FalloVerificacion {
    FalloVerificacion::Lectura(e.to_string())
}

/// Devuelve el SHA-256 en hexadecimal si el archivo pasa todas las comprobaciones.
pub fn verificar_archivo(ruta: &Path, tamano_esperado: u64, extension: &str) -> Result<String, FalloVerificacion> {
    let meta = std::fs::metadata(ruta).map_err(|_| FalloVerificacion::NoExiste)?;
    if meta.len() != tamano_esperado {
        return Err(FalloVerificacion::Tamano { esperado: tamano_esperado, real: meta.len() });
    }
    let mut f = File::open(ruta).map_err(lectura)?;
    comprobar_estructura(&mut f, meta.len(), extension)?;
    f.seek(SeekFrom::Start(0)).map_err(lectura)?;
    sha256(&mut f)
}

/// Tamaño exacto + SHA-256, sin mirar la estructura. Solo para partes con estructura atípica ya
/// confirmadas por doble descarga (el original en iCloud es así): su SHA-256 anotado es la referencia.
pub fn verificar_archivo_sin_estructura(ruta: &Path, tamano_esperado: u64) -> Result<String, FalloVerificacion> {
    let meta = std::fs::metadata(ruta).map_err(|_| FalloVerificacion::NoExiste)?;
    if meta.len() != tamano_esperado {
        return Err(FalloVerificacion::Tamano { esperado: tamano_esperado, real: meta.len() });
    }
    sha256(&mut File::open(ruta).map_err(lectura)?)
}

/// Re-verificación de una parte ya verificada: las de estructura atípica (confirmadas por doble
/// descarga) solo por tamaño y SHA-256; el resto, con la comprobación estructural completa.
pub fn reverificar(ruta: &Path, tamano_esperado: u64, extension: &str, estructura_atipica: bool) -> Result<String, FalloVerificacion> {
    if estructura_atipica {
        verificar_archivo_sin_estructura(ruta, tamano_esperado)
    } else {
        verificar_archivo(ruta, tamano_esperado, extension)
    }
}

fn sha256(f: &mut File) -> Result<String, FalloVerificacion> {
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).map_err(lectura)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

pub fn comprobar_estructura<R: Read + Seek>(f: &mut R, len: u64, extension: &str) -> Result<(), FalloVerificacion> {
    match extension.to_ascii_uppercase().as_str() {
        "JPG" | "JPEG" => jpeg(f, len),
        "PNG" => png(f, len),
        "HEIC" | "HEIF" | "MOV" | "MP4" | "M4V" | "AVIF" | "3GP" => isobmff(f, len),
        _ => Ok(()),
    }
}

fn leer_en<R: Read + Seek>(f: &mut R, pos: u64, n: usize) -> Result<Vec<u8>, FalloVerificacion> {
    f.seek(SeekFrom::Start(pos)).map_err(lectura)?;
    let mut v = vec![0u8; n];
    f.read_exact(&mut v).map_err(lectura)?;
    Ok(v)
}

fn byte<B: std::io::BufRead>(r: &mut B) -> Result<Option<u8>, FalloVerificacion> {
    let b = r.fill_buf().map_err(lectura)?.first().copied();
    if b.is_some() {
        r.consume(1);
    }
    Ok(b)
}

/// Recorre la estructura JPEG por segmentos hasta el EOI (FF D9). Los bytes posteriores al EOI se permiten
/// (fotos con movimiento de Android/Samsung añaden un MP4 detrás). Lee en streaming con búfer.
fn jpeg<R: Read + Seek>(f: &mut R, _len: u64) -> Result<(), FalloVerificacion> {
    const CORTADO: &str = "JPEG cortado antes del marcador de fin";
    f.seek(SeekFrom::Start(0)).map_err(lectura)?;
    let mut r = std::io::BufReader::with_capacity(1 << 16, f);
    let cortado = || FalloVerificacion::Estructura(CORTADO.into());
    if byte(&mut r)? != Some(0xFF) || byte(&mut r)? != Some(0xD8) {
        return Err(FalloVerificacion::Estructura("no empieza como JPEG".into()));
    }
    // Marcador pendiente ya leído (tras un escaneo de datos entropía) o None.
    let mut pendiente: Option<u8> = None;
    loop {
        let m = match pendiente.take() {
            Some(m) => m,
            None => {
                if byte(&mut r)?.ok_or_else(cortado)? != 0xFF {
                    return Err(FalloVerificacion::Estructura("JPEG: se esperaba un marcador".into()));
                }
                let mut m = byte(&mut r)?.ok_or_else(cortado)?;
                while m == 0xFF {
                    m = byte(&mut r)?.ok_or_else(cortado)?;
                }
                m
            }
        };
        match m {
            0xD9 => return Ok(()),
            0x00 => return Err(FalloVerificacion::Estructura("JPEG: marcador inválido".into())),
            0x01 | 0xD0..=0xD8 => {}
            _ => {
                let hi = byte(&mut r)?.ok_or_else(cortado)?;
                let lo = byte(&mut r)?.ok_or_else(cortado)?;
                let l = u16::from_be_bytes([hi, lo]) as usize;
                if l < 2 {
                    return Err(FalloVerificacion::Estructura("JPEG: longitud de segmento inválida".into()));
                }
                // Salta el contenido del segmento (sin leerlo a memoria).
                let mut resto = l - 2;
                while resto > 0 {
                    let n = {
                        let b = r.fill_buf().map_err(lectura)?;
                        if b.is_empty() {
                            return Err(cortado());
                        }
                        b.len().min(resto)
                    };
                    r.consume(n);
                    resto -= n;
                }
                if m == 0xDA {
                    // Datos entropía: FF 00 y RSTn siguen; otro marcador reanuda el recorrido.
                    loop {
                        let b = {
                            let buf = r.fill_buf().map_err(lectura)?;
                            if buf.is_empty() {
                                return Err(cortado());
                            }
                            match buf.iter().position(|&x| x == 0xFF) {
                                Some(i) => {
                                    r.consume(i + 1);
                                    None
                                }
                                None => Some(buf.len()),
                            }
                        };
                        if let Some(n) = b {
                            r.consume(n);
                            continue;
                        }
                        let mut s = byte(&mut r)?.ok_or_else(cortado)?;
                        while s == 0xFF {
                            s = byte(&mut r)?.ok_or_else(cortado)?;
                        }
                        if s == 0x00 || (0xD0..=0xD7).contains(&s) {
                            continue;
                        }
                        pendiente = Some(s);
                        break;
                    }
                }
            }
        }
    }
}

fn png<R: Read + Seek>(f: &mut R, len: u64) -> Result<(), FalloVerificacion> {
    const FIRMA: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    const IEND: [u8; 12] = [0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82];
    if len < 20 || leer_en(f, 0, 8)? != FIRMA {
        return Err(FalloVerificacion::Estructura("no empieza como PNG".into()));
    }
    if leer_en(f, len - 12, 12)? != IEND {
        return Err(FalloVerificacion::Estructura("PNG sin bloque IEND final".into()));
    }
    Ok(())
}

fn isobmff<R: Read + Seek>(f: &mut R, len: u64) -> Result<(), FalloVerificacion> {
    let mut pos = 0u64;
    let mut primera = true;
    while pos < len {
        if len - pos < 8 {
            return Err(FalloVerificacion::Estructura(format!("caja truncada en el byte {pos}")));
        }
        let h = leer_en(f, pos, 8)?;
        let tam32 = u32::from_be_bytes([h[0], h[1], h[2], h[3]]);
        let tipo = &h[4..8];
        if !tipo.iter().all(|b| b.is_ascii_graphic() || *b == b' ') {
            return Err(FalloVerificacion::Estructura(format!("tipo de caja inválido en el byte {pos}")));
        }
        if primera && !matches!(tipo, b"ftyp" | b"wide" | b"free" | b"skip" | b"mdat" | b"moov" | b"pnot") {
            return Err(FalloVerificacion::Estructura("no empieza como HEIC/MOV/MP4".into()));
        }
        primera = false;
        let tam = match tam32 {
            0 => len - pos,
            1 => {
                if len - pos < 16 {
                    return Err(FalloVerificacion::Estructura(format!("caja grande truncada en el byte {pos}")));
                }
                let g = leer_en(f, pos + 8, 8)?;
                u64::from_be_bytes([g[0], g[1], g[2], g[3], g[4], g[5], g[6], g[7]])
            }
            n => n as u64,
        };
        if tam < 8 {
            return Err(FalloVerificacion::Estructura(format!("tamaño de caja inválido en el byte {pos}")));
        }
        pos = pos.checked_add(tam).ok_or_else(|| FalloVerificacion::Estructura("tamaño de caja desbordado".into()))?;
    }
    if pos != len {
        return Err(FalloVerificacion::Estructura(format!("el archivo está cortado: la última caja termina en {pos} y el archivo mide {len}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn caja(tipo: &[u8; 4], contenido: &[u8]) -> Vec<u8> {
        let mut v = ((8 + contenido.len()) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(tipo);
        v.extend_from_slice(contenido);
        v
    }

    fn heic() -> Vec<u8> {
        let mut v = caja(b"ftyp", b"heic\0\0\0\0mif1heic");
        v.extend(caja(b"mdat", &[7u8; 100]));
        v
    }

    fn est(datos: &[u8], ext: &str) -> Result<(), FalloVerificacion> {
        comprobar_estructura(&mut Cursor::new(datos), datos.len() as u64, ext)
    }

    #[test]
    fn isobmff_completo_y_cortado() {
        let v = heic();
        assert_eq!(est(&v, "HEIC"), Ok(()));
        assert!(matches!(est(&v[..v.len() - 10], "HEIC"), Err(FalloVerificacion::Estructura(_))));
        assert!(matches!(est(&v[..5], "MOV"), Err(FalloVerificacion::Estructura(_))));
    }

    #[test]
    fn isobmff_caja_de_tamano_cero_llega_al_final() {
        let mut v = caja(b"ftyp", b"qt  \0\0\0\0");
        v.extend_from_slice(&0u32.to_be_bytes());
        v.extend_from_slice(b"mdat");
        v.extend_from_slice(&[1u8; 50]);
        assert_eq!(est(&v, "MOV"), Ok(()));
    }

    #[test]
    fn jpeg_y_png() {
        let jpg = jpeg_base();
        assert_eq!(est(&jpg, "jpg"), Ok(()));
        assert!(est(&jpg[..jpg.len() - 3], "JPEG").is_err());
        assert!(est(b"no es un jpeg", "JPG").is_err());
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3];
        png.extend_from_slice(&[0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82]);
        assert_eq!(est(&png, "PNG"), Ok(()));
        assert!(est(&png[..png.len() - 1], "PNG").is_err());
    }

    fn seg(m: u8, carga: &[u8]) -> Vec<u8> {
        let mut v = vec![0xFF, m];
        v.extend_from_slice(&((carga.len() + 2) as u16).to_be_bytes());
        v.extend_from_slice(carga);
        v
    }

    /// JPEG sintético: SOI, APP0, DQT, SOF, DHT, SOS + entropía (con FF 00 y RST) + EOI.
    fn jpeg_base() -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8];
        v.extend(seg(0xE0, b"JFIF "));
        v.extend(seg(0xDB, &[0u8; 65]));
        v.extend(seg(0xC0, &[8, 0, 8, 0, 8, 1, 1, 0x11, 0]));
        v.extend(seg(0xC4, &[0u8; 20]));
        v.extend(seg(0xDA, &[1, 1, 0, 0, 63, 0]));
        v.extend_from_slice(&[1, 2, 0xFF, 0x00, 3, 0xFF, 0xD0, 4, 0xFF, 0xFF, 0x00, 5]);
        v.extend_from_slice(&[0xFF, 0xD9]);
        v
    }

    fn jpeg_progresivo() -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8];
        v.extend(seg(0xC2, &[8, 0, 8, 0, 8, 1, 1, 0x11, 0]));
        for _ in 0..3 {
            v.extend(seg(0xC4, &[0u8; 20]));
            v.extend(seg(0xDA, &[1, 1, 0, 0, 0, 0]));
            v.extend_from_slice(&[9, 8, 0xFF, 0x00, 7]);
        }
        v.extend_from_slice(&[0xFF, 0xD9]);
        v
    }

    fn es_estructura(r: Result<(), FalloVerificacion>) -> bool {
        matches!(r, Err(FalloVerificacion::Estructura(_)))
    }

    #[test]
    fn jpeg_sintetico_base_y_progresivo() {
        assert_eq!(est(&jpeg_base(), "JPG"), Ok(()));
        assert_eq!(est(&jpeg_progresivo(), "JPG"), Ok(()));
    }

    #[test]
    fn jpeg_con_datos_tras_el_eoi_motion_photo() {
        let mut v = jpeg_base();
        v.extend(caja(b"ftyp", b"mp42    mp42"));
        v.extend(vec![0xAB; 2 << 20]);
        assert_eq!(est(&v, "jpg"), Ok(()));
    }

    #[test]
    fn jpeg_cortado_en_entropia_o_en_segmento_falla() {
        let v = jpeg_base();
        assert!(es_estructura(est(&v[..v.len() - 2], "JPG")), "sin EOI");
        assert!(es_estructura(est(&v[..v.len() - 1], "JPG")), "FF suelto");
        assert!(es_estructura(est(&v[..30], "JPG")), "a mitad de segmento");
        assert!(es_estructura(est(&[0xFF, 0xD8, 0xFF], "JPG")));
        let p = jpeg_progresivo();
        assert!(es_estructura(est(&p[..p.len() - 2], "JPG")));
    }

    #[test]
    fn jpeg_miniatura_exif_con_eoi_no_cuenta() {
        let mut miniatura = jpeg_base();
        miniatura.truncate(miniatura.len());
        let mut v = vec![0xFF, 0xD8];
        v.extend(seg(0xE1, &[b"Exif  ".as_slice(), &miniatura].concat()));
        v.extend(seg(0xC0, &[8, 0, 8, 0, 8, 1, 1, 0x11, 0]));
        v.extend(seg(0xDA, &[1, 1, 0, 0, 63, 0]));
        v.extend_from_slice(&[1, 2, 3]);
        assert!(es_estructura(est(&v, "JPG")), "imagen principal cortada");
        v.extend_from_slice(&[0xFF, 0xD9]);
        assert_eq!(est(&v, "JPG"), Ok(()));
    }

    #[test]
    fn jpeg_reales_de_pillow_base_y_progresivo() {
        for (nombre, datos) in [
            ("base", &include_bytes!("../tests/datos/base.jpg")[..]),
            ("prog", &include_bytes!("../tests/datos/prog.jpg")[..]),
        ] {
            assert_eq!(est(datos, "JPG"), Ok(()), "{nombre}");
            let mut con_cola = datos.to_vec();
            con_cola.extend(vec![0x55; 3 << 20]);
            assert_eq!(est(&con_cola, "JPG"), Ok(()), "{nombre} + cola");
            assert!(es_estructura(est(&datos[..datos.len() - 2], "JPG")), "{nombre} sin EOI");
            assert!(es_estructura(est(&datos[..datos.len() / 2], "JPG")), "{nombre} a la mitad");
        }
    }

    #[test]
    fn otras_extensiones_solo_tamano() {
        assert_eq!(est(b"lo que sea", "DNG"), Ok(()));
    }

    #[test]
    fn verificar_archivo_devuelve_sha256() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path().join("a.txt");
        std::fs::write(&r, b"abc").unwrap();
        assert_eq!(
            verificar_archivo(&r, 3, "TXT").unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(verificar_archivo(&r, 4, "TXT"), Err(FalloVerificacion::Tamano { esperado: 4, real: 3 }));
        assert_eq!(verificar_archivo(&d.path().join("no"), 3, "TXT"), Err(FalloVerificacion::NoExiste));
    }
}
