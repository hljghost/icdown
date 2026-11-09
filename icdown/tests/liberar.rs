mod comun;
use comun::*;
use icdown::descarga;
use icdown::fuente::FuenteFalsa;
use icdown::liberar;
use icdown::modelo::TipoParte;
use icdown::motor::{self, Avance, OpcionesRonda};
use icdown::registro::Registro;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

struct Escenario {
    _s: Servidor,
    dir: tempfile::TempDir,
    reg: Mutex<Registro>,
    fuente: FuenteFalsa,
}

/// V1 y V2 viejas y personales; C1 vieja de Fototeca Compartida; P1 vieja que no se pudo copiar (403);
/// N1 de ayer.
fn escenario() -> Escenario {
    let (v1, v2, c1, n1) = (jpeg(100), jpeg(200), jpeg(300), jpeg(400));
    let s = servidor(vec![
        ("/v1", Comportamiento::Servir(v1.clone())),
        ("/v2", Comportamiento::Servir(v2.clone())),
        ("/c1", Comportamiento::Servir(c1.clone())),
        ("/p1", Comportamiento::Estado(403)),
        ("/n1", Comportamiento::Servir(n1.clone())),
    ]);
    let ayer = (chrono::Local::now() - chrono::Duration::days(1)).fixed_offset().to_rfc3339();
    let fuente = FuenteFalsa {
        assets: vec![
            asset("V1", "IMG_V1", "2025-01-10T10:00:00-05:00", "PrimarySync", vec![parte(TipoParte::Original, &v1, s.url("/v1"), "JPG")]),
            asset("V2", "IMG_V2", "2025-01-11T10:00:00-05:00", "PrimarySync", vec![parte(TipoParte::Original, &v2, s.url("/v2"), "JPG")]),
            asset("C1", "IMG_C1", "2025-01-12T10:00:00-05:00", "SharedSync-AB", vec![parte(TipoParte::Original, &c1, s.url("/c1"), "JPG")]),
            asset("P1", "IMG_P1", "2025-03-01T10:00:00-05:00", "PrimarySync", vec![parte(TipoParte::Original, &jpeg(10), s.url("/p1"), "JPG")]),
            asset("N1", "IMG_N1", &ayer, "PrimarySync", vec![parte(TipoParte::Original, &n1, s.url("/n1"), "JPG")]),
        ],
        ..Default::default()
    };
    let mut e = Escenario { _s: s, dir: tempfile::tempdir().unwrap(), reg: Mutex::new(Registro::en_memoria().unwrap()), fuente };
    let op = OpcionesRonda { concurrencia: 1, mes: None };
    motor::ejecutar_ronda(e.dir.path(), &e.reg, &mut e.fuente, &descarga::cliente(), &op, &AtomicBool::new(false), &|_: Avance| {}).unwrap();
    e
}

#[test]
fn prepara_solo_lo_seguro_y_borra() {
    let mut e = escenario();
    // V2 se altera en disco después de verificarse
    let ruta_v2 = icdown::layout::en_disco(e.dir.path(), r"2025\2025-01\IMG_V2.JPG");
    let mut v2 = std::fs::read(&ruta_v2).unwrap();
    v2[15] ^= 0xFF;
    std::fs::write(&ruta_v2, v2).unwrap();

    let plan = liberar::preparar(e.dir.path(), &e.reg, 30, motor::ahora(), &|_, _| {}).unwrap();
    assert_eq!(plan.listos.iter().map(|c| c.asset_id.as_str()).collect::<Vec<_>>(), vec!["V1"]);
    assert_eq!(plan.excluidos.iter().map(|(c, _)| c.asset_id.as_str()).collect::<Vec<_>>(), vec!["V2"]);
    assert_eq!(plan.no_verificados, 1); // P1
    assert_eq!(plan.bytes, 114);

    let r = liberar::ejecutar(e.dir.path(), &e.reg, &mut e.fuente, &plan.listos, &|_, _| {}).unwrap();
    assert_eq!(r.borrados, 1);
    assert_eq!(e.fuente.borrados, vec!["V1"]);
    let csv = std::fs::read_to_string(icdown::layout::en_disco(e.dir.path(), r".icdown\borrados.csv")).unwrap();
    assert!(csv.starts_with("fecha_borrado,asset_id,nombre,fecha_captura,archivos"));
    assert!(csv.contains("\"V1\""));
    assert!(icdown::layout::en_disco(e.dir.path(), r"2025\2025-01\IMG_V1.JPG").exists(), "nunca se borra del PC");

    let otra = liberar::preparar(e.dir.path(), &e.reg, 30, motor::ahora(), &|_, _| {}).unwrap();
    assert!(otra.listos.is_empty());
}

#[test]
fn las_copias_van_en_carpetas_reales_y_se_reverifican_ahi() {
    // En macOS, una ruta del registro con `\` unida tal cual crearía un archivo con barras invertidas
    // en el nombre. Las copias deben estar en carpetas de verdad (separador del sistema) y la
    // re-verificación previa al borrado debe encontrarlas ahí.
    let e = escenario();
    let carpeta = e.dir.path().join("2025").join("2025-01");
    assert!(carpeta.join("IMG_V1.JPG").is_file());
    assert!(carpeta.join("IMG_V2.JPG").is_file());
    let plan = liberar::preparar(e.dir.path(), &e.reg, 30, motor::ahora(), &|_, _| {}).unwrap();
    assert_eq!(plan.listos.iter().map(|c| c.asset_id.as_str()).collect::<Vec<_>>(), vec!["V1", "V2"]);
    assert!(plan.excluidos.is_empty(), "{:?}", plan.excluidos.iter().map(|(_, m)| m).collect::<Vec<_>>());
}

#[test]
fn los_rechazados_no_se_marcan() {
    let mut e = escenario();
    e.fuente.rechazar = vec!["V1".into()];
    let plan = liberar::preparar(e.dir.path(), &e.reg, 30, motor::ahora(), &|_, _| {}).unwrap();
    // Aquí V2 no se altera: listos = [V1, V2]; V1 es rechazada por iCloud (tras sus reintentos) y V2 se borra.
    let r = liberar::ejecutar_con(e.dir.path(), &e.reg, &mut e.fuente, &plan.listos, &|_, _| {}, &rapido()).unwrap();
    assert_eq!((r.borrados, r.fallidos.len()), (1, 1));
    let de_nuevo = liberar::preparar(e.dir.path(), &e.reg, 30, motor::ahora(), &|_, _| {}).unwrap();
    assert!(de_nuevo.listos.iter().any(|c| c.asset_id == "V1"));
}

#[test]
fn sin_listado_no_se_puede_liberar() {
    let d = tempfile::tempdir().unwrap();
    let reg = Mutex::new(Registro::en_memoria().unwrap());
    assert!(liberar::preparar(d.path(), &reg, 30, motor::ahora(), &|_, _| {}).is_err());
}

#[test]
fn assets_con_datos_no_copiables_no_se_proponen() {
    let v1 = jpeg(100);
    let s = servidor(vec![("/v1", Comportamiento::Servir(v1.clone()))]);
    let mut a = asset("V1", "IMG_V1", "2025-01-10T10:00:00-05:00", "PrimarySync", vec![parte(TipoParte::Original, &v1, s.url("/v1"), "JPG")]);
    a.liberable = false;
    let mut fuente = FuenteFalsa { assets: vec![a], ..Default::default() };
    let d = tempfile::tempdir().unwrap();
    let reg = Mutex::new(Registro::en_memoria().unwrap());
    let op = OpcionesRonda { concurrencia: 1, mes: None };
    let r = motor::ejecutar_ronda(d.path(), &reg, &mut fuente, &descarga::cliente(), &op, &AtomicBool::new(false), &|_: Avance| {}).unwrap();
    assert_eq!(r.verificadas, 1, "se copia igualmente");
    let plan = liberar::preparar(d.path(), &reg, 30, motor::ahora(), &|_, _| {}).unwrap();
    assert!(plan.listos.is_empty());
    assert_eq!((plan.no_verificados, plan.no_liberables), (0, 1));
}

#[test]
fn atipicos_confirmados_por_doble_descarga_son_liberables() {
    let (v1, a1) = (jpeg(100), jpeg_atipico(200));
    let s = servidor(vec![("/v1", Comportamiento::Servir(v1.clone())), ("/a1", Comportamiento::Servir(a1.clone()))]);
    let mut fuente = FuenteFalsa {
        assets: vec![
            asset("V1", "IMG_V1", "2025-01-10T10:00:00-05:00", "PrimarySync", vec![parte(TipoParte::Original, &v1, s.url("/v1"), "JPG")]),
            asset("A1", "IMG_A1", "2025-01-11T10:00:00-05:00", "PrimarySync", vec![parte(TipoParte::Original, &a1, s.url("/a1"), "JPG")]),
        ],
        ..Default::default()
    };
    let d = tempfile::tempdir().unwrap();
    let reg = Mutex::new(Registro::en_memoria().unwrap());
    let op = OpcionesRonda { concurrencia: 1, mes: None };
    let r = motor::ejecutar_ronda(d.path(), &reg, &mut fuente, &descarga::cliente(), &op, &AtomicBool::new(false), &|_: Avance| {}).unwrap();
    assert_eq!(r.verificadas, 2);
    let plan = liberar::preparar(d.path(), &reg, 30, motor::ahora(), &|_, _| {}).unwrap();
    assert_eq!(plan.listos.iter().map(|c| c.asset_id.as_str()).collect::<Vec<_>>(), vec!["V1", "A1"]);
    assert!(plan.excluidos.is_empty(), "{:?}", plan.excluidos);
    assert_eq!(plan.atipicos, 1);
}

#[test]
fn un_fallo_al_anotar_no_detiene_el_borrado_ni_pierde_registros() {
    let mut e = escenario();
    // El CSV no se puede escribir (en su lugar hay una carpeta).
    std::fs::create_dir_all(icdown::layout::en_disco(e.dir.path(), r".icdown\borrados.csv")).unwrap();
    let plan = liberar::preparar(e.dir.path(), &e.reg, 30, motor::ahora(), &|_, _| {}).unwrap();
    assert_eq!(plan.listos.len(), 2);
    let r = liberar::ejecutar(e.dir.path(), &e.reg, &mut e.fuente, &plan.listos, &|_, _| {}).unwrap();
    assert_eq!(r.borrados, 2);
    assert_eq!(r.errores.iter().filter(|(id, _)| !id.is_empty()).count(), 2, "{:?}", r.errores);
    let otra = liberar::preparar(e.dir.path(), &e.reg, 30, motor::ahora(), &|_, _| {}).unwrap();
    assert!(otra.listos.is_empty(), "lo borrado queda anotado en el registro");
}

#[test]
fn el_avance_queda_en_el_registro_por_fases() {
    use icdown::progreso_liberar::{self, FaseLiberar};
    let mut e = escenario();
    let fases = std::cell::RefCell::new(Vec::new());
    let fase_actual = |reg: &Mutex<Registro>| progreso_liberar::leer(&reg.lock().unwrap()).map(|p| p.fase);
    let plan = liberar::preparar(e.dir.path(), &e.reg, 30, motor::ahora(), &|_, _| fases.borrow_mut().push(fase_actual(&e.reg))).unwrap();
    let p = progreso_liberar::leer(&e.reg.lock().unwrap()).unwrap();
    assert_eq!((p.fase, p.total, p.bytes_total, p.dias), (FaseLiberar::Confirmando, 2, plan.bytes, 30));
    let reg = &e.reg;
    let r = liberar::ejecutar(e.dir.path(), reg, &mut e.fuente, &plan.listos, &|_, _| fases.borrow_mut().push(fase_actual(reg))).unwrap();
    assert_eq!(r.borrados, 2);
    let fases = fases.into_inner();
    assert!(fases.contains(&Some(FaseLiberar::Reverificando)), "{fases:?}");
    assert!(fases.contains(&Some(FaseLiberar::Borrando)), "{fases:?}");
    let p = progreso_liberar::leer(&e.reg.lock().unwrap()).unwrap();
    assert_eq!((p.fase, p.hechos, p.total, p.borrados, p.fallidos, p.bytes_hechos), (FaseLiberar::Terminado, 2, 2, 2, 0, plan.bytes));
    assert!(p.inicio_unix <= p.actualizado_unix);
    // Un liberar nuevo sin nada que borrar no deja nada que mostrar.
    let otra = liberar::preparar(e.dir.path(), &e.reg, 30, motor::ahora(), &|_, _| {}).unwrap();
    assert!(otra.listos.is_empty());
    assert_eq!(progreso_liberar::leer(&e.reg.lock().unwrap()), None);
}

#[test]
fn un_error_de_icloud_deja_el_avance_como_fallido() {
    use icdown::progreso_liberar::{self, FaseLiberar};
    let mut e = escenario();
    let plan = liberar::preparar(e.dir.path(), &e.reg, 30, motor::ahora(), &|_, _| {}).unwrap();
    e.fuente.sesion_caducada = true;
    assert!(liberar::ejecutar(e.dir.path(), &e.reg, &mut e.fuente, &plan.listos, &|_, _| {}).is_err());
    let p = progreso_liberar::leer(&e.reg.lock().unwrap()).unwrap();
    assert_eq!((p.fase, p.borrados), (FaseLiberar::Fallido, 0));
    assert!(p.error.unwrap().contains("borrando en iCloud"));
}

#[test]
fn por_anios_y_restaurar_la_sesion() {
    use icdown::criterio::Criterio;
    let mut e = escenario();
    // V1 y V2 son de 2025; pedir solo 2024 no propone nada.
    let c = Criterio::Anios([2024].into_iter().collect());
    assert!(liberar::preparar_criterio(e.dir.path(), &e.reg, &c, motor::ahora(), &|_, _| {}).unwrap().listos.is_empty());
    let c = Criterio::Anios([2025].into_iter().collect());
    let plan = liberar::preparar_criterio(e.dir.path(), &e.reg, &c, motor::ahora(), &|_, _| {}).unwrap();
    assert_eq!(plan.listos.len(), 2);
    let lista = liberar::escribir_plan_csv(e.dir.path(), &plan).unwrap();
    assert_eq!(std::fs::read_to_string(lista).unwrap().lines().count(), 3);
    // borrados.csv de una versión anterior: se le añade la columna de sesión.
    std::fs::write(icdown::layout::en_disco(e.dir.path(), r".icdown\borrados.csv"), "fecha_borrado,asset_id,nombre,fecha_captura,archivos\n\"x\",\"OLD\",\"n\",\"f\",\"a\"\n").unwrap();
    let r = liberar::ejecutar(e.dir.path(), &e.reg, &mut e.fuente, &plan.listos, &|_, _| {}).unwrap();
    assert_eq!((r.borrados, r.bytes), (2, plan.bytes));
    let csv = std::fs::read_to_string(icdown::layout::en_disco(e.dir.path(), r".icdown\borrados.csv")).unwrap();
    let lineas: Vec<&str> = csv.lines().collect();
    assert_eq!(lineas[0], "fecha_borrado,asset_id,nombre,fecha_captura,archivos,sesion");
    assert!(lineas[1].contains("OLD"), "las filas antiguas se conservan");
    assert!(lineas[2].ends_with(&format!("\"{}\"", r.sesion)), "{}", lineas[2]);
    assert_eq!(e.reg.lock().unwrap().ultima_sesion_borrado().unwrap(), Some(r.sesion.clone()));
    // Restaurar la liberación: vuelven a iCloud y al registro; se pueden volver a liberar.
    let rr = liberar::restaurar_sesion(e.dir.path(), &e.reg, &mut e.fuente, &r.sesion).unwrap();
    assert_eq!(rr.restaurados.len(), 2);
    assert!(rr.fallidos.is_empty());
    assert!(e.fuente.papelera.is_empty());
    assert!(e.reg.lock().unwrap().ids_de_sesion(&r.sesion).unwrap().is_empty());
    assert!(std::fs::read_to_string(icdown::layout::en_disco(e.dir.path(), r".icdown\restaurados.csv")).unwrap().contains("\"V1\""));
    assert_eq!(liberar::preparar_criterio(e.dir.path(), &e.reg, &c, motor::ahora(), &|_, _| {}).unwrap().listos.len(), 2);
    // Restaurar algo que no está en "Eliminado recientemente" falla sin tocar nada.
    let rr = liberar::restaurar(e.dir.path(), &e.reg, &mut e.fuente, &["V1".into()]).unwrap();
    assert_eq!(rr.fallidos.len(), 1);
}

#[test]
fn cancelar_y_pausar_entre_lotes() {
    use icdown::liberar::ControlBorrado;
    use icdown::progreso_liberar::{self, FaseLiberar};
    use std::sync::atomic::Ordering;
    let mut e = escenario();
    let plan = liberar::preparar(e.dir.path(), &e.reg, 30, motor::ahora(), &|_, _| {}).unwrap();
    let control = ControlBorrado::default();
    control.cancelar.store(true, Ordering::SeqCst);
    let r = liberar::ejecutar_con(e.dir.path(), &e.reg, &mut e.fuente, &plan.listos, &|_, _| {}, &control).unwrap();
    assert!(r.cancelado);
    assert_eq!(r.borrados, 0, "se cancela antes del primer lote");
    let p = progreso_liberar::leer(&e.reg.lock().unwrap()).unwrap();
    assert_eq!((p.fase, p.cancelado), (FaseLiberar::Terminado, true));
    // En pausa espera; al reanudar, borra todo.
    let control = std::sync::Arc::new(ControlBorrado::default());
    control.pausa.store(true, Ordering::SeqCst);
    let c2 = control.clone();
    let reanudar = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(400));
        c2.pausa.store(false, Ordering::SeqCst);
    });
    let inicio = std::time::Instant::now();
    let r = liberar::ejecutar_con(e.dir.path(), &e.reg, &mut e.fuente, &plan.listos, &|_, _| {}, &control).unwrap();
    reanudar.join().unwrap();
    assert!(inicio.elapsed() >= std::time::Duration::from_millis(350));
    assert_eq!((r.borrados, r.cancelado), (2, false));
}

fn rapido() -> liberar::ControlBorrado {
    liberar::ControlBorrado { politica: liberar::Politica::rapida(), ..Default::default() }
}

/// Muchos assets viejos ya copiados (sin servidor: se dan por verificados en el registro y en disco).
fn muchos(n: usize) -> (tempfile::TempDir, Mutex<Registro>, FuenteFalsa, Vec<icdown::registro::Candidato>) {
    let dir = tempfile::tempdir().unwrap();
    let datos = jpeg(64);
    let s = servidor(vec![("/x", Comportamiento::Servir(datos.clone()))]);
    let assets: Vec<_> = (0..n)
        .map(|i| asset(&format!("A{i:03}"), &format!("IMG_{i:03}"), "2024-01-10T10:00:00-05:00", "PrimarySync", vec![parte(TipoParte::Original, &datos, s.url("/x"), "JPG")]))
        .collect();
    let mut fuente = FuenteFalsa { assets, renovar: true, ..Default::default() };
    let reg = Mutex::new(Registro::en_memoria().unwrap());
    let op = OpcionesRonda { concurrencia: 4, mes: None };
    motor::ejecutar_ronda(dir.path(), &reg, &mut fuente, &descarga::cliente(), &op, &AtomicBool::new(false), &|_: Avance| {}).unwrap();
    let plan = liberar::preparar(dir.path(), &reg, 30, motor::ahora(), &|_, _| {}).unwrap();
    assert_eq!(plan.listos.len(), n);
    (dir, reg, fuente, plan.listos)
}

#[test]
fn si_apple_pide_esperar_se_espera_y_se_reintenta_el_mismo_lote() {
    use icdown::progreso_liberar;
    let n = 2 * liberar::TAMANO_LOTE + 20; // 3 lotes
    let (dir, reg, mut fuente, listos) = muchos(n);
    // Frena la 2.ª y la 3.ª llamada (el segundo lote, dos veces seguidas).
    fuente.frenar_llamadas = vec![1, 2];
    let r = liberar::ejecutar_con(dir.path(), &reg, &mut fuente, &listos, &|_, _| {}, &rapido()).unwrap();
    assert_eq!((r.borrados, r.fallidos.len()), (n, 0), "nada falla por el frenazo: {:?}", r.fallidos);
    assert_eq!(fuente.llamadas_borrar, 5, "3 lotes + 2 reintentos del segundo");
    let p = progreso_liberar::leer(&reg.lock().unwrap()).unwrap();
    assert_eq!((p.borrados, p.espera_hasta), (n as u64, None));
}

#[test]
fn si_apple_no_deja_nunca_se_para_sin_dar_por_fallidos_a_los_elementos_por_su_culpa() {
    let lote = liberar::TAMANO_LOTE;
    let n = lote + lote / 2 + 20;
    let (dir, reg, mut fuente, listos) = muchos(n);
    fuente.frenar_llamadas = (1..100).collect();
    let r = liberar::ejecutar_con(dir.path(), &reg, &mut fuente, &listos, &|_, _| {}, &rapido()).unwrap();
    assert_eq!(r.borrados, lote);
    assert_eq!(r.fallidos.len(), n - lote);
    assert!(r.fallidos.iter().all(|(_, m)| m == liberar::LIMITE_APPLE));
    assert_eq!(fuente.llamadas_borrar, 1 + 1 + liberar::Politica::rapida().max_esperas, "se rinde tras los frenazos seguidos permitidos");
    // Lo no borrado sigue propuesto para la próxima vez.
    assert_eq!(liberar::preparar(dir.path(), &reg, 30, motor::ahora(), &|_, _| {}).unwrap().listos.len(), n - lote);
}

#[test]
fn try_again_later_se_reintenta_al_final_con_datos_frescos() {
    let (dir, reg, mut fuente, listos) = muchos(60);
    fuente.ocupados_una_vez = vec!["A003".into(), "A055".into()];
    let r = liberar::ejecutar_con(dir.path(), &reg, &mut fuente, &listos, &|_, _| {}, &rapido()).unwrap();
    assert_eq!((r.borrados, r.fallidos.len()), (60, 0));
    assert_eq!(fuente.refrescos, vec![vec!["A003".to_string(), "A055".to_string()]], "se piden datos frescos antes de reintentar");
    assert_eq!(&fuente.borrados[58..], ["A003", "A055"]);
}
