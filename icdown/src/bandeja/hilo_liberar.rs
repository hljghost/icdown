//! Liberar espacio desde la bandeja, en el hilo del motor: revisar (listar iCloud y re-verificar),
//! borrar por lotes (con pausa y cancelación) y restaurar. Mientras tanto no hay rondas de copia.
use crate::bandeja::asistente::{EventoLiberar, ResumenFinal, ResumenPlan};
use crate::bandeja::estado::EventoUi;
use crate::bitacora;
use crate::cerrojo;
use crate::config::Config;
use crate::criterio::Criterio;
use crate::energia::SinSuspension;
use crate::fuente::FuenteIcloud;
use crate::i18n::{miles, t, tf, T};
use crate::liberar::{self, ControlBorrado, PlanLiberar};
use crate::motor::{self, Avance, FinRonda, Listado};
use crate::progreso_liberar::{self, FaseLiberar, Seguimiento};
use crate::registro::Registro;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Un plan listo esperando confirmación. Guarda el puente que listó: solo él puede borrar esos ids.
pub struct PlanPendiente {
    destino: PathBuf,
    registro: Mutex<Registro>,
    fuente: Box<dyn FuenteIcloud>,
    plan: PlanLiberar,
    desde: Instant,
}

/// Cada cuánto se renueva el avance mientras se espera la confirmación.
pub const LATIDO: Duration = Duration::from_secs(20);
/// Un plan sin confirmar caduca (iCloud o el disco pueden haber cambiado).
pub const CADUCIDAD_PLAN: Duration = Duration::from_secs(30 * 60);

impl PlanPendiente {
    /// Renueva el avance; `false` si el plan caducó (y se descarta).
    pub fn latir(&self) -> bool {
        if self.desde.elapsed() >= CADUCIDAD_PLAN {
            let _ = self.registro.lock().unwrap().borrar_meta(progreso_liberar::CLAVE);
            return false;
        }
        progreso_liberar::latir(&self.registro);
        true
    }

    pub fn descartar(self) {
        let _ = self.registro.lock().unwrap().borrar_meta(progreso_liberar::CLAVE);
    }
}

fn emitir_liberar(emitir: &(dyn Fn(EventoUi) + Sync), e: EventoLiberar) {
    emitir(EventoUi::Liberar(e));
}

fn aviso(emitir: &(dyn Fn(EventoUi) + Sync), texto: String) {
    emitir(EventoUi::Aviso { titulo: "icdown".into(), texto });
}

fn abrir_registro(destino: &std::path::Path) -> anyhow::Result<Registro> {
    Registro::abrir(&destino.join(".icdown").join("registro.db"))
}

/// Lista iCloud y re-verifica lo que se liberaría con `criterio`. Avisa con `Plan` o `RevisionFallida`.
pub fn revisar(
    cfg: &Config,
    cancelar: &AtomicBool,
    abrir_fuente: &dyn Fn(&Config) -> anyhow::Result<Box<dyn FuenteIcloud>>,
    emitir: &(dyn Fn(EventoUi) + Sync),
    criterio: &Criterio,
) -> Option<PlanPendiente> {
    cancelar.store(false, Ordering::SeqCst);
    let fallo = |m: String| {
        emitir_liberar(emitir, EventoLiberar::RevisionFallida(m));
        None
    };
    let Some(destino) = cfg.destino.clone() else { return fallo(t(T::TitElige).into()) };
    let dir_datos = destino.join(".icdown");
    let Ok(Some(_cerrojo)) = cerrojo::intentar(&dir_datos) else { return fallo(t(T::AsOcupado).into()) };
    let registro = match abrir_registro(&destino) {
        Ok(r) => Mutex::new(r),
        Err(e) => return fallo(format!("{e:#}")),
    };
    let _energia = SinSuspension::activar();
    let previsto = registro.lock().unwrap().assets_ultimo_listado().ok().flatten().unwrap_or(0);
    let mut seg = Seguimiento::empezar(&registro, criterio.dias());
    seg.fase(FaseLiberar::Listando, previsto, 0);
    let mut fuente = match abrir_fuente(cfg) {
        Ok(f) => f,
        Err(e) => {
            seg.fallar(&format!("{e:#}"));
            return fallo(format!("{e:#}"));
        }
    };
    let seg = Mutex::new(seg);
    let avance = |a: Avance| {
        if let Avance::Listando { vistos } = a {
            seg.lock().unwrap().avance(vistos as u64, 0);
        }
    };
    let listado = motor::listar_en_registro(&registro, fuente.as_mut(), cancelar, &avance);
    let mut seg = seg.into_inner().unwrap();
    match listado {
        Ok(Listado::Completo(_)) => {}
        Ok(Listado::Interrumpido(FinRonda::Cancelada, _)) => {
            seg.descartar();
            return fallo(t(T::AsCancelar).into());
        }
        Ok(Listado::Interrumpido(FinRonda::SesionCaducada, _)) => {
            seg.fallar(t(T::EstSesion));
            return fallo(t(T::EstSesion).into());
        }
        Ok(Listado::Interrumpido(fin, _)) => {
            let m = format!("{fin:?}");
            seg.fallar(&m);
            return fallo(m);
        }
        Err(e) => {
            seg.fallar(&format!("{e:#}"));
            return fallo(format!("{e:#}"));
        }
    }
    drop(seg);
    let plan = match liberar::preparar_cancelable(&destino, &registro, criterio, motor::ahora(), &|_, _| {}, cancelar) {
        Ok(p) => p,
        Err(e) => return fallo(format!("{e:#}")),
    };
    let lista = if plan.listos.is_empty() { None } else { liberar::escribir_plan_csv(&destino, &plan).ok() };
    let fecha = |c: &crate::registro::Candidato| crate::layout::fecha(&c.fecha).ok().map(|f| f.timestamp());
    let resumen = ResumenPlan {
        elementos: plan.listos.len() as u64,
        bytes: plan.bytes,
        desde: plan.listos.first().and_then(fecha),
        hasta: plan.listos.last().and_then(fecha),
        reverificados: (plan.listos.len() + plan.excluidos.len()) as u64,
        excluidos_reverificacion: plan.excluidos.len() as u64,
        no_verificados: plan.no_verificados,
        no_liberables: plan.no_liberables,
        atipicos: plan.atipicos,
        lista,
    };
    bitacora::escribir(&dir_datos, &format!("LIBERAR revisado {criterio:?}: {} listos, {} bytes, {} excluidos", resumen.elementos, resumen.bytes, resumen.excluidos_reverificacion));
    for (c, m) in &plan.excluidos {
        bitacora::escribir(&dir_datos, &format!("LIBERAR excluido {} ({}): {m}", c.nombre_base, c.asset_id));
    }
    emitir_liberar(emitir, EventoLiberar::Plan(resumen));
    (!plan.listos.is_empty()).then(|| PlanPendiente { destino, registro, fuente, plan, desde: Instant::now() })
}

/// Borra el plan confirmado. `control` permite pausar o cancelar desde la interfaz.
pub fn ejecutar(mut p: PlanPendiente, control: &ControlBorrado, emitir: &(dyn Fn(EventoUi) + Sync)) {
    let dir_datos = p.destino.join(".icdown");
    let Ok(Some(_cerrojo)) = cerrojo::intentar(&dir_datos) else {
        p.descartar();
        emitir_liberar(emitir, EventoLiberar::Fallido(t(T::AsOcupado).into()));
        return;
    };
    let _energia = SinSuspension::activar();
    match liberar::ejecutar_con(&p.destino, &p.registro, p.fuente.as_mut(), &p.plan.listos, &|_, _| {}, control) {
        Ok(r) => {
            bitacora::escribir(&dir_datos, &format!("LIBERAR sesión {}: {} borrados, {} fallidos, cancelado={}", r.sesion, r.borrados, r.fallidos.len(), r.cancelado));
            for (id, e) in r.errores.iter().chain(&r.fallidos) {
                bitacora::escribir(&dir_datos, &format!("LIBERAR {id}: {e}"));
            }
            if r.borrados > 0 {
                aviso(emitir, tf(T::AvLiberado, &[("b", &crate::bandeja::panel::bytes_dec(r.bytes)), ("n", &miles(r.borrados as u64))]));
            }
            emitir_liberar(
                emitir,
                EventoLiberar::Terminado(ResumenFinal {
                    borrados: r.borrados as u64,
                    bytes: r.bytes,
                    fallidos: r.fallidos.len() as u64,
                    cancelado: r.cancelado,
                    sesion: r.sesion,
                    csv: Some(dir_datos.join("borrados.csv")),
                }),
            );
        }
        Err(e) => {
            bitacora::escribir(&dir_datos, &format!("LIBERAR error: {e:#}"));
            aviso(emitir, tf(T::AvLiberarFallo, &[("e", &format!("{e:#}"))]));
            emitir_liberar(emitir, EventoLiberar::Fallido(format!("{e:#}")));
        }
    }
}

/// Saca de "Eliminado recientemente" lo de una liberación.
pub fn restaurar(
    cfg: &Config,
    abrir_fuente: &dyn Fn(&Config) -> anyhow::Result<Box<dyn FuenteIcloud>>,
    emitir: &(dyn Fn(EventoUi) + Sync),
    sesion: &str,
) {
    let fallo = |m: String| emitir_liberar(emitir, EventoLiberar::RestaurarFallido(m));
    let Some(destino) = cfg.destino.clone() else { return fallo(t(T::TitElige).into()) };
    let dir_datos = destino.join(".icdown");
    let Ok(Some(_cerrojo)) = cerrojo::intentar(&dir_datos) else { return fallo(t(T::AsOcupado).into()) };
    let registro = match abrir_registro(&destino) {
        Ok(r) => Mutex::new(r),
        Err(e) => return fallo(format!("{e:#}")),
    };
    let mut fuente = match abrir_fuente(cfg) {
        Ok(f) => f,
        Err(e) => return fallo(format!("{e:#}")),
    };
    match liberar::restaurar_sesion(&destino, &registro, fuente.as_mut(), sesion) {
        Ok(r) => {
            bitacora::escribir(&dir_datos, &format!("RESTAURAR sesión {sesion}: {} restaurados, {} fallidos", r.restaurados.len(), r.fallidos.len()));
            aviso(emitir, tf(T::AvRestaurados, &[("n", &miles(r.restaurados.len() as u64))]));
            emitir_liberar(emitir, EventoLiberar::Restaurado { restaurados: r.restaurados.len() as u64, fallidos: r.fallidos.len() as u64 });
        }
        Err(e) => fallo(format!("{e:#}")),
    }
}
