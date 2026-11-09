#![allow(dead_code)]
//! Servidor HTTP mínimo para pruebas (Range, cortes, códigos forzados) y archivos válidos.
use icdown::modelo::{AssetIcloud, ParteIcloud, TipoParte};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub enum Comportamiento {
    Servir(Vec<u8>),
    CortarPrimeraVez { datos: Vec<u8>, tras: usize },
    Estado(u16),
    /// Responde `codigo` tras esperar `ms` (p. ej. un enlace que caduca mientras se pide).
    EstadoTras { codigo: u16, ms: u64 },
    /// La petición n.º i (desde 0) a la ruta recibe `datos[i]` (o el último si hay más peticiones).
    PorPeticion(Vec<Vec<u8>>),
}

#[derive(Debug, Clone)]
pub struct Peticion {
    pub ruta: String,
    pub rango: Option<String>,
}

pub struct Servidor {
    pub base: String,
    pub peticiones: Arc<Mutex<Vec<Peticion>>>,
}

impl Servidor {
    pub fn url(&self, ruta: &str) -> String {
        format!("{}{}", self.base, ruta)
    }
    pub fn peticiones_a(&self, ruta: &str) -> Vec<Peticion> {
        self.peticiones.lock().unwrap().iter().filter(|p| p.ruta == ruta).cloned().collect()
    }
    pub fn total_peticiones(&self) -> usize {
        self.peticiones.lock().unwrap().len()
    }
}

pub fn servidor(rutas: Vec<(&str, Comportamiento)>) -> Servidor {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mapa: HashMap<String, Comportamiento> = rutas.into_iter().map(|(r, c)| (r.to_string(), c)).collect();
    let peticiones = Arc::new(Mutex::new(Vec::new()));
    let registro = peticiones.clone();
    std::thread::spawn(move || {
        let mut cortadas: HashMap<String, bool> = HashMap::new();
        for s in listener.incoming().flatten() {
            atender(s, &mapa, &registro, &mut cortadas);
        }
    });
    Servidor { base, peticiones }
}

fn atender(mut stream: TcpStream, mapa: &HashMap<String, Comportamiento>, peticiones: &Mutex<Vec<Peticion>>, cortadas: &mut HashMap<String, bool>) {
    let mut lector = BufReader::new(stream.try_clone().unwrap());
    let mut primera = String::new();
    if lector.read_line(&mut primera).is_err() {
        return;
    }
    // La query (p. ej. `?e=<caducidad>`) no cuenta para elegir la ruta.
    let ruta = primera.split_whitespace().nth(1).unwrap_or("/").split('?').next().unwrap_or("/").to_string();
    let mut rango = None;
    loop {
        let mut l = String::new();
        if lector.read_line(&mut l).unwrap_or(0) == 0 {
            break;
        }
        let l = l.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            if k.eq_ignore_ascii_case("range") {
                rango = Some(v.trim().to_string());
            }
        }
    }
    let previas = {
        let mut p = peticiones.lock().unwrap();
        let n = p.iter().filter(|x| x.ruta == ruta).count();
        p.push(Peticion { ruta: ruta.clone(), rango: rango.clone() });
        n
    };
    let desde = rango
        .as_deref()
        .and_then(|r| r.strip_prefix("bytes="))
        .and_then(|r| r.split('-').next())
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(0);
    match mapa.get(&ruta) {
        None => {
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        }
        Some(Comportamiento::Estado(c)) => {
            let _ = write!(stream, "HTTP/1.1 {c} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        }
        Some(Comportamiento::EstadoTras { codigo, ms }) => {
            std::thread::sleep(std::time::Duration::from_millis(*ms));
            let _ = write!(stream, "HTTP/1.1 {codigo} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        }
        Some(Comportamiento::Servir(datos)) => enviar(&mut stream, datos, desde, None),
        Some(Comportamiento::PorPeticion(v)) => enviar(&mut stream, &v[previas.min(v.len() - 1)], desde, None),
        Some(Comportamiento::CortarPrimeraVez { datos, tras }) => {
            let primera_vez = cortadas.insert(ruta.clone(), true).is_none();
            enviar(&mut stream, datos, desde, if primera_vez { Some(*tras) } else { None });
        }
    }
    let _ = stream.flush();
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

fn enviar(stream: &mut TcpStream, datos: &[u8], desde: usize, cortar_tras: Option<usize>) {
    let cuerpo = &datos[desde.min(datos.len())..];
    let cab = if desde > 0 {
        format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
            cuerpo.len(), desde, datos.len() - 1, datos.len()
        )
    } else {
        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", cuerpo.len())
    };
    let _ = stream.write_all(cab.as_bytes());
    let n = cortar_tras.unwrap_or(cuerpo.len()).min(cuerpo.len());
    let _ = stream.write_all(&cuerpo[..n]);
}

pub fn patron(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i % 251) as u8).collect()
}

pub fn caja(tipo: &[u8; 4], contenido: &[u8]) -> Vec<u8> {
    let mut v = ((8 + contenido.len()) as u32).to_be_bytes().to_vec();
    v.extend_from_slice(tipo);
    v.extend_from_slice(contenido);
    v
}

pub fn heic(n: usize) -> Vec<u8> {
    let mut v = caja(b"ftyp", b"heic\0\0\0\0mif1heic");
    v.extend(caja(b"mdat", &patron(n)));
    v
}

pub fn mov(n: usize) -> Vec<u8> {
    let mut v = caja(b"ftyp", b"qt  \0\0\0\0qt  ");
    v.extend(caja(b"mdat", &patron(n)));
    v
}

pub fn jpeg(n: usize) -> Vec<u8> {
    // SOI + COM (segmento con longitud) + SOS vacío + datos "entropía" sin 0xFF + EOI.
    let mut v = vec![0xFF, 0xD8, 0xFF, 0xFE, 0x00, 0x04, b'o', b'k', 0xFF, 0xDA, 0x00, 0x02];
    v.extend(patron(n).into_iter().map(|b| if b == 0xFF { 0xFE } else { b }));
    v.extend([0xFF, 0xD9]);
    v
}

/// JPEG sin marcador de fin (como algunos originales guardados así en iCloud).
pub fn jpeg_atipico(n: usize) -> Vec<u8> {
    let mut v = jpeg(n);
    v.truncate(v.len() - 2);
    v
}

pub fn parte(tipo: TipoParte, datos: &[u8], url: String, ext: &str) -> ParteIcloud {
    ParteIcloud { tipo, tamano: datos.len() as u64, url, extension: ext.into(), checksum: "c".into() }
}

pub fn asset(id: &str, nombre: &str, fecha: &str, biblioteca: &str, partes: Vec<ParteIcloud>) -> AssetIcloud {
    AssetIcloud { id: id.into(), biblioteca: biblioteca.into(), nombre_base: nombre.into(), fecha: fecha.into(), fecha_agregada: None, liberable: true, partes }
}
