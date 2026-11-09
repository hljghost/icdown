mod comun;
use comun::*;
use icdown::bandeja::estado::{EventoUi, Fase};
use icdown::bandeja::hilo_motor::{self, Orden};
use icdown::config::Config;
use icdown::fuente::{FuenteFalsa, FuenteIcloud};
use icdown::modelo::TipoParte;
use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

#[test]
fn primera_ronda_copia_y_avisa() {
    let datos = jpeg(1000);
    let s = servidor(vec![("/a", Comportamiento::Servir(datos.clone()))]);
    let destino = tempfile::tempdir().unwrap();
    let cfg = Config { destino: Some(destino.path().to_path_buf()), apple_id: Some("yo@x.com".into()), ..Config::default() };
    let a = asset("A1", "IMG_0001", "2026-08-10T19:37:18-04:00", "PrimarySync", vec![parte(TipoParte::Original, &datos, s.url("/a"), "JPG")]);

    let (tx, rx) = mpsc::channel();
    let (tx_ev, rx_ev) = mpsc::channel::<EventoUi>();
    let tx_ev = Mutex::new(tx_ev);
    let h = hilo_motor::lanzar(
        rx,
        Arc::new(AtomicBool::new(false)),
        Box::new(move || cfg.clone()),
        Box::new(move |_| Ok(Box::new(FuenteFalsa { assets: vec![a.clone()], ..Default::default() }) as Box<dyn FuenteIcloud>)),
        Box::new(move |e| {
            let _ = tx_ev.lock().unwrap().send(e);
        }),
    );

    let limite = Instant::now() + Duration::from_secs(20);
    let (mut aviso, mut al_dia) = (false, false);
    while Instant::now() < limite && !(aviso && al_dia) {
        match rx_ev.recv_timeout(Duration::from_secs(1)) {
            // El texto sale en el idioma del sistema: español o inglés.
            Ok(EventoUi::Aviso { texto, .. }) if texto.starts_with("1 archivos") || texto.starts_with("1 new files") => aviso = true,
            Ok(EventoUi::Estado(e)) if e.fase == Fase::Inactiva && e.verificadas == 1 && e.total == 1 => al_dia = true,
            _ => {}
        }
    }
    assert!(aviso && al_dia, "aviso={aviso} al_dia={al_dia}");
    assert!(icdown::layout::en_disco(destino.path(), r"2026\2026-08\IMG_0001.JPG").exists());
    tx.send(Orden::Salir).unwrap();
    h.join().unwrap();
}

#[test]
fn pausar_que_llega_justo_antes_de_la_ronda_no_se_pierde() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let datos = jpeg(1000);
    let s = servidor(vec![("/a", Comportamiento::Servir(datos.clone()))]);
    let destino = tempfile::tempdir().unwrap();
    let cfg = Config { destino: Some(destino.path().to_path_buf()), apple_id: Some("yo@x.com".into()), ..Config::default() };
    let a = asset("A1", "IMG_0001", "2026-08-10T19:37:18-04:00", "PrimarySync", vec![parte(TipoParte::Original, &datos, s.url("/a"), "JPG")]);

    let (tx, rx) = mpsc::channel();
    let cancelar = Arc::new(AtomicBool::new(false));
    // Como hace la bandeja: CopiarAhora y enseguida Pausar (con `cancelar` a true).
    tx.send(Orden::CopiarAhora).unwrap();
    cancelar.store(true, Ordering::SeqCst);
    tx.send(Orden::Pausar).unwrap();
    let aperturas = Arc::new(AtomicUsize::new(0));
    let ap = aperturas.clone();
    let (tx_ev, rx_ev) = mpsc::channel::<EventoUi>();
    let tx_ev = Mutex::new(tx_ev);
    let h = hilo_motor::lanzar(
        rx,
        cancelar,
        Box::new(move || cfg.clone()),
        Box::new(move |_| {
            ap.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(FuenteFalsa { assets: vec![a.clone()], ..Default::default() }) as Box<dyn FuenteIcloud>)
        }),
        Box::new(move |e| {
            let _ = tx_ev.lock().unwrap().send(e);
        }),
    );
    let limite = Instant::now() + Duration::from_secs(20);
    let mut pausada = false;
    while Instant::now() < limite && !pausada {
        if let Ok(EventoUi::Estado(e)) = rx_ev.recv_timeout(Duration::from_secs(1)) {
            pausada = e.fase == Fase::Pausada;
        }
    }
    tx.send(Orden::Salir).unwrap();
    h.join().unwrap();
    assert!(pausada);
    assert_eq!(aperturas.load(Ordering::SeqCst), 0, "no debe empezar una ronda estando en pausa");
    assert!(!icdown::layout::en_disco(destino.path(), r"2026\2026-08\IMG_0001.JPG").exists());
}

/// Fuente falsa compartida entre aperturas (como la cuenta de iCloud real: lo borrado sigue borrado).
struct Compartida(Arc<Mutex<FuenteFalsa>>);

impl FuenteIcloud for Compartida {
    fn listar(&mut self, f: &mut dyn FnMut(icdown::modelo::AssetIcloud) -> bool) -> Result<usize, icdown::fuente::ErrorFuente> {
        self.0.lock().unwrap().listar(f)
    }
    fn refrescar(&mut self, ids: &[String], f: &mut dyn FnMut(icdown::modelo::AssetIcloud) -> bool) -> Result<Vec<String>, icdown::fuente::ErrorFuente> {
        self.0.lock().unwrap().refrescar(ids, f)
    }
    fn borrar(&mut self, ids: &[String]) -> Result<icdown::fuente::ResultadoBorrado, icdown::fuente::ErrorFuente> {
        self.0.lock().unwrap().borrar(ids)
    }
    fn restaurar(&mut self, ids: &[String]) -> Result<icdown::fuente::ResultadoBorrado, icdown::fuente::ErrorFuente> {
        self.0.lock().unwrap().restaurar(ids)
    }
}

#[test]
fn liberar_desde_la_bandeja_revisa_borra_y_restaura() {
    use icdown::bandeja::asistente::EventoLiberar;
    use icdown::criterio::Criterio;
    use icdown::progreso_liberar::{self, FaseLiberar};
    let (d1, d2) = (jpeg(1000), jpeg(2000));
    let s = servidor(vec![("/a", Comportamiento::Servir(d1.clone())), ("/b", Comportamiento::Servir(d2.clone()))]);
    let destino = tempfile::tempdir().unwrap();
    let cfg = Config { destino: Some(destino.path().to_path_buf()), apple_id: Some("yo@x.com".into()), ..Config::default() };
    let fuente = Arc::new(Mutex::new(FuenteFalsa {
        assets: vec![
            asset("A1", "IMG_0001", "2024-08-10T19:37:18-04:00", "PrimarySync", vec![parte(TipoParte::Original, &d1, s.url("/a"), "JPG")]),
            asset("B1", "IMG_0002", "2025-03-10T19:37:18-04:00", "PrimarySync", vec![parte(TipoParte::Original, &d2, s.url("/b"), "JPG")]),
        ],
        ..Default::default()
    }));
    let (tx, rx) = mpsc::channel();
    let (tx_ev, rx_ev) = mpsc::channel::<EventoUi>();
    let tx_ev = Mutex::new(tx_ev);
    let f2 = fuente.clone();
    let control = Arc::new(icdown::liberar::ControlBorrado::default());
    let h = hilo_motor::lanzar_con(
        rx,
        Arc::new(AtomicBool::new(false)),
        control,
        Box::new(move || cfg.clone()),
        Box::new(move |_| Ok(Box::new(Compartida(f2.clone())) as Box<dyn FuenteIcloud>)),
        Box::new(move |e| {
            let _ = tx_ev.lock().unwrap().send(e);
        }),
    );
    let esperar = |pred: &dyn Fn(&EventoUi) -> bool| {
        let limite = Instant::now() + Duration::from_secs(20);
        while Instant::now() < limite {
            if let Ok(e) = rx_ev.recv_timeout(Duration::from_millis(200)) {
                if pred(&e) {
                    return Some(e);
                }
            }
        }
        None
    };
    // Primera ronda: todo copiado.
    esperar(&|e| matches!(e, EventoUi::Estado(e) if e.fase == Fase::Inactiva && e.verificadas == 2)).expect("ronda");
    // Solo 2024.
    tx.send(Orden::LiberarRevisar(Criterio::Anios([2024].into_iter().collect()))).unwrap();
    let Some(EventoUi::Liberar(EventoLiberar::Plan(plan))) = esperar(&|e| matches!(e, EventoUi::Liberar(_))) else { panic!("sin plan") };
    assert_eq!((plan.elementos, plan.bytes), (1, d1.len() as u64));
    assert!(plan.lista.as_ref().is_some_and(|l| l.exists()));
    let reg = icdown::registro::Registro::abrir_lectura(&icdown::layout::en_disco(destino.path(), r".icdown\registro.db")).unwrap();
    assert_eq!(progreso_liberar::leer(&reg).unwrap().fase, FaseLiberar::Confirmando);
    tx.send(Orden::LiberarEjecutar).unwrap();
    let Some(EventoUi::Liberar(EventoLiberar::Terminado(fin))) = esperar(&|e| matches!(e, EventoUi::Liberar(_))) else { panic!("sin final") };
    assert_eq!((fin.borrados, fin.fallidos, fin.cancelado), (1, 0, false));
    assert_eq!(fuente.lock().unwrap().borrados, vec!["A1"]);
    assert_eq!(progreso_liberar::leer(&reg).unwrap().fase, FaseLiberar::Terminado);
    tx.send(Orden::LiberarRestaurar(fin.sesion.clone())).unwrap();
    let Some(EventoUi::Liberar(r)) = esperar(&|e| matches!(e, EventoUi::Liberar(_))) else { panic!("sin restaurar") };
    assert_eq!(r, EventoLiberar::Restaurado { restaurados: 1, fallidos: 0 });
    assert_eq!(fuente.lock().unwrap().assets.len(), 2);
    // Revisar y descartar: no se borra nada y no queda avance.
    tx.send(Orden::LiberarRevisar(Criterio::ConservarDias(30))).unwrap();
    assert!(matches!(esperar(&|e| matches!(e, EventoUi::Liberar(_))), Some(EventoUi::Liberar(EventoLiberar::Plan(p))) if p.elementos == 2));
    tx.send(Orden::LiberarDescartar).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(progreso_liberar::leer(&reg), None);
    tx.send(Orden::Salir).unwrap();
    h.join().unwrap();
    assert_eq!(fuente.lock().unwrap().borrados, vec!["A1"], "nada más se borró");
}
