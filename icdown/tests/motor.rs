mod comun;
use comun::*;
use icdown::descarga;
use icdown::fuente::FuenteFalsa;
use icdown::modelo::TipoParte;
use icdown::motor::{self, Avance, FinRonda, OpcionesRonda, ResultadoRonda};
use icdown::registro::Registro;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

const AGO: &str = "2026-08-10T19:37:18-04:00";
const SEP: &str = "2026-09-02T10:00:00-04:00";

fn ronda_con(destino: &Path, reg: &Mutex<Registro>, f: &mut FuenteFalsa, conc: usize, mes: Option<&str>) -> ResultadoRonda {
    let op = OpcionesRonda { concurrencia: conc, mes: mes.map(String::from) };
    motor::ejecutar_ronda(destino, reg, f, &descarga::cliente(), &op, &AtomicBool::new(false), &|_: Avance| {}).unwrap()
}

fn ronda(destino: &Path, reg: &Mutex<Registro>, f: &mut FuenteFalsa) -> ResultadoRonda {
    ronda_con(destino, reg, f, 2, None)
}

fn nuevo_registro() -> Mutex<Registro> {
    Mutex::new(Registro::en_memoria().unwrap())
}

#[test]
fn copia_live_photo_y_jpg_en_carpetas_por_mes() {
    let (h, m, j) = (heic(5000), mov(3000), jpeg(2000));
    let s = servidor(vec![("/h", Comportamiento::Servir(h.clone())), ("/m", Comportamiento::Servir(m.clone())), ("/j", Comportamiento::Servir(j.clone()))]);
    let mut f = FuenteFalsa {
        assets: vec![
            asset("A1", "IMG_0001", AGO, "PrimarySync", vec![parte(TipoParte::Original, &h, s.url("/h"), "HEIC"), parte(TipoParte::Live, &m, s.url("/m"), "MOV")]),
            asset("A2", "IMG_0002", SEP, "PrimarySync", vec![parte(TipoParte::Original, &j, s.url("/j"), "JPG")]),
        ],
        ..Default::default()
    };
    let d = tempfile::tempdir().unwrap();
    let reg = nuevo_registro();
    let r = ronda(d.path(), &reg, &mut f);
    assert_eq!((r.fin, r.verificadas, r.fallidas), (FinRonda::Completa, 3, 0));
    let ago = d.path().join("2026").join("2026-08");
    assert_eq!(std::fs::read(ago.join("IMG_0001.HEIC")).unwrap(), h);
    assert_eq!(std::fs::read(ago.join("IMG_0001.MOV")).unwrap(), m);
    assert_eq!(std::fs::read(d.path().join("2026").join("2026-09").join("IMG_0002.JPG")).unwrap(), j);
    assert!(!ago.join("IMG_0001.HEIC.part").exists());
    let r2 = ronda(d.path(), &reg, &mut f);
    assert_eq!(r2.verificadas, 0);
    assert_eq!(s.total_peticiones(), 3);
}

#[test]
fn reanuda_tras_corte() {
    let h = heic(3_000_000);
    let s = servidor(vec![("/h", Comportamiento::CortarPrimeraVez { datos: h.clone(), tras: 1_000_000 })]);
    let mut f = FuenteFalsa { assets: vec![asset("A1", "IMG_0001", AGO, "PrimarySync", vec![parte(TipoParte::Original, &h, s.url("/h"), "HEIC")])], ..Default::default() };
    let d = tempfile::tempdir().unwrap();
    let reg = nuevo_registro();
    assert_eq!(ronda(d.path(), &reg, &mut f).fallidas, 1);
    reg.lock().unwrap().reintentar_ya().unwrap();
    assert_eq!(ronda(d.path(), &reg, &mut f).verificadas, 1);
    assert_eq!(std::fs::read(icdown::layout::en_disco(d.path(), r"2026\2026-08\IMG_0001.HEIC")).unwrap(), h);
    assert!(s.peticiones_a("/h")[1].rango.is_some());
}

#[test]
fn url_caducada_pide_relistar() {
    let h = heic(100);
    let (a, b) = (jpeg(100), jpeg(200));
    let s = servidor(vec![("/h", Comportamiento::Estado(403)), ("/a", Comportamiento::Servir(a.clone())), ("/b", Comportamiento::Servir(b.clone()))]);
    let mut f = FuenteFalsa {
        assets: vec![
            asset("A1", "IMG_0001", AGO, "PrimarySync", vec![parte(TipoParte::Original, &h, s.url("/h"), "HEIC")]),
            asset("A2", "IMG_0002", AGO, "PrimarySync", vec![parte(TipoParte::Original, &a, s.url("/a"), "JPG")]),
            asset("A3", "IMG_0003", SEP, "PrimarySync", vec![parte(TipoParte::Original, &b, s.url("/b"), "JPG")]),
        ],
        ..Default::default()
    };
    let d = tempfile::tempdir().unwrap();
    let r = ronda_con(d.path(), &nuevo_registro(), &mut f, 1, None);
    assert_eq!((r.fin, r.fallidas, r.verificadas), (FinRonda::Relistar, 1, 2), "el resto de la ronda sigue copiándose");
}

fn sin_temporales(carpeta: &Path) {
    let restos: Vec<String> = std::fs::read_dir(carpeta)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.contains(".part"))
        .collect();
    assert!(restos.is_empty(), "quedan temporales: {restos:?}");
}

#[test]
fn estructura_atipica_constante_se_acepta_por_doble_descarga() {
    let j = jpeg_atipico(1000);
    let s = servidor(vec![("/j", Comportamiento::Servir(j.clone()))]);
    let mut f = FuenteFalsa { assets: vec![asset("A1", "IMG_0001", AGO, "PrimarySync", vec![parte(TipoParte::Original, &j, s.url("/j"), "JPG")])], ..Default::default() };
    let d = tempfile::tempdir().unwrap();
    let reg = nuevo_registro();
    let r = ronda(d.path(), &reg, &mut f);
    assert_eq!((r.verificadas, r.fallidas), (1, 0));
    let carpeta = icdown::layout::en_disco(d.path(), r"2026\2026-08");
    assert_eq!(std::fs::read(carpeta.join("IMG_0001.JPG")).unwrap(), j);
    sin_temporales(&carpeta);
    let p = s.peticiones_a("/j");
    assert_eq!(p.len(), 2, "exactamente dos descargas");
    assert!(p.iter().all(|x| x.rango.is_none()), "las dos desde el byte 0: {p:?}");
    let g = reg.lock().unwrap().partes_verificadas().unwrap();
    assert!(g[0].estructura_atipica);
    assert_eq!(reg.lock().unwrap().resumen().unwrap().estructura_atipica, 1);
    assert!(errores(&reg).is_empty());
    assert!(leer_bitacora(d.path()).contains("ATIPICO IMG_0001 (original): estructura no estándar en iCloud; confirmado por doble descarga"));
    // Ya verificado: otra ronda no descarga nada.
    assert_eq!(ronda(d.path(), &reg, &mut f).verificadas, 0);
    assert_eq!(s.total_peticiones(), 2);
}

#[test]
fn estructura_atipica_en_png_e_isobmff_tambien() {
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    png.extend(patron(300));
    let mut mov_cortado = mov(3000);
    mov_cortado.truncate(2000);
    let s = servidor(vec![("/p", Comportamiento::Servir(png.clone())), ("/m", Comportamiento::Servir(mov_cortado.clone()))]);
    let mut f = FuenteFalsa {
        assets: vec![
            asset("A1", "IMG_0001", AGO, "PrimarySync", vec![parte(TipoParte::Original, &png, s.url("/p"), "PNG")]),
            asset("A2", "IMG_0002", AGO, "PrimarySync", vec![parte(TipoParte::Original, &mov_cortado, s.url("/m"), "MOV")]),
        ],
        ..Default::default()
    };
    let d = tempfile::tempdir().unwrap();
    let reg = nuevo_registro();
    assert_eq!(ronda(d.path(), &reg, &mut f).verificadas, 2);
    assert_eq!((s.peticiones_a("/p").len(), s.peticiones_a("/m").len()), (2, 2));
    assert_eq!(reg.lock().unwrap().resumen().unwrap().estructura_atipica, 2);
}

#[test]
fn dos_descargas_distintas_se_rechazan() {
    let malo = jpeg_atipico(1000);
    let mut otro = malo.clone();
    otro[500] ^= 0x01;
    let s = servidor(vec![("/j", Comportamiento::PorPeticion(vec![malo.clone(), otro]))]);
    let mut f = FuenteFalsa { assets: vec![asset("A1", "IMG_0001", AGO, "PrimarySync", vec![parte(TipoParte::Original, &malo, s.url("/j"), "JPG")])], ..Default::default() };
    let d = tempfile::tempdir().unwrap();
    let reg = nuevo_registro();
    let r = ronda(d.path(), &reg, &mut f);
    assert_eq!((r.verificadas, r.fallidas), (0, 1));
    assert_eq!(s.peticiones_a("/j").len(), 2);
    let carpeta = icdown::layout::en_disco(d.path(), r"2026\2026-08");
    assert!(!carpeta.join("IMG_0001.JPG").exists());
    sin_temporales(&carpeta);
    let e = errores(&reg);
    assert_eq!(e.len(), 1);
    assert!(e[0].contains("las dos descargas no coinciden"), "{e:?}");
    assert!(leer_bitacora(d.path()).contains("ATIPICO IMG_0001 (original): estructura no estándar en iCloud; las dos descargas no coinciden"));
    assert!(reg.lock().unwrap().partes_pendientes(motor::ahora(), None).unwrap().is_empty(), "espera con el reintento normal");
}

#[test]
fn atipico_ya_confirmado_se_adopta_tras_un_corte() {
    let j = jpeg_atipico(500);
    let s = servidor(vec![("/j", Comportamiento::Servir(j.clone()))]);
    let mut f = FuenteFalsa { assets: vec![asset("A1", "IMG_0001", AGO, "PrimarySync", vec![parte(TipoParte::Original, &j, s.url("/j"), "JPG")])], ..Default::default() };
    let d = tempfile::tempdir().unwrap();
    let reg = nuevo_registro();
    ronda(d.path(), &reg, &mut f);
    assert_eq!(s.total_peticiones(), 2);
    // Simula un corte entre renombrar el .part confirmado y anotarlo como verificado.
    let sha = reg.lock().unwrap().partes_verificadas().unwrap()[0].sha256.clone();
    reg.lock().unwrap().marcar_para_redescarga("A1", TipoParte::Original, "prueba").unwrap();
    reg.lock().unwrap().anotar_sha_pendiente("A1", TipoParte::Original, &sha, true).unwrap();
    assert_eq!(ronda(d.path(), &reg, &mut f).verificadas, 1);
    assert_eq!(s.total_peticiones(), 2, "se adopta sin descargar");
    assert!(reg.lock().unwrap().partes_verificadas().unwrap()[0].estructura_atipica);
}

#[test]
fn colision_de_nombres_no_pisa() {
    let (a, b) = (jpeg(100), jpeg(200));
    let s = servidor(vec![("/a", Comportamiento::Servir(a.clone())), ("/b", Comportamiento::Servir(b.clone()))]);
    let mut f = FuenteFalsa {
        assets: vec![
            asset("A1xxxx", "IMG_0001", AGO, "PrimarySync", vec![parte(TipoParte::Original, &a, s.url("/a"), "JPG")]),
            asset("Bq9zzz", "IMG_0001", AGO, "PrimarySync", vec![parte(TipoParte::Original, &b, s.url("/b"), "JPG")]),
        ],
        ..Default::default()
    };
    let d = tempfile::tempdir().unwrap();
    assert_eq!(ronda_con(d.path(), &nuevo_registro(), &mut f, 1, None).verificadas, 2);
    let carpeta = icdown::layout::en_disco(d.path(), r"2026\2026-08");
    assert_eq!(std::fs::read(carpeta.join("IMG_0001.JPG")).unwrap(), a);
    assert_eq!(std::fs::read(carpeta.join("IMG_0001 (bq9zzz).JPG")).unwrap(), b);
}

#[test]
fn sesion_caducada() {
    let mut f = FuenteFalsa { sesion_caducada: true, ..Default::default() };
    let d = tempfile::tempdir().unwrap();
    assert_eq!(ronda(d.path(), &nuevo_registro(), &mut f).fin, FinRonda::SesionCaducada);
}

#[test]
fn destino_ausente() {
    let mut f = FuenteFalsa::default();
    let d = tempfile::tempdir().unwrap();
    assert_eq!(ronda(&d.path().join("no-existe"), &nuevo_registro(), &mut f).fin, FinRonda::DestinoAusente);
}

#[test]
fn archivo_final_existente_se_adopta_sin_descargar() {
    let j = jpeg(500);
    let s = servidor(vec![("/j", Comportamiento::Servir(j.clone()))]);
    let mut f = FuenteFalsa { assets: vec![asset("A1", "IMG_0001", AGO, "PrimarySync", vec![parte(TipoParte::Original, &j, s.url("/j"), "JPG")])], ..Default::default() };
    let d = tempfile::tempdir().unwrap();
    let reg = nuevo_registro();
    ronda(d.path(), &reg, &mut f);
    // Simula un corte entre renombrar el .part verificado y anotarlo como verificado.
    let sha = reg.lock().unwrap().partes_verificadas().unwrap()[0].sha256.clone();
    reg.lock().unwrap().marcar_para_redescarga("A1", TipoParte::Original, "prueba").unwrap();
    reg.lock().unwrap().anotar_sha_pendiente("A1", TipoParte::Original, &sha, false).unwrap();
    assert_eq!(ronda(d.path(), &reg, &mut f).verificadas, 1);
    assert_eq!(s.total_peticiones(), 1);
}

#[test]
fn archivo_final_con_sha_pendiente_distinto_no_se_adopta() {
    let j = jpeg(500);
    let s = servidor(vec![("/j", Comportamiento::Servir(j.clone()))]);
    let mut f = FuenteFalsa { assets: vec![asset("A1", "IMG_0001", AGO, "PrimarySync", vec![parte(TipoParte::Original, &j, s.url("/j"), "JPG")])], ..Default::default() };
    let d = tempfile::tempdir().unwrap();
    let reg = nuevo_registro();
    ronda(d.path(), &reg, &mut f);
    reg.lock().unwrap().marcar_para_redescarga("A1", TipoParte::Original, "prueba").unwrap();
    reg.lock().unwrap().anotar_sha_pendiente("A1", TipoParte::Original, "otro", false).unwrap();
    assert_eq!(ronda(d.path(), &reg, &mut f).verificadas, 1);
    assert_eq!(s.total_peticiones(), 2);
}

#[test]
fn filtro_por_mes() {
    let (a, b) = (jpeg(100), jpeg(200));
    let s = servidor(vec![("/a", Comportamiento::Servir(a.clone())), ("/b", Comportamiento::Servir(b.clone()))]);
    let mut f = FuenteFalsa {
        assets: vec![
            asset("A1", "IMG_0001", AGO, "PrimarySync", vec![parte(TipoParte::Original, &a, s.url("/a"), "JPG")]),
            asset("A2", "IMG_0002", SEP, "PrimarySync", vec![parte(TipoParte::Original, &b, s.url("/b"), "JPG")]),
        ],
        ..Default::default()
    };
    let d = tempfile::tempdir().unwrap();
    assert_eq!(ronda_con(d.path(), &nuevo_registro(), &mut f, 2, Some("2026-08")).verificadas, 1);
    assert_eq!(s.peticiones_a("/b").len(), 0);
}

#[test]
fn un_part_ajeno_ocupa_el_nombre() {
    let j = jpeg(100);
    let s = servidor(vec![("/j", Comportamiento::Servir(j.clone()))]);
    let mut f = FuenteFalsa { assets: vec![asset("A1xxxx", "IMG_0001", AGO, "PrimarySync", vec![parte(TipoParte::Original, &j, s.url("/j"), "JPG")])], ..Default::default() };
    let d = tempfile::tempdir().unwrap();
    let carpeta = icdown::layout::en_disco(d.path(), r"2026\2026-08");
    std::fs::create_dir_all(&carpeta).unwrap();
    std::fs::write(carpeta.join("IMG_0001.JPG.part"), b"de otro").unwrap();
    assert_eq!(ronda(d.path(), &nuevo_registro(), &mut f).verificadas, 1);
    assert_eq!(std::fs::read(carpeta.join("IMG_0001 (a1xxxx).JPG")).unwrap(), j);
    assert_eq!(std::fs::read(carpeta.join("IMG_0001.JPG.part")).unwrap(), b"de otro");
}

#[test]
fn edicion_nueva_no_reanuda_el_part_viejo() {
    let viejo = heic(3_000_000);
    let mut nuevo = viejo.clone();
    let n = nuevo.len();
    nuevo[n - 1] ^= 0xFF; // mismo tamaño, otro contenido
    let s = servidor(vec![
        ("/v", Comportamiento::CortarPrimeraVez { datos: viejo.clone(), tras: 1_000_000 }),
        ("/n", Comportamiento::Servir(nuevo.clone())),
    ]);
    let mut p = parte(TipoParte::Original, &viejo, s.url("/v"), "HEIC");
    let mut f = FuenteFalsa { assets: vec![asset("A1", "IMG_0001", AGO, "PrimarySync", vec![p.clone()])], ..Default::default() };
    let d = tempfile::tempdir().unwrap();
    let reg = nuevo_registro();
    assert_eq!(ronda(d.path(), &reg, &mut f).fallidas, 1);
    p.url = s.url("/n");
    p.checksum = "otro".into();
    f.assets[0].partes = vec![p];
    reg.lock().unwrap().reintentar_ya().unwrap();
    assert_eq!(ronda(d.path(), &reg, &mut f).verificadas, 1);
    assert_eq!(s.peticiones_a("/n")[0].rango, None);
    let final_ = icdown::layout::en_disco(d.path(), r"2026\2026-08\IMG_0001.HEIC");
    assert_eq!(std::fs::read(&final_).unwrap(), nuevo);
    assert!(!descarga::ruta_meta(&descarga::ruta_part(&final_)).exists());
}

/// URL con caducidad `e=<unix>` como las de iCloud (cvws-h2.icloud-content.com).
fn con_caducidad(url: String, e: i64) -> String {
    format!("{url}?o=x&e={e}&v=1")
}

fn errores(reg: &Mutex<Registro>) -> Vec<String> {
    reg.lock().unwrap().errores(100).unwrap()
}

fn leer_bitacora(destino: &Path) -> String {
    let Ok(dir) = std::fs::read_dir(destino.join(".icdown").join("logs")) else { return String::new() };
    dir.map(|e| std::fs::read_to_string(e.unwrap().path()).unwrap()).collect()
}

#[test]
fn enlace_a_punto_de_caducar_se_aplaza_sin_contar_fallo() {
    let (a, b) = (jpeg(100), jpeg(200));
    let s = servidor(vec![("/a", Comportamiento::Servir(a.clone())), ("/b", Comportamiento::Servir(b.clone()))]);
    let ahora = motor::ahora();
    let mut f = FuenteFalsa {
        assets: vec![
            // El primer trabajo ya caduca (p. ej. el listado tardó demasiado).
            asset("A1", "IMG_0001", AGO, "PrimarySync", vec![parte(TipoParte::Original, &a, con_caducidad(s.url("/a"), ahora + 30), "JPG")]),
            asset("A2", "IMG_0002", SEP, "PrimarySync", vec![parte(TipoParte::Original, &b, con_caducidad(s.url("/b"), ahora + 3600), "JPG")]),
        ],
        ..Default::default()
    };
    let d = tempfile::tempdir().unwrap();
    let reg = nuevo_registro();
    let r = ronda_con(d.path(), &reg, &mut f, 1, None);
    assert_eq!((r.fin, r.fallidas, r.aplazadas, r.verificadas), (FinRonda::Relistar, 0, 1, 1));
    assert_eq!(s.peticiones_a("/a").len(), 0, "no se descarga un enlace que va a caducar");
    // Sigue pendiente, sin penalización: con enlace nuevo se copia en la ronda siguiente.
    let p = reg.lock().unwrap().partes_pendientes(motor::ahora(), None).unwrap();
    assert_eq!(p.len(), 1);
    assert!(errores(&reg).is_empty(), "no se anota ningún fallo");
    f.assets[0].partes[0].url = con_caducidad(s.url("/a"), motor::ahora() + 3600);
    let r = ronda_con(d.path(), &reg, &mut f, 1, None);
    assert_eq!((r.fin, r.verificadas, r.aplazadas), (FinRonda::Completa, 1, 0));
}

#[test]
fn todo_caducado_tras_listar_reciente_se_intenta_igual() {
    // Reloj desfasado: todo parece caducado nada más listar. No puede quedarse relistando sin fin.
    let a = jpeg(100);
    let s = servidor(vec![("/a", Comportamiento::Servir(a.clone())), ("/g", Comportamiento::Estado(410))]);
    let pasado = motor::ahora() - 7200;
    let mut f = FuenteFalsa {
        assets: vec![
            asset("A1", "IMG_0001", AGO, "PrimarySync", vec![parte(TipoParte::Original, &a, con_caducidad(s.url("/a"), pasado), "JPG")]),
            asset("A2", "IMG_0002", SEP, "PrimarySync", vec![parte(TipoParte::Original, &a, con_caducidad(s.url("/g"), pasado), "JPG")]),
        ],
        ..Default::default()
    };
    let d = tempfile::tempdir().unwrap();
    let reg = nuevo_registro();
    let r = ronda_con(d.path(), &reg, &mut f, 1, None);
    assert_eq!((r.verificadas, r.fallidas, r.aplazadas), (1, 1, 0));
    assert_eq!(s.peticiones_a("/g").len(), 1);
    assert_eq!(reg.lock().unwrap().partes_pendientes(motor::ahora(), None).unwrap().len(), 0, "el fallo cuenta y espera");
}

#[test]
fn enlace_reciente_rechazado_cuenta_como_fallo_real() {
    let a = jpeg(100);
    let s = servidor(vec![("/g", Comportamiento::Estado(410))]);
    let mut f = FuenteFalsa {
        assets: vec![asset("A1", "IMG_0001", AGO, "PrimarySync", vec![parte(TipoParte::Original, &a, con_caducidad(s.url("/g"), motor::ahora() + 3600), "JPG")])],
        ..Default::default()
    };
    let d = tempfile::tempdir().unwrap();
    let reg = nuevo_registro();
    let r = ronda_con(d.path(), &reg, &mut f, 1, None);
    assert_eq!((r.fallidas, r.aplazadas), (1, 0));
    // Relistar no borra un fallo real: si no, un asset roto se reintentaría sin fin.
    ronda_con(d.path(), &reg, &mut f, 1, None);
    assert!(reg.lock().unwrap().partes_pendientes(motor::ahora(), None).unwrap().is_empty());
}

#[test]
fn aplazadas_dejan_una_sola_linea_en_la_bitacora() {
    let a = jpeg(100);
    let s = servidor(vec![("/a", Comportamiento::Servir(a.clone()))]);
    let ahora = motor::ahora();
    let mut assets = vec![asset("A0", "IMG_0000", AGO, "PrimarySync", vec![parte(TipoParte::Original, &a, con_caducidad(s.url("/a"), ahora + 3600), "JPG")])];
    for i in 1..=5 {
        assets.push(asset(&format!("A{i}"), &format!("IMG_000{i}"), SEP, "PrimarySync", vec![parte(TipoParte::Original, &a, con_caducidad(s.url("/a"), ahora + 10), "JPG")]));
    }
    let mut f = FuenteFalsa { assets, ..Default::default() };
    let d = tempfile::tempdir().unwrap();
    let r = ronda_con(d.path(), &nuevo_registro(), &mut f, 2, None);
    assert_eq!((r.fin, r.aplazadas, r.fallidas), (FinRonda::Relistar, 5, 0));
    let log = leer_bitacora(d.path());
    assert!(!log.contains("FALLO"), "{log}");
    assert_eq!(log.matches("ENLACES CADUCAN: 5 archivos aplazados hasta el próximo listado").count(), 1, "{log}");
}

#[test]
fn enlace_que_caduca_a_media_ronda_aplaza_el_resto_sin_contar_fallos() {
    let a = jpeg(100);
    let s = servidor(vec![("/g", Comportamiento::EstadoTras { codigo: 410, ms: 3000 }), ("/a", Comportamiento::Servir(a.clone()))]);
    let ahora = motor::ahora();
    let lejos = ahora + 3600;
    let mut f = FuenteFalsa {
        assets: vec![
            // Vale al empezar (122 s > margen de 120 s), pero cuando iCloud responde ya está en el margen.
            asset("A1", "IMG_0001", AGO, "PrimarySync", vec![parte(TipoParte::Original, &a, con_caducidad(s.url("/g"), ahora + 122), "JPG")]),
            asset("A2", "IMG_0002", SEP, "PrimarySync", vec![parte(TipoParte::Original, &a, con_caducidad(s.url("/a"), lejos), "JPG")]),
            asset("A3", "IMG_0003", SEP, "PrimarySync", vec![parte(TipoParte::Original, &a, con_caducidad(s.url("/a"), lejos), "JPG")]),
        ],
        ..Default::default()
    };
    let d = tempfile::tempdir().unwrap();
    let reg = nuevo_registro();
    let r = ronda_con(d.path(), &reg, &mut f, 1, None);
    assert_eq!((r.fin, r.fallidas, r.aplazadas, r.verificadas), (FinRonda::Relistar, 0, 3, 0));
    assert_eq!(s.peticiones_a("/a").len(), 0, "tras la caducidad no se empiezan más descargas");
    assert!(errores(&reg).is_empty());
    assert_eq!(reg.lock().unwrap().partes_pendientes(motor::ahora(), None).unwrap().len(), 3);
}

fn con_renovacion(assets: Vec<icdown::modelo::AssetIcloud>) -> FuenteFalsa {
    FuenteFalsa { assets, renovar: true, ..Default::default() }
}

#[test]
fn enlaces_que_caducan_a_media_ronda_se_renuevan_sin_relistar() {
    let a = jpeg(100);
    let s = servidor(vec![("/a", Comportamiento::Servir(a.clone()))]);
    let ahora = motor::ahora();
    let mut assets = vec![asset("A0", "IMG_0000", AGO, "PrimarySync", vec![parte(TipoParte::Original, &a, con_caducidad(s.url("/a"), ahora + 3600), "JPG")])];
    for i in 1..=5 {
        assets.push(asset(&format!("A{i}"), &format!("IMG_000{i}"), SEP, "PrimarySync", vec![parte(TipoParte::Original, &a, con_caducidad(s.url("/a"), ahora + 10), "JPG")]));
    }
    let mut f = con_renovacion(assets);
    let d = tempfile::tempdir().unwrap();
    let reg = nuevo_registro();
    let avisos = Mutex::new(vec![]);
    let op = OpcionesRonda { concurrencia: 1, mes: None };
    let r = motor::ejecutar_ronda(d.path(), &reg, &mut f, &descarga::cliente(), &op, &AtomicBool::new(false), &|a: Avance| {
        if let Avance::RenovandoEnlaces { n, .. } = a {
            avisos.lock().unwrap().push(n);
        }
    })
    .unwrap();
    assert_eq!((r.fin, r.verificadas, r.aplazadas, r.fallidas), (FinRonda::Completa, 6, 0, 0));
    assert_eq!(f.refrescos, vec![vec!["A1", "A2", "A3", "A4", "A5"]], "un solo lote con todos los que caducan");
    assert_eq!(*avisos.lock().unwrap(), vec![5], "el panel ve «Renovando enlaces (5)…»");
    assert_eq!(s.peticiones_a("/a").len(), 6);
    let r = reg.lock().unwrap();
    assert_eq!(r.assets_ultimo_listado().unwrap(), Some(6), "renovar no cambia la marca del listado");
    assert!(r.partes_pendientes(motor::ahora(), None).unwrap().is_empty());
    drop(r);
    let log = leer_bitacora(d.path());
    assert!(log.contains("ENLACES RENOVADOS: 5 assets en 1 lotes"), "{log}");
    assert!(!log.contains("ENLACES CADUCAN"), "{log}");
}

#[test]
fn renovar_con_varios_hilos_completa_la_ronda() {
    let a = jpeg(100);
    let s = servidor(vec![("/a", Comportamiento::Servir(a.clone()))]);
    let ahora = motor::ahora();
    let mut assets = vec![asset("A0", "IMG_0000", AGO, "PrimarySync", vec![parte(TipoParte::Original, &a, con_caducidad(s.url("/a"), ahora + 3600), "JPG")])];
    for i in 1..=30 {
        assets.push(asset(&format!("A{i}"), &format!("IMG_{i:04}"), SEP, "PrimarySync", vec![parte(TipoParte::Original, &a, con_caducidad(s.url("/a"), ahora + 10), "JPG")]));
    }
    let mut f = con_renovacion(assets);
    let d = tempfile::tempdir().unwrap();
    let reg = nuevo_registro();
    let r = ronda_con(d.path(), &reg, &mut f, 3, None);
    assert_eq!((r.fin, r.verificadas, r.aplazadas, r.fallidas), (FinRonda::Completa, 31, 0, 0));
    assert!(!f.refrescos.is_empty() && f.refrescos.len() <= 3, "{:?}", f.refrescos);
}

#[test]
fn si_no_se_puede_renovar_se_relista_como_antes() {
    let a = jpeg(100);
    let s = servidor(vec![("/a", Comportamiento::Servir(a.clone()))]);
    let ahora = motor::ahora();
    let mut assets = vec![asset("A0", "IMG_0000", AGO, "PrimarySync", vec![parte(TipoParte::Original, &a, con_caducidad(s.url("/a"), ahora + 3600), "JPG")])];
    for i in 1..=3 {
        assets.push(asset(&format!("A{i}"), &format!("IMG_000{i}"), SEP, "PrimarySync", vec![parte(TipoParte::Original, &a, con_caducidad(s.url("/a"), ahora + 10), "JPG")]));
    }
    // Puente antiguo: `refrescar` falla.
    let mut f = FuenteFalsa { assets, ..Default::default() };
    let d = tempfile::tempdir().unwrap();
    let reg = nuevo_registro();
    let r = ronda_con(d.path(), &reg, &mut f, 1, None);
    assert_eq!((r.fin, r.verificadas, r.aplazadas, r.fallidas), (FinRonda::Relistar, 1, 3, 0));
    assert_eq!(f.refrescos.len(), 1, "tras un fallo no se insiste en la misma ronda");
    assert!(errores(&reg).is_empty());
    assert_eq!(reg.lock().unwrap().partes_pendientes(motor::ahora(), None).unwrap().len(), 3);
    let log = leer_bitacora(d.path());
    assert!(log.contains("no se pudieron renovar los enlaces"), "{log}");
}

#[test]
fn lo_que_falta_al_renovar_queda_pendiente_sin_relistar() {
    let a = jpeg(100);
    let s = servidor(vec![("/a", Comportamiento::Servir(a.clone()))]);
    let ahora = motor::ahora();
    let mut f = con_renovacion(vec![
        asset("A0", "IMG_0000", AGO, "PrimarySync", vec![parte(TipoParte::Original, &a, con_caducidad(s.url("/a"), ahora + 3600), "JPG")]),
        asset("A1", "IMG_0001", SEP, "PrimarySync", vec![
            parte(TipoParte::Original, &a, con_caducidad(s.url("/a"), ahora + 10), "JPG"),
            parte(TipoParte::Live, &mov(50), con_caducidad(s.url("/a"), ahora + 10), "MOV"),
        ]),
        asset("A2", "IMG_0002", SEP, "PrimarySync", vec![parte(TipoParte::Original, &a, con_caducidad(s.url("/a"), ahora + 10), "JPG")]),
    ]);
    f.faltan = vec!["A1".into()];
    let d = tempfile::tempdir().unwrap();
    let reg = nuevo_registro();
    let r = ronda_con(d.path(), &reg, &mut f, 1, None);
    assert_eq!((r.fin, r.verificadas, r.aplazadas, r.fallidas), (FinRonda::Completa, 2, 2, 0));
    assert_eq!(f.refrescos, vec![vec!["A1", "A2"]], "las dos partes de A1 salen de la cola de una vez");
    let p = reg.lock().unwrap().partes_pendientes(motor::ahora(), None).unwrap();
    assert_eq!(p.iter().map(|t| t.asset_id.as_str()).collect::<Vec<_>>(), vec!["A1", "A1"], "siguen pendientes, sin penalizar");
    assert!(errores(&reg).is_empty());
}

#[test]
fn enlace_rechazado_a_media_descarga_se_renueva_y_se_reanuda() {
    let a = jpeg(100);
    let s = servidor(vec![("/g", Comportamiento::EstadoTras { codigo: 410, ms: 3000 }), ("/a", Comportamiento::Servir(a.clone()))]);
    let ahora = motor::ahora();
    let lejos = ahora + 3600;
    let mut f = con_renovacion(vec![
        // Vale al empezar (122 s > margen de 120 s), pero cuando iCloud responde ya está en el margen.
        asset("A1", "IMG_0001", AGO, "PrimarySync", vec![parte(TipoParte::Original, &a, con_caducidad(s.url("/g"), ahora + 122), "JPG")]),
        asset("A2", "IMG_0002", SEP, "PrimarySync", vec![parte(TipoParte::Original, &a, con_caducidad(s.url("/a"), lejos), "JPG")]),
    ]);
    f.renovados = vec![asset("A1", "IMG_0001", AGO, "PrimarySync", vec![parte(TipoParte::Original, &a, con_caducidad(s.url("/a"), lejos), "JPG")])];
    let d = tempfile::tempdir().unwrap();
    let reg = nuevo_registro();
    let r = ronda_con(d.path(), &reg, &mut f, 1, None);
    assert_eq!((r.fin, r.verificadas, r.aplazadas, r.fallidas), (FinRonda::Completa, 2, 0, 0));
    assert_eq!(f.refrescos, vec![vec!["A1"]]);
    assert!(errores(&reg).is_empty());
}

#[test]
fn enlace_recien_renovado_y_rechazado_es_fallo_real() {
    let a = jpeg(100);
    let s = servidor(vec![("/g", Comportamiento::EstadoTras { codigo: 410, ms: 3000 })]);
    let ahora = motor::ahora();
    let mut f = con_renovacion(vec![asset("A1", "IMG_0001", AGO, "PrimarySync", vec![parte(TipoParte::Original, &a, con_caducidad(s.url("/g"), ahora + 122), "JPG")])]);
    let d = tempfile::tempdir().unwrap();
    let reg = nuevo_registro();
    let r = ronda_con(d.path(), &reg, &mut f, 1, None);
    // Primer rechazo: caducidad → enlace nuevo (e lejos). Segundo rechazo con enlace recién pedido: fallo real.
    assert_eq!((r.fin, r.fallidas, r.aplazadas), (FinRonda::Relistar, 1, 0));
    assert_eq!(f.refrescos.len(), 1, "no se renueva sin fin");
    assert_eq!(s.peticiones_a("/g").len(), 2);
}
