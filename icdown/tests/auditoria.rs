mod comun;
use comun::*;
use icdown::auditoria;
use icdown::descarga;
use icdown::fuente::FuenteFalsa;
use icdown::modelo::TipoParte;
use icdown::motor::{self, Avance, OpcionesRonda};
use icdown::registro::Registro;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

#[test]
fn detecta_archivos_danados_y_ausentes_y_los_reprograma() {
    let (a, b, c) = (jpeg(300), jpeg(400), jpeg(500));
    let s = servidor(vec![("/a", Comportamiento::Servir(a.clone())), ("/b", Comportamiento::Servir(b.clone())), ("/c", Comportamiento::Servir(c.clone()))]);
    let f8 = "2026-08-10T19:37:18-04:00";
    let mut f = FuenteFalsa {
        assets: vec![
            asset("A1", "IMG_A", f8, "PrimarySync", vec![parte(TipoParte::Original, &a, s.url("/a"), "JPG")]),
            asset("A2", "IMG_B", f8, "PrimarySync", vec![parte(TipoParte::Original, &b, s.url("/b"), "JPG")]),
            asset("A3", "IMG_C", f8, "PrimarySync", vec![parte(TipoParte::Original, &c, s.url("/c"), "JPG")]),
        ],
        ..Default::default()
    };
    let d = tempfile::tempdir().unwrap();
    let reg = Mutex::new(Registro::en_memoria().unwrap());
    let op = OpcionesRonda { concurrencia: 2, mes: None };
    motor::ejecutar_ronda(d.path(), &reg, &mut f, &descarga::cliente(), &op, &AtomicBool::new(false), &|_: Avance| {}).unwrap();
    let carpeta = icdown::layout::en_disco(d.path(), r"2026\2026-08");
    let mut danado = b.clone();
    danado[10] ^= 0xFF; // mismo tamaño, contenido distinto
    std::fs::write(carpeta.join("IMG_B.JPG"), danado).unwrap();
    std::fs::remove_file(carpeta.join("IMG_C.JPG")).unwrap();

    let r = auditoria::verificar_todo(d.path(), &reg, &AtomicBool::new(false), &|_, _| {}).unwrap();
    assert_eq!(r.revisadas, 3);
    let mut malas: Vec<&str> = r.malas.iter().map(|(ruta, _)| ruta.as_str()).collect();
    malas.sort();
    assert_eq!(malas, vec![r"2026\2026-08\IMG_B.JPG", r"2026\2026-08\IMG_C.JPG"]);
    assert_eq!(reg.lock().unwrap().resumen().unwrap().verificadas, 1);
    assert_eq!(reg.lock().unwrap().partes_pendientes(motor::ahora(), None).unwrap().len(), 2);
}

#[test]
fn atipico_confirmado_se_reverifica_por_sha_sin_estructura() {
    let j = jpeg_atipico(400);
    let s = servidor(vec![("/j", Comportamiento::Servir(j.clone()))]);
    let f8 = "2026-08-10T19:37:18-04:00";
    let mut f = FuenteFalsa { assets: vec![asset("A1", "IMG_A", f8, "PrimarySync", vec![parte(TipoParte::Original, &j, s.url("/j"), "JPG")])], ..Default::default() };
    let d = tempfile::tempdir().unwrap();
    let reg = Mutex::new(Registro::en_memoria().unwrap());
    let op = OpcionesRonda { concurrencia: 1, mes: None };
    motor::ejecutar_ronda(d.path(), &reg, &mut f, &descarga::cliente(), &op, &AtomicBool::new(false), &|_: Avance| {}).unwrap();
    assert_eq!(reg.lock().unwrap().resumen().unwrap().estructura_atipica, 1);

    let r = auditoria::verificar_todo(d.path(), &reg, &AtomicBool::new(false), &|_, _| {}).unwrap();
    assert_eq!((r.revisadas, r.malas.len()), (1, 0), "{:?}", r.malas);

    let ruta = icdown::layout::en_disco(d.path(), r"2026\2026-08\IMG_A.JPG");
    let mut danado = j.clone();
    danado[100] ^= 0x01;
    std::fs::write(&ruta, &danado).unwrap();
    let r = auditoria::verificar_todo(d.path(), &reg, &AtomicBool::new(false), &|_, _| {}).unwrap();
    assert_eq!(r.malas.len(), 1);
    assert!(r.malas[0].1.contains("SHA-256"), "{:?}", r.malas);
}

#[test]
fn archivo_danado_se_vuelve_a_descargar_y_no_se_readopta() {
    let b = jpeg(400);
    let s = servidor(vec![("/b", Comportamiento::Servir(b.clone()))]);
    let f8 = "2026-08-10T19:37:18-04:00";
    let mut f = FuenteFalsa { assets: vec![asset("A2", "IMG_B", f8, "PrimarySync", vec![parte(TipoParte::Original, &b, s.url("/b"), "JPG")])], ..Default::default() };
    let d = tempfile::tempdir().unwrap();
    let reg = Mutex::new(Registro::en_memoria().unwrap());
    let op = OpcionesRonda { concurrencia: 1, mes: None };
    motor::ejecutar_ronda(d.path(), &reg, &mut f, &descarga::cliente(), &op, &AtomicBool::new(false), &|_: Avance| {}).unwrap();
    assert_eq!(s.total_peticiones(), 1);
    let ruta = icdown::layout::en_disco(d.path(), r"2026\2026-08\IMG_B.JPG");
    let mut danado = b.clone();
    danado[10] ^= 0xFF; // mismo tamaño, estructura intacta
    std::fs::write(&ruta, &danado).unwrap();

    let r = auditoria::verificar_todo(d.path(), &reg, &AtomicBool::new(false), &|_, _| {}).unwrap();
    assert_eq!(r.malas.len(), 1);
    let apartados: Vec<_> = std::fs::read_dir(icdown::layout::en_disco(d.path(), r".icdown\danados")).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    assert_eq!(apartados.len(), 1);
    assert!(apartados[0].ends_with("_IMG_B.JPG"), "{apartados:?}");

    let r2 = motor::ejecutar_ronda(d.path(), &reg, &mut f, &descarga::cliente(), &op, &AtomicBool::new(false), &|_: Avance| {}).unwrap();
    assert_eq!(r2.verificadas, 1);
    assert_eq!(s.total_peticiones(), 2, "debe volver a descargarse del servidor");
    assert_eq!(std::fs::read(&ruta).unwrap(), b);
}

#[test]
fn archivo_danado_no_apartado_tampoco_se_readopta() {
    // Aunque el archivo dañado siga en su sitio (p. ej. no se pudo apartar), no se acepta.
    let b = jpeg(400);
    let s = servidor(vec![("/b", Comportamiento::Servir(b.clone()))]);
    let f8 = "2026-08-10T19:37:18-04:00";
    let mut f = FuenteFalsa { assets: vec![asset("A2", "IMG_B", f8, "PrimarySync", vec![parte(TipoParte::Original, &b, s.url("/b"), "JPG")])], ..Default::default() };
    let d = tempfile::tempdir().unwrap();
    let reg = Mutex::new(Registro::en_memoria().unwrap());
    let op = OpcionesRonda { concurrencia: 1, mes: None };
    motor::ejecutar_ronda(d.path(), &reg, &mut f, &descarga::cliente(), &op, &AtomicBool::new(false), &|_: Avance| {}).unwrap();
    let ruta = icdown::layout::en_disco(d.path(), r"2026\2026-08\IMG_B.JPG");
    let mut danado = b.clone();
    danado[10] ^= 0xFF;
    std::fs::write(&ruta, &danado).unwrap();
    reg.lock().unwrap().marcar_para_redescarga("A2", TipoParte::Original, "prueba").unwrap();
    motor::ejecutar_ronda(d.path(), &reg, &mut f, &descarga::cliente(), &op, &AtomicBool::new(false), &|_: Avance| {}).unwrap();
    assert_eq!(s.total_peticiones(), 2);
    assert_eq!(std::fs::read(&ruta).unwrap(), b);
}
