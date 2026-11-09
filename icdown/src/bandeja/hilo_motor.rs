//! Hilo que ejecuta rondas periódicas y obedece órdenes del menú.
use crate::auditoria;
use crate::bandeja::estado::{EstadoUi, EventoUi, Fase, Ronda};
use crate::bandeja::hilo_liberar::{self, PlanPendiente};
use crate::criterio::Criterio;
use crate::i18n::{miles, t, tf, T};
use crate::liberar::ControlBorrado;
use crate::bitacora;
use crate::cerrojo;
use crate::config::Config;
use crate::descarga;
use crate::energia::SinSuspension;
use crate::fuente::FuenteIcloud;
use crate::motor::{self, Avance, FinRonda, OpcionesRonda};
use crate::planificador;
use crate::registro::Registro;
use reqwest::blocking::Client;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub enum Orden {
    CopiarAhora,
    Pausar,
    Reanudar,
    Verificar,
    Salir,
    /// Listar iCloud y re-verificar lo que se liberaría (el plan queda esperando confirmación).
    LiberarRevisar(Criterio),
    /// Borrar el plan revisado (pausa y cancelación con el `ControlBorrado` compartido).
    LiberarEjecutar,
    /// Olvidar el plan sin borrar nada.
    LiberarDescartar,
    /// Restaurar lo borrado en una liberación.
    LiberarRestaurar(String),
}

impl Orden {
    fn de_liberar(&self) -> bool {
        matches!(self, Orden::LiberarRevisar(_) | Orden::LiberarEjecutar | Orden::LiberarDescartar | Orden::LiberarRestaurar(_))
    }
}

pub type CargarConfig = Box<dyn Fn() -> Config + Send>;
pub type AbrirFuente = Box<dyn Fn(&Config) -> anyhow::Result<Box<dyn FuenteIcloud>> + Send>;
pub type Emitir = Box<dyn Fn(EventoUi) + Send + Sync>;

pub fn lanzar(rx: Receiver<Orden>, cancelar: Arc<AtomicBool>, cargar_config: CargarConfig, abrir_fuente: AbrirFuente, emitir: Emitir) -> JoinHandle<()> {
    lanzar_con(rx, cancelar, Arc::new(ControlBorrado::default()), cargar_config, abrir_fuente, emitir)
}

/// Como `lanzar`, con el control (pausa/cancelación) de los borrados que haga el motor.
pub fn lanzar_con(
    rx: Receiver<Orden>,
    cancelar: Arc<AtomicBool>,
    control: Arc<ControlBorrado>,
    cargar_config: CargarConfig,
    abrir_fuente: AbrirFuente,
    emitir: Emitir,
) -> JoinHandle<()> {
    std::thread::spawn(move || bucle(rx, &cancelar, &control, &*cargar_config, &*abrir_fuente, &*emitir))
}

fn aviso(emitir: &(dyn Fn(EventoUi) + Sync), texto: impl Into<String>) {
    emitir(EventoUi::Aviso { titulo: "icdown".into(), texto: texto.into() });
}

fn abrir_registro(destino: &Path) -> anyhow::Result<Registro> {
    Registro::abrir(&destino.join(".icdown").join("registro.db"))
}

fn emitir_resumen(cfg: &Config, fase: Fase, emitir: &(dyn Fn(EventoUi) + Sync)) {
    let progreso = cfg
        .destino
        .as_deref()
        .filter(|d| d.is_dir())
        .and_then(|d| abrir_registro(d).ok())
        .and_then(|r| r.progreso().ok())
        .unwrap_or_default();
    emitir(EventoUi::Estado(EstadoUi::con_progreso(fase, &progreso)));
}

fn bucle(
    rx: Receiver<Orden>,
    cancelar: &AtomicBool,
    control: &ControlBorrado,
    cargar_config: &dyn Fn() -> Config,
    abrir_fuente: &dyn Fn(&Config) -> anyhow::Result<Box<dyn FuenteIcloud>>,
    emitir: &(dyn Fn(EventoUi) + Sync),
) {
    let http = descarga::cliente();
    let mut pausado = false;
    let mut espera: Option<Duration> = Some(Duration::ZERO);
    let mut ultimo_fin: Option<FinRonda> = None;
    // La ronda anterior aplazó archivos por enlaces caducados: el próximo listado los renueva.
    let mut renovar = false;
    // Plan de liberar esperando confirmación, y la espera de rondas que había antes de él.
    let mut plan: Option<PlanPendiente> = None;
    let mut espera_normal: Option<Option<Duration>> = None;
    // Orden de liberar llegada mientras se vaciaba la cola antes de una ronda: se atiende primero.
    let mut diferida: Option<Orden> = None;
    emitir_resumen(&cargar_config(), Fase::Inactiva, emitir);
    loop {
        let orden = if let Some(o) = diferida.take() {
            Some(o)
        } else {
            match espera {
                Some(d) => match rx.recv_timeout(d) {
                    Ok(o) => Some(o),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                },
                None => match rx.recv() {
                    Ok(o) => Some(o),
                    Err(_) => return,
                },
            }
        };
        let mut forzar = false;
        match orden {
            Some(Orden::Salir) => {
                if let Some(p) = plan.take() {
                    p.descartar();
                }
                return;
            }
            Some(Orden::LiberarRevisar(c)) => {
                if let Some(p) = plan.take() {
                    p.descartar();
                }
                emitir_resumen(&cargar_config(), Fase::Inactiva, emitir);
                plan = hilo_liberar::revisar(&cargar_config(), cancelar, abrir_fuente, emitir, &c);
                if plan.is_some() {
                    espera_normal.get_or_insert(espera);
                    espera = Some(hilo_liberar::LATIDO);
                }
                continue;
            }
            Some(Orden::LiberarEjecutar) => {
                if let Some(p) = plan.take() {
                    control.pausa.store(false, Ordering::SeqCst);
                    control.cancelar.store(false, Ordering::SeqCst);
                    hilo_liberar::ejecutar(p, control, emitir);
                    emitir_resumen(&cargar_config(), Fase::Inactiva, emitir);
                }
                espera = espera_normal.take().unwrap_or(espera);
                continue;
            }
            Some(Orden::LiberarDescartar) => {
                if let Some(p) = plan.take() {
                    p.descartar();
                }
                espera = espera_normal.take().unwrap_or(espera);
                continue;
            }
            Some(Orden::LiberarRestaurar(sesion)) => {
                hilo_liberar::restaurar(&cargar_config(), abrir_fuente, emitir, &sesion);
                emitir_resumen(&cargar_config(), Fase::Inactiva, emitir);
                continue;
            }
            // Esperando confirmación: no hay rondas; se renueva el avance (o caduca el plan).
            None if plan.is_some() => {
                if !plan.as_ref().is_some_and(|p| p.latir()) {
                    plan = None;
                    espera = espera_normal.take().unwrap_or(espera);
                }
                continue;
            }
            Some(Orden::Pausar) => {
                pausado = true;
                espera = None;
                emitir_resumen(&cargar_config(), Fase::Pausada, emitir);
                continue;
            }
            Some(Orden::Reanudar) | Some(Orden::CopiarAhora) => {
                pausado = false;
                forzar = true;
            }
            Some(Orden::Verificar) => {
                verificar(&cargar_config(), cancelar, emitir);
                continue;
            }
            None => {}
        }
        if pausado {
            espera = None;
            continue;
        }
        // Una orden llegada justo antes de borrar `cancelar` (p. ej. Pausar) no debe perderse:
        // tras borrarlo se atiende lo que haya en la cola, hasta que esté vacía.
        loop {
            cancelar.store(false, Ordering::SeqCst);
            let mut hubo = false;
            loop {
                match rx.try_recv() {
                    Ok(Orden::Salir) | Err(TryRecvError::Disconnected) => return,
                    Ok(Orden::Pausar) => pausado = true,
                    Ok(Orden::Reanudar) | Ok(Orden::CopiarAhora) => {
                        pausado = false;
                        forzar = true;
                    }
                    Ok(Orden::Verificar) => verificar(&cargar_config(), cancelar, emitir),
                    Ok(o) if o.de_liberar() => {
                        diferida = Some(o);
                        break;
                    }
                    Ok(_) => {}
                    Err(TryRecvError::Empty) => break,
                }
                hubo = true;
            }
            if !hubo || pausado || diferida.is_some() {
                break;
            }
        }
        if diferida.is_some() {
            continue;
        }
        if pausado {
            espera = None;
            emitir_resumen(&cargar_config(), Fase::Pausada, emitir);
            continue;
        }
        espera = una_ronda(&cargar_config(), &http, cancelar, abrir_fuente, emitir, forzar, &mut ultimo_fin, &mut renovar);
    }
}

#[allow(clippy::too_many_arguments)]
fn una_ronda(
    cfg: &Config,
    http: &Client,
    cancelar: &AtomicBool,
    abrir_fuente: &dyn Fn(&Config) -> anyhow::Result<Box<dyn FuenteIcloud>>,
    emitir: &(dyn Fn(EventoUi) + Sync),
    forzar: bool,
    ultimo_fin: &mut Option<FinRonda>,
    renovar: &mut bool,
) -> Option<Duration> {
    let Some(destino) = cfg.destino.clone() else {
        emitir(EventoUi::Estado(EstadoUi::sin_datos(Fase::SinDestino)));
        return None;
    };
    if cfg.apple_id.is_none() {
        emitir(EventoUi::Estado(EstadoUi::sin_datos(Fase::SesionCaducada)));
        return None;
    }
    if !destino.is_dir() {
        emitir(EventoUi::Estado(EstadoUi::sin_datos(Fase::DestinoAusente)));
        return Some(Duration::from_secs(600));
    }
    let dir_datos = destino.join(".icdown");
    let _cerrojo = match cerrojo::intentar(&dir_datos) {
        Ok(Some(c)) => c,
        Ok(None) => return Some(Duration::from_secs(60)), // otro proceso (p. ej. "liberar") trabaja
        Err(e) => {
            emitir(EventoUi::Estado(EstadoUi::sin_datos(Fase::Error(e.to_string()))));
            return Some(Duration::from_secs(600));
        }
    };
    let registro = match abrir_registro(&destino) {
        Ok(r) => Mutex::new(r),
        Err(e) => {
            emitir(EventoUi::Estado(EstadoUi::sin_datos(Fase::Error(e.to_string()))));
            return Some(Duration::from_secs(600));
        }
    };
    if forzar {
        let _ = registro.lock().unwrap().reintentar_ya();
    }
    let _energia = SinSuspension::activar();
    let mut fuente = match abrir_fuente(cfg) {
        Ok(f) => f,
        Err(e) => {
            bitacora::escribir(&dir_datos, &format!("ERROR abriendo el puente: {e:#}"));
            emitir(EventoUi::Estado(EstadoUi::sin_datos(Fase::Error(e.to_string()))));
            return Some(Duration::from_secs(900));
        }
    };
    // Como mucho 4 avisos por segundo a la interfaz y una consulta al registro por segundo.
    let inicio = Instant::now() - Duration::from_secs(10);
    let ritmo = Mutex::new((inicio, inicio, registro.lock().unwrap().progreso().unwrap_or_default(), None::<Ronda>));
    let renovando = std::mem::take(renovar);
    // Lo que trajo el último listado completo: total previsto mientras se vuelve a listar.
    let previsto = registro.lock().unwrap().assets_ultimo_listado().ok().flatten();
    let avance = |a: Avance| {
        let (fase, actual, ultimo) = match a {
            Avance::Listando { vistos } => (Fase::Listando { vistos }, None, false),
            Avance::Copiando { hechas, total, actual } => (Fase::Copiando { hechas, total }, Some(actual), hechas == total),
            // Breve, sin relistar: se muestra siempre (no se descarta por el límite de avisos por segundo).
            Avance::RenovandoEnlaces { n, hechas, total } => (Fase::Copiando { hechas, total }, Some(tf(T::AvRenovando, &[("n", &n)])), true),
        };
        let mut r = ritmo.lock().unwrap();
        if let (Fase::Copiando { total, .. }, None) = (&fase, &r.3) {
            // Primer archivo de la ronda: se mide su trabajo para mostrar su propio avance.
            let reg = registro.lock().unwrap();
            let bytes = reg.partes_pendientes(motor::ahora(), None).map(|v| v.iter().map(|t| t.tamano).sum()).unwrap_or(0);
            if let Ok(p) = reg.progreso() {
                r.2 = p;
            }
            r.3 = Some(Ronda { archivos: *total as u64, bytes, bytes_base: r.2.bytes_verificados });
        }
        let ahora = Instant::now();
        if !ultimo && ahora.duration_since(r.0) < Duration::from_millis(250) {
            return;
        }
        r.0 = ahora;
        if ahora.duration_since(r.1) >= Duration::from_secs(1) {
            r.1 = ahora;
            if let Ok(p) = registro.lock().unwrap().progreso() {
                r.2 = p;
            }
        }
        let mut e = EstadoUi::con_progreso(fase, &r.2);
        e.detalle.actual = actual;
        e.detalle.ronda = r.3.clone();
        e.detalle.renovando_enlaces = renovando && matches!(e.fase, Fase::Listando { .. });
        if matches!(e.fase, Fase::Listando { .. }) {
            e.detalle.listado_previsto = previsto;
        }
        drop(r);
        emitir(EventoUi::Estado(e));
    };
    let op = OpcionesRonda { concurrencia: cfg.concurrencia, mes: None };
    let r = motor::ejecutar_ronda(&destino, &registro, fuente.as_mut(), http, &op, cancelar, &avance);
    drop(fuente);
    let reg = registro.lock().unwrap();
    let progreso = reg.progreso().unwrap_or_default();
    let prox = reg.proximo_reintento(None).ok().flatten();
    // El estado final lleva la hora prevista de la próxima ronda.
    let estado = |fase: Fase, espera: Option<Duration>| {
        let mut e = EstadoUi::con_progreso(fase, &progreso);
        e.detalle.proxima = espera.map(|d| motor::ahora() + d.as_secs() as i64);
        EventoUi::Estado(e)
    };
    match r {
        Err(e) => {
            bitacora::escribir(&dir_datos, &format!("ERROR en la ronda: {e:#}"));
            let espera = Some(Duration::from_secs(900));
            emitir(estado(Fase::Error(e.to_string()), espera));
            espera
        }
        Ok(r) => {
            bitacora::escribir(&dir_datos, &format!("RONDA {:?}: {} verificadas, {} fallidas, {} aplazadas", r.fin, r.verificadas, r.fallidas, r.aplazadas));
            // Solo si se aplazaron archivos para relistar (no los que ya no estaban al renovar enlaces).
            *renovar = r.fin == FinRonda::Relistar && r.aplazadas > 0;
            if r.verificadas > 0 {
                aviso(emitir, tf(T::AvCopiados, &[("n", &miles(r.verificadas as u64))]));
            }
            let nuevo = *ultimo_fin != Some(r.fin);
            *ultimo_fin = Some(r.fin);
            let fase = match r.fin {
                FinRonda::Completa | FinRonda::Relistar => Fase::Inactiva,
                FinRonda::Cancelada => Fase::Pausada,
                FinRonda::SesionCaducada => {
                    if nuevo {
                        aviso(emitir, t(T::AvSesion));
                    }
                    Fase::SesionCaducada
                }
                FinRonda::SinEspacio => {
                    if nuevo {
                        aviso(emitir, tf(T::AvSinEspacio, &[("d", &destino.display())]));
                    }
                    Fase::SinEspacio
                }
                FinRonda::DestinoAusente => Fase::DestinoAusente,
            };
            let espera = planificador::espera_tras_ronda(&r, prox, motor::ahora(), cfg.intervalo_horas);
            emitir(estado(fase, espera));
            espera
        }
    }
}

fn verificar(cfg: &Config, cancelar: &AtomicBool, emitir: &(dyn Fn(EventoUi) + Sync)) {
    let Some(destino) = cfg.destino.clone() else { return };
    let dir_datos = destino.join(".icdown");
    let Ok(Some(_cerrojo)) = cerrojo::intentar(&dir_datos) else {
        aviso(emitir, t(T::AvCopiaEnMarcha));
        return;
    };
    let Ok(reg) = abrir_registro(&destino) else { return };
    let reg = Mutex::new(reg);
    cancelar.store(false, Ordering::SeqCst);
    let r = auditoria::verificar_todo(&destino, &reg, cancelar, &|h, t| {
        if h % 50 == 0 || h == t {
            emitir(EventoUi::Estado(EstadoUi::sin_datos(Fase::Verificando { hechas: h, total: t })));
        }
    });
    match r {
        Ok(res) if res.malas.is_empty() => aviso(emitir, tf(T::AvVerificado, &[("n", &miles(res.revisadas as u64))])),
        Ok(res) => {
            for (ruta, m) in &res.malas {
                bitacora::escribir(&dir_datos, &format!("DAÑADO {ruta}: {m}"));
            }
            aviso(emitir, tf(T::AvDanados, &[("n", &miles(res.malas.len() as u64))]));
        }
        Err(e) => aviso(emitir, tf(T::AvNoVerificado, &[("e", &e)])),
    }
    drop(reg);
    emitir_resumen(cfg, Fase::Inactiva, emitir);
}
