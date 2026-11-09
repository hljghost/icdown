mod comun;
use comun::*;
use icdown::descarga::{self, ErrorDescarga};
use std::sync::atomic::AtomicBool;

const ID: &str = "id-1";

fn bajar(url: &str, part: &std::path::Path, tamano: u64) -> Result<(), ErrorDescarga> {
    bajar_como(url, part, tamano, ID)
}

fn bajar_como(url: &str, part: &std::path::Path, tamano: u64, identidad: &str) -> Result<(), ErrorDescarga> {
    descarga::descargar(&descarga::cliente(), url, part, tamano, identidad, &AtomicBool::new(false))
}

#[test]
fn descarga_completa() {
    let datos = patron(300_000);
    let s = servidor(vec![("/a", Comportamiento::Servir(datos.clone()))]);
    let d = tempfile::tempdir().unwrap();
    let part = d.path().join("a.part");
    bajar(&s.url("/a"), &part, datos.len() as u64).unwrap();
    assert_eq!(std::fs::read(&part).unwrap(), datos);
}

#[test]
fn reanuda_tras_corte() {
    let datos = patron(3_000_000);
    let s = servidor(vec![("/a", Comportamiento::CortarPrimeraVez { datos: datos.clone(), tras: 1_000_000 })]);
    let d = tempfile::tempdir().unwrap();
    let part = d.path().join("a.part");
    assert!(matches!(bajar(&s.url("/a"), &part, datos.len() as u64), Err(ErrorDescarga::Transitorio(_))));
    let parcial = std::fs::metadata(&part).unwrap().len();
    assert!(parcial > 0 && parcial < datos.len() as u64, "parcial = {parcial}");
    bajar(&s.url("/a"), &part, datos.len() as u64).unwrap();
    assert_eq!(std::fs::read(&part).unwrap(), datos);
    assert_eq!(s.peticiones_a("/a")[1].rango.as_deref(), Some(format!("bytes={parcial}-").as_str()));
}

#[test]
fn url_caducada() {
    let s = servidor(vec![("/a", Comportamiento::Estado(403))]);
    let d = tempfile::tempdir().unwrap();
    assert_eq!(bajar(&s.url("/a"), &d.path().join("a.part"), 10), Err(ErrorDescarga::UrlCaducada(403)));
}

#[test]
fn error_de_servidor_es_transitorio() {
    let s = servidor(vec![("/a", Comportamiento::Estado(503))]);
    let d = tempfile::tempdir().unwrap();
    assert!(matches!(bajar(&s.url("/a"), &d.path().join("a.part"), 10), Err(ErrorDescarga::Transitorio(_))));
}

#[test]
fn part_mas_grande_que_lo_esperado_se_descarta() {
    let datos = patron(1000);
    let s = servidor(vec![("/a", Comportamiento::Servir(datos.clone()))]);
    let d = tempfile::tempdir().unwrap();
    let part = d.path().join("a.part");
    std::fs::write(&part, patron(1500)).unwrap();
    bajar(&s.url("/a"), &part, 1000).unwrap();
    assert_eq!(std::fs::read(&part).unwrap(), datos);
    assert_eq!(s.peticiones_a("/a")[0].rango, None);
}

#[test]
fn ya_completa_no_pide_nada() {
    let s = servidor(vec![]);
    let d = tempfile::tempdir().unwrap();
    let part = d.path().join("a.part");
    std::fs::write(&part, patron(10)).unwrap();
    std::fs::write(descarga::ruta_meta(&part), format!("{ID}\n10\n")).unwrap();
    bajar(&s.url("/a"), &part, 10).unwrap();
    assert_eq!(s.total_peticiones(), 0);
}

#[test]
fn cancelada() {
    let datos = patron(1000);
    let s = servidor(vec![("/a", Comportamiento::Servir(datos))]);
    let d = tempfile::tempdir().unwrap();
    let r = descarga::descargar(&descarga::cliente(), &s.url("/a"), &d.path().join("a.part"), 1000, ID, &AtomicBool::new(true));
    assert_eq!(r, Err(ErrorDescarga::Cancelada));
}

#[test]
fn ruta_part_anade_sufijo() {
    let p = descarga::ruta_part(std::path::Path::new(r"C:\x\IMG_1.HEIC"));
    assert_eq!(p, std::path::PathBuf::from(r"C:\x\IMG_1.HEIC.part"));
}

#[test]
fn part_de_otro_contenido_se_descarta() {
    let datos = patron(3_000_000);
    let s = servidor(vec![("/a", Comportamiento::CortarPrimeraVez { datos: datos.clone(), tras: 1_000_000 })]);
    let d = tempfile::tempdir().unwrap();
    let part = d.path().join("a.part");
    assert!(bajar_como(&s.url("/a"), &part, datos.len() as u64, "viejo").is_err());
    assert!(std::fs::metadata(&part).unwrap().len() > 0);
    bajar_como(&s.url("/a"), &part, datos.len() as u64, "nuevo").unwrap();
    assert_eq!(std::fs::read(&part).unwrap(), datos);
    assert_eq!(s.peticiones_a("/a")[1].rango, None, "no se reanuda un .part de otro contenido");
    assert_eq!(std::fs::read_to_string(descarga::ruta_meta(&part)).unwrap().lines().next(), Some("nuevo"));
}

#[test]
fn part_sin_meta_se_descarta() {
    let datos = patron(1000);
    let s = servidor(vec![("/a", Comportamiento::Servir(datos.clone()))]);
    let d = tempfile::tempdir().unwrap();
    let part = d.path().join("a.part");
    std::fs::write(&part, patron(500)).unwrap();
    bajar(&s.url("/a"), &part, 1000).unwrap();
    assert_eq!(std::fs::read(&part).unwrap(), datos);
    assert_eq!(s.peticiones_a("/a")[0].rango, None);
}

#[test]
fn tras_un_apagon_solo_se_conserva_lo_sincronizado() {
    let datos = patron(1000);
    let s = servidor(vec![("/a", Comportamiento::Servir(datos.clone()))]);
    let d = tempfile::tempdir().unwrap();
    let part = d.path().join("a.part");
    let mut con_ceros = datos[..600].to_vec();
    con_ceros.extend(vec![0u8; 400]); // lo no sincronizado quedó en ceros tras el corte de luz
    std::fs::write(&part, &con_ceros).unwrap();
    std::fs::write(descarga::ruta_meta(&part), format!("{ID}\n600\n")).unwrap();
    bajar(&s.url("/a"), &part, 1000).unwrap();
    assert_eq!(s.peticiones_a("/a")[0].rango.as_deref(), Some("bytes=600-"));
    assert_eq!(std::fs::read(&part).unwrap(), datos);
}

#[test]
fn meta_sin_longitud_sincronizada_empieza_de_cero() {
    let datos = patron(1000);
    let s = servidor(vec![("/a", Comportamiento::Servir(datos.clone()))]);
    let d = tempfile::tempdir().unwrap();
    let part = d.path().join("a.part");
    std::fs::write(&part, vec![0u8; 700]).unwrap();
    std::fs::write(descarga::ruta_meta(&part), format!("{ID}\n")).unwrap();
    bajar(&s.url("/a"), &part, 1000).unwrap();
    assert_eq!(s.peticiones_a("/a")[0].rango, None);
    assert_eq!(std::fs::read(&part).unwrap(), datos);
}
