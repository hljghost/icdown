//! icdown-tray: icono de bandeja (StatusNotifierItem) que lanza el motor de copia en segundo plano.
use anyhow::Context;
use fs2::FileExt;
use icdown::bandeja::estado::{Color, EstadoUi, EventoUi, Fase};
use icdown::bandeja::hilo_motor::{self, Orden};
use icdown::bandeja::{autoarranque, avisos};
use icdown::config::{self, Config};
use icdown::fuente::FuenteIcloud;
use icdown::i18n::{t, T};
use icdown::liberar::ControlBorrado;
use icdown::puente::Puente;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

enum Ev {
    Ui(EventoUi),
    Menu(MenuEvent),
}

/// Bloqueo de instancia única: un archivo con `flock` en la carpeta de configuración
/// (con `ICDOWN_DIR`, una instancia por carpeta). Se mantiene abierto mientras viva el proceso.
fn instancia_unica() -> Option<std::fs::File> {
    let dir = config::dir_app();
    std::fs::create_dir_all(&dir).ok()?;
    let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(dir.join("tray.lock")).ok()?;
    f.try_lock_exclusive().ok()?;
    Some(f)
}

/// Icono redondo de 32x32 del color del estado, con una flecha blanca hacia abajo.
fn icono_de(color: Color) -> Icon {
    const N: u32 = 32;
    let (r, g, b) = match color {
        Color::Gris => (0x8a, 0x8f, 0x98),
        Color::Azul => (0x2f, 0x80, 0xed),
        Color::Naranja => (0xf2, 0x99, 0x4a),
        Color::Rojo => (0xeb, 0x57, 0x57),
    };
    let mut px = vec![0u8; (N * N * 4) as usize];
    let c = (N as f32 - 1.0) / 2.0;
    for y in 0..N {
        for x in 0..N {
            let (dx, dy) = (x as f32 - c, y as f32 - c);
            let d = (dx * dx + dy * dy).sqrt();
            let i = ((y * N + x) * 4) as usize;
            if d <= 15.0 {
                // Flecha: asta vertical y punta triangular.
                let asta = dx.abs() <= 2.5 && (-9.0..=2.0).contains(&dy);
                let punta = (0.0..=8.0).contains(&(dy - 1.0)) && dx.abs() <= 8.0 - (dy - 1.0);
                let blanco = asta || punta;
                let a = if d > 14.0 { ((15.0 - d) * 255.0) as u8 } else { 255 };
                let rgb = if blanco { (255, 255, 255) } else { (r, g, b) };
                px[i..i + 4].copy_from_slice(&[rgb.0, rgb.1, rgb.2, a]);
            }
        }
    }
    Icon::from_rgba(px, N, N).expect("icono válido")
}

fn abrir_carpeta() {
    if let Some(d) = Config::cargar().ok().and_then(|c| c.destino) {
        let _ = std::process::Command::new("xdg-open").arg(d).spawn();
    }
}

fn main() {
    let Some(_bloqueo) = instancia_unica() else {
        eprintln!("icdown-tray ya está en marcha");
        return;
    };
    let mut cfg = Config::cargar().unwrap_or_default();
    if cfg.destino.is_none() {
        if let Some(p) = rfd::FileDialog::new().set_title(t(T::DlgDestino)).pick_folder() {
            cfg.destino = Some(p);
            let _ = cfg.guardar();
        }
    }

    let event_loop = EventLoopBuilder::<Ev>::with_user_event().build();
    let proxy_ui = Mutex::new(event_loop.create_proxy());
    let proxy_menu = Mutex::new(event_loop.create_proxy());
    MenuEvent::set_event_handler(Some(move |e| {
        let _ = proxy_menu.lock().unwrap().send_event(Ev::Menu(e));
    }));

    let (tx, rx) = mpsc::channel::<Orden>();
    let cancelar = Arc::new(AtomicBool::new(false));
    let control = Arc::new(ControlBorrado::default());
    let motor = hilo_motor::lanzar_con(
        rx,
        cancelar.clone(),
        control,
        Box::new(|| Config::cargar().unwrap_or_default()),
        Box::new(|cfg: &Config| {
            let id = cfg.apple_id.as_deref().context("sin Apple ID")?;
            Ok(Box::new(Puente::iniciar(id, &config::dir_sesion(), &config::log_puente())?) as Box<dyn FuenteIcloud>)
        }),
        Box::new(move |e| {
            let _ = proxy_ui.lock().unwrap().send_event(Ev::Ui(e));
        }),
    );
    let mut motor = Some(motor);

    // Sin Apple ID todavía: avisar de cómo iniciar sesión (el inicio de sesión interactivo es por CLI).
    let mut login_avisado = cfg.apple_id.is_none();
    if login_avisado {
        avisos::mostrar("icdown", t(T::AvLoginCmd));
    }

    let i_version = MenuItem::new(format!("icdown {}", env!("CARGO_PKG_VERSION")), false, None);
    let i_estado = MenuItem::new(t(T::MenuIniciando), false, None);
    let i_copiar = MenuItem::new(t(T::MenuCopiar), true, None);
    let i_pausa = MenuItem::new(t(T::MenuPausar), true, None);
    let i_verificar = MenuItem::new(t(T::MenuVerificar), true, None);
    let i_abrir = MenuItem::new(t(T::MenuAbrir), true, None);
    let i_destino = MenuItem::new(t(T::MenuDestino), true, None);
    let i_login = MenuItem::new(t(T::MenuSesion), true, None);
    let i_auto = CheckMenuItem::new(t(T::MenuAuto), true, autoarranque::activo(), None);
    let i_salir = MenuItem::new(t(T::MenuSalir), true, None);
    let menu = Menu::new();
    menu.append_items(&[
        &i_version,
        &PredefinedMenuItem::separator(),
        &i_estado,
        &PredefinedMenuItem::separator(),
        &i_copiar,
        &i_pausa,
        &i_verificar,
        &PredefinedMenuItem::separator(),
        &i_abrir,
        &i_destino,
        &i_login,
        &i_auto,
        &PredefinedMenuItem::separator(),
        &i_salir,
    ])
    .expect("menú");

    let mut tray: Option<TrayIcon> = None;
    let mut estado = EstadoUi::sin_datos(Fase::Inactiva);
    let mut pausado = false;
    // Para no inundar de notificaciones: una al empezar a copiar y otra por cada 10 % de avance.
    let mut ultimo_decil: Option<u64> = None;
    let mut ultimo_aviso = Instant::now() - Duration::from_secs(3600);

    event_loop.run(move |event, _destino_ev, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::NewEvents(StartCause::Init) => {
                tray = Some(
                    TrayIconBuilder::new()
                        .with_menu(Box::new(menu.clone()))
                        .with_tooltip("icdown")
                        .with_title("icdown")
                        .with_icon(icono_de(Color::Gris))
                        .build()
                        .expect("icono de bandeja"),
                );
            }
            Event::UserEvent(Ev::Ui(EventoUi::Estado(e))) => {
                let texto = e.texto();
                i_estado.set_text(&texto);
                if let Some(tr) = &tray {
                    let _ = tr.set_tooltip(Some(format!("icdown — {texto}")));
                    if e.color() != estado.color() {
                        let _ = tr.set_icon(Some(icono_de(e.color())));
                    }
                }
                match e.fase {
                    Fase::SesionCaducada if !login_avisado => {
                        login_avisado = true;
                        avisos::mostrar("icdown", t(T::AvLoginCmd));
                    }
                    Fase::Listando { .. } | Fase::Copiando { .. } | Fase::Verificando { .. } => login_avisado = false,
                    _ => {}
                }
                // Progreso por notificaciones (además del tooltip y del texto del menú).
                if matches!(e.fase, Fase::Copiando { .. } | Fase::Verificando { .. }) {
                    if let Some(p) = e.porcentaje() {
                        let decil = p / 10;
                        if ultimo_decil != Some(decil) && ultimo_aviso.elapsed() >= Duration::from_secs(60) {
                            ultimo_decil = Some(decil);
                            ultimo_aviso = Instant::now();
                            avisos::mostrar("icdown", &texto);
                        }
                    }
                } else if !e.trabajando() {
                    ultimo_decil = None;
                }
                estado = e;
            }
            Event::UserEvent(Ev::Ui(EventoUi::Aviso { titulo, texto })) => avisos::mostrar(&titulo, &texto),
            Event::UserEvent(Ev::Ui(EventoUi::Liberar(_))) => {}
            Event::UserEvent(Ev::Menu(ev)) => {
                let id = ev.id;
                if id == *i_copiar.id() {
                    pausado = false;
                    i_pausa.set_text(t(T::MenuPausar));
                    let _ = tx.send(Orden::CopiarAhora);
                } else if id == *i_pausa.id() {
                    if pausado {
                        pausado = false;
                        i_pausa.set_text(t(T::MenuPausar));
                        let _ = tx.send(Orden::Reanudar);
                    } else {
                        pausado = true;
                        i_pausa.set_text(t(T::MenuReanudar));
                        let _ = tx.send(Orden::Pausar);
                        cancelar.store(true, Ordering::SeqCst);
                    }
                } else if id == *i_verificar.id() {
                    let _ = tx.send(Orden::Verificar);
                } else if id == *i_login.id() {
                    avisos::mostrar("icdown", t(T::AvLoginCmd));
                } else if id == *i_abrir.id() {
                    abrir_carpeta();
                } else if id == *i_destino.id() {
                    if let Some(p) = rfd::FileDialog::new().set_title(t(T::DlgNuevoDestino)).pick_folder() {
                        let mut c = Config::cargar().unwrap_or_default();
                        c.destino = Some(p);
                        let _ = c.guardar();
                        cancelar.store(true, Ordering::SeqCst);
                        let _ = tx.send(Orden::CopiarAhora);
                    }
                } else if id == *i_auto.id() {
                    if autoarranque::fijar(i_auto.is_checked()).is_err() {
                        // No se pudo: deja la casilla como estaba realmente.
                        i_auto.set_checked(autoarranque::activo());
                    }
                } else if id == *i_salir.id() {
                    cancelar.store(true, Ordering::SeqCst);
                    let _ = tx.send(Orden::Salir);
                    tray.take();
                    // Espera breve a que el motor suelte el puente (se cierra al soltarlo).
                    if let Some(h) = motor.take() {
                        let limite = Instant::now() + Duration::from_secs(5);
                        while !h.is_finished() && Instant::now() < limite {
                            std::thread::sleep(Duration::from_millis(50));
                        }
                    }
                    *control_flow = ControlFlow::Exit;
                }
            }
            _ => {}
        }
    });
}
