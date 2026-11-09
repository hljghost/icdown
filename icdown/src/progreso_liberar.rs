//! Avance de "liberar espacio" guardado en el registro (tabla `meta`, clave `liberar_progreso`),
//! para que la bandeja lo muestre aunque el borrado lo haga otro proceso (`icdown liberar`).
use crate::registro::Registro;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Instant;

pub const CLAVE: &str = "liberar_progreso";
/// Sin noticias durante este tiempo, un liberar en marcha se da por interrumpido.
pub const CADUCA_SEG: i64 = 120;
/// El resumen final (o el aviso de interrupción) se muestra durante un día.
pub const RESUMEN_SEG: i64 = 24 * 3600;
/// Como mucho dos escrituras por segundo (más los cambios de fase y el final).
const CADA_MS: u128 = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FaseLiberar {
    Listando,
    Reverificando,
    /// Plan listo: esperando a que la persona confirme (escriba BORRAR).
    Confirmando,
    Borrando,
    Terminado,
    Fallido,
}

impl FaseLiberar {
    pub fn terminal(self) -> bool {
        matches!(self, FaseLiberar::Terminado | FaseLiberar::Fallido)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProgresoLiberar {
    pub fase: FaseLiberar,
    pub hechos: u64,
    pub total: u64,
    pub bytes_hechos: u64,
    pub bytes_total: u64,
    pub inicio_unix: i64,
    pub actualizado_unix: i64,
    pub borrados: u64,
    pub fallidos: u64,
    pub dias: u32,
    /// Elementos por segundo en la fase actual (ventana móvil), si ya se pudo medir.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub por_seg: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Borrado en pausa (a petición): sigue vivo, pero no avanza.
    #[serde(default)]
    pub en_pausa: bool,
    /// Terminó porque se canceló (tras el lote en curso).
    #[serde(default)]
    pub cancelado: bool,
    /// Identificador de la liberación (para restaurarla).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sesion: Option<String>,
    /// Apple pidió esperar: se reanuda a esta hora (unix).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub espera_hasta: Option<i64>,
}

/// Cómo debe mostrarse ahora un avance guardado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Situacion {
    EnCurso,
    Confirmando,
    Terminado,
    Fallido,
    /// En marcha según el registro, pero sin noticias hace más de `CADUCA_SEG` (proceso cerrado o caído).
    Interrumpido,
}

impl ProgresoLiberar {
    pub fn nuevo(dias: u32, ahora: i64) -> ProgresoLiberar {
        ProgresoLiberar {
            fase: FaseLiberar::Listando,
            hechos: 0,
            total: 0,
            bytes_hechos: 0,
            bytes_total: 0,
            inicio_unix: ahora,
            actualizado_unix: ahora,
            borrados: 0,
            fallidos: 0,
            dias,
            por_seg: None,
            error: None,
            en_pausa: false,
            cancelado: false,
            sesion: None,
            espera_hasta: None,
        }
    }

    pub fn a_json(&self) -> String {
        serde_json::to_string(self).expect("JSON del avance")
    }

    /// `None` si el texto no es un avance válido (p. ej. de otra versión).
    pub fn de_json(s: &str) -> Option<ProgresoLiberar> {
        serde_json::from_str(s).ok()
    }

    /// `None` = no hay nada que mostrar (caducó el resumen, o se abandonó la confirmación).
    pub fn situacion(&self, ahora: i64) -> Option<Situacion> {
        let edad = ahora - self.actualizado_unix;
        match self.fase {
            FaseLiberar::Terminado if edad < RESUMEN_SEG => Some(Situacion::Terminado),
            FaseLiberar::Fallido if edad < RESUMEN_SEG => Some(Situacion::Fallido),
            FaseLiberar::Terminado | FaseLiberar::Fallido => None,
            // Sin borrar nada todavía: si la consola se cerró, no hay nada que contar.
            FaseLiberar::Confirmando if edad <= CADUCA_SEG => Some(Situacion::Confirmando),
            FaseLiberar::Confirmando => None,
            _ if edad <= CADUCA_SEG => Some(Situacion::EnCurso),
            _ if edad < RESUMEN_SEG => Some(Situacion::Interrumpido),
            _ => None,
        }
    }

    /// Segundos que faltan según el ritmo medido (`None` sin medida o sin total).
    pub fn restante_seg(&self) -> Option<u64> {
        let v = self.por_seg.filter(|v| *v > 0.0)?;
        (self.total > 0 && self.hechos < self.total).then(|| ((self.total - self.hechos) as f64 / v).round() as u64)
    }

    /// Fracción hecha (0–1); `None` si no se conoce el total.
    pub fn fraccion(&self) -> Option<f64> {
        (self.total > 0).then(|| (self.hechos as f64 / self.total as f64).clamp(0.0, 1.0))
    }
}

/// Lee el avance guardado (o `None` si no hay o no se entiende).
pub fn leer(reg: &Registro) -> Option<ProgresoLiberar> {
    reg.leer_meta(CLAVE).ok().flatten().and_then(|s| ProgresoLiberar::de_json(&s))
}

/// Ritmo (unidades por segundo) sobre una ventana móvil de muestras (segundos, contador).
#[derive(Debug, Default)]
pub struct Ritmo {
    muestras: VecDeque<(f64, u64)>,
}

impl Ritmo {
    pub const VENTANA: f64 = 60.0;
    /// Hace falta al menos este tramo medido para dar un ritmo.
    pub const MINIMO: f64 = 5.0;

    pub fn registrar(&mut self, t: f64, n: u64) {
        if self.muestras.back().is_some_and(|&(_, m)| n < m) {
            self.muestras.clear();
        }
        self.muestras.push_back((t, n));
        while self.muestras.len() > 2 && t - self.muestras[1].0 >= Self::VENTANA {
            self.muestras.pop_front();
        }
    }

    pub fn por_seg(&self) -> Option<f64> {
        let (&(t0, n0), &(t1, n1)) = (self.muestras.front()?, self.muestras.back()?);
        (t1 - t0 >= Self::MINIMO && n1 > n0).then(|| (n1 - n0) as f64 / (t1 - t0))
    }

    pub fn reiniciar(&mut self) {
        self.muestras.clear();
    }
}

fn ahora_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Anota en el registro el avance de un liberar. Un fallo al anotar nunca detiene el borrado:
/// solo deja la bandeja sin noticias.
pub struct Seguimiento<'a> {
    reg: &'a Mutex<Registro>,
    p: ProgresoLiberar,
    ultima: Option<Instant>,
    t0: Instant,
    ritmo: Ritmo,
}

impl<'a> Seguimiento<'a> {
    /// Empieza un liberar nuevo (fase "listando").
    pub fn empezar(reg: &'a Mutex<Registro>, dias: u32) -> Seguimiento<'a> {
        let mut s = Seguimiento { reg, p: ProgresoLiberar::nuevo(dias, ahora_unix()), ultima: None, t0: Instant::now(), ritmo: Ritmo::default() };
        s.escribir();
        s
    }

    /// Sigue el liberar en marcha (mismo inicio) o, si no hay uno vivo, empieza otro.
    pub fn continuar(reg: &'a Mutex<Registro>, dias: Option<u32>) -> Seguimiento<'a> {
        let ahora = ahora_unix();
        let previo = leer(&reg.lock().unwrap())
            .filter(|p| matches!(p.situacion(ahora), Some(Situacion::EnCurso | Situacion::Confirmando)));
        let mut p = match previo {
            Some(p) => ProgresoLiberar { dias: dias.unwrap_or(p.dias), ..ProgresoLiberar::nuevo(p.dias, p.inicio_unix) },
            None => ProgresoLiberar::nuevo(dias.unwrap_or(0), ahora),
        };
        p.actualizado_unix = ahora;
        Seguimiento { reg, p, ultima: None, t0: Instant::now(), ritmo: Ritmo::default() }
    }

    pub fn progreso(&self) -> &ProgresoLiberar {
        &self.p
    }

    /// Cambia de fase (siempre se anota).
    pub fn fase(&mut self, fase: FaseLiberar, total: u64, bytes_total: u64) {
        self.p.fase = fase;
        self.p.total = total;
        self.p.bytes_total = bytes_total;
        self.p.hechos = 0;
        self.p.bytes_hechos = 0;
        self.p.por_seg = None;
        self.ritmo.reiniciar();
        self.ritmo.registrar(self.t0.elapsed().as_secs_f64(), 0);
        self.escribir();
    }

    /// Avance dentro de la fase (se anota como mucho dos veces por segundo).
    pub fn avance(&mut self, hechos: u64, bytes_hechos: u64) {
        self.p.hechos = hechos;
        self.p.bytes_hechos = bytes_hechos;
        self.ritmo.registrar(self.t0.elapsed().as_secs_f64(), hechos);
        self.p.por_seg = self.ritmo.por_seg();
        if self.ultima.is_none_or(|t| t.elapsed().as_millis() >= CADA_MS) {
            self.escribir();
        }
    }

    pub fn contar(&mut self, borrados: u64, fallidos: u64) {
        self.p.borrados = borrados;
        self.p.fallidos = fallidos;
    }

    /// Fin del borrado (`cancelado`: se paró a petición antes de acabar).
    pub fn terminar(&mut self, cancelado: bool) {
        self.p.fase = FaseLiberar::Terminado;
        self.p.por_seg = None;
        self.p.en_pausa = false;
        self.p.cancelado = cancelado;
        self.escribir();
    }

    /// Entra o sale de la pausa (se anota siempre; en pausa conviene repetirlo como latido).
    pub fn pausa(&mut self, si: bool) {
        self.p.en_pausa = si;
        if si {
            self.p.por_seg = None;
            self.ritmo.reiniciar();
        }
        self.escribir();
    }

    /// Apple pidió esperar hasta `hasta` (unix); 0 = ya no se espera. Se anota siempre.
    pub fn esperar_apple(&mut self, hasta: i64) {
        self.p.espera_hasta = (hasta > 0).then_some(hasta);
        if hasta > 0 {
            self.p.por_seg = None;
            self.ritmo.reiniciar();
        }
        self.escribir();
    }

    /// Solo renueva la hora (latido durante una espera larga).
    pub fn latir(&mut self) {
        self.escribir();
    }

    pub fn sesion(&mut self, sesion: &str) {
        self.p.sesion = Some(sesion.to_string());
    }

    pub fn fallar(&mut self, motivo: &str) {
        self.p.fase = FaseLiberar::Fallido;
        self.p.por_seg = None;
        self.p.error = Some(motivo.chars().take(200).collect());
        self.escribir();
    }

    /// No se va a borrar nada (cancelado o nada que borrar): la bandeja no muestra nada.
    pub fn descartar(self) {
        let _ = self.reg.lock().unwrap().borrar_meta(CLAVE);
    }

    fn escribir(&mut self) {
        self.p.actualizado_unix = ahora_unix();
        self.ultima = Some(Instant::now());
        let _ = self.reg.lock().unwrap().guardar_meta(CLAVE, &self.p.a_json());
    }
}

/// Renueva la hora del liberar en marcha sin cambiar nada más (p. ej. mientras se espera la confirmación).
pub fn latir(reg: &Mutex<Registro>) {
    let mut r = reg.lock().unwrap();
    if let Some(mut p) = leer(&r).filter(|p| !p.fase.terminal()) {
        p.actualizado_unix = ahora_unix();
        let _ = r.guardar_meta(CLAVE, &p.a_json());
    }
}

/// Marca como fallido el liberar en marcha, si lo hay (p. ej. tras un error antes de borrar).
pub fn marcar_fallido(reg: &Mutex<Registro>, motivo: &str) {
    let Some(p) = leer(&reg.lock().unwrap()).filter(|p| !p.fase.terminal()) else { return };
    Seguimiento { reg, p, ultima: None, t0: Instant::now(), ritmo: Ritmo::default() }.fallar(motivo);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(fase: FaseLiberar, actualizado: i64) -> ProgresoLiberar {
        ProgresoLiberar { fase, actualizado_unix: actualizado, ..ProgresoLiberar::nuevo(30, 0) }
    }

    #[test]
    fn json_ida_y_vuelta_y_formato() {
        let mut a = p(FaseLiberar::Borrando, 100);
        a.hechos = 2350;
        a.total = 11029;
        a.por_seg = Some(2.5);
        let s = a.a_json();
        assert!(s.contains("\"fase\":\"borrando\""), "{s}");
        assert!(!s.contains("error"), "{s}");
        assert_eq!(ProgresoLiberar::de_json(&s), Some(a));
        assert_eq!(ProgresoLiberar::de_json("{basura"), None);
        // Campos opcionales ausentes (otra versión).
        let min = r#"{"fase":"terminado","hechos":1,"total":1,"bytes_hechos":5,"bytes_total":5,"inicio_unix":1,"actualizado_unix":2,"borrados":1,"fallidos":0,"dias":30}"#;
        assert_eq!(ProgresoLiberar::de_json(min).unwrap().fase, FaseLiberar::Terminado);
    }

    #[test]
    fn situacion_caducidad_e_interrupcion() {
        let ahora = 1_000_000;
        assert_eq!(p(FaseLiberar::Borrando, ahora - 10).situacion(ahora), Some(Situacion::EnCurso));
        assert_eq!(p(FaseLiberar::Borrando, ahora - CADUCA_SEG).situacion(ahora), Some(Situacion::EnCurso));
        assert_eq!(p(FaseLiberar::Borrando, ahora - CADUCA_SEG - 1).situacion(ahora), Some(Situacion::Interrumpido));
        assert_eq!(p(FaseLiberar::Listando, ahora - 600).situacion(ahora), Some(Situacion::Interrumpido));
        assert_eq!(p(FaseLiberar::Borrando, ahora - RESUMEN_SEG).situacion(ahora), None);
        assert_eq!(p(FaseLiberar::Confirmando, ahora - 30).situacion(ahora), Some(Situacion::Confirmando));
        assert_eq!(p(FaseLiberar::Confirmando, ahora - 600).situacion(ahora), None, "consola cerrada sin confirmar");
        assert_eq!(p(FaseLiberar::Terminado, ahora - 3600).situacion(ahora), Some(Situacion::Terminado));
        assert_eq!(p(FaseLiberar::Terminado, ahora - RESUMEN_SEG).situacion(ahora), None);
        assert_eq!(p(FaseLiberar::Fallido, ahora - 60).situacion(ahora), Some(Situacion::Fallido));
    }

    #[test]
    fn tiempo_restante_y_fraccion() {
        let mut a = p(FaseLiberar::Borrando, 0);
        assert_eq!((a.restante_seg(), a.fraccion()), (None, None));
        a.total = 11029;
        a.hechos = 2350;
        assert_eq!(a.restante_seg(), None, "sin ritmo medido");
        a.por_seg = Some(10.0);
        assert_eq!(a.restante_seg(), Some(868));
        assert!((a.fraccion().unwrap() - 2350.0 / 11029.0).abs() < 1e-9);
        a.hechos = a.total;
        assert_eq!(a.restante_seg(), None);
    }

    #[test]
    fn ritmo_ventana_movil() {
        let mut r = Ritmo::default();
        r.registrar(0.0, 0);
        r.registrar(3.0, 150);
        assert_eq!(r.por_seg(), None, "menos de 5 s medidos");
        r.registrar(10.0, 500);
        assert_eq!(r.por_seg(), Some(50.0));
        // Pasa a 10/s: las muestras viejas salen de la ventana.
        for t in 1..=12 {
            r.registrar(10.0 + t as f64 * 10.0, 500 + t * 100);
        }
        assert_eq!(r.por_seg(), Some(10.0));
        r.registrar(200.0, 5);
        assert_eq!(r.por_seg(), None, "un contador que vuelve atrás reinicia");
    }

    #[test]
    fn seguimiento_continua_el_mismo_liberar_y_descarta() {
        let reg = Mutex::new(Registro::en_memoria().unwrap());
        let mut s = Seguimiento::empezar(&reg, 30);
        let inicio = s.progreso().inicio_unix;
        s.avance(10, 0);
        assert_eq!(leer(&reg.lock().unwrap()).unwrap().hechos, 0, "justo tras una escritura, el avance espera");
        std::thread::sleep(std::time::Duration::from_millis(550));
        s.avance(20, 0);
        let leido = leer(&reg.lock().unwrap()).unwrap();
        assert_eq!((leido.fase, leido.hechos), (FaseLiberar::Listando, 20));
        let mut s = Seguimiento::continuar(&reg, None);
        assert_eq!((s.progreso().inicio_unix, s.progreso().dias), (inicio, 30));
        s.fase(FaseLiberar::Confirmando, 3, 9);
        s.descartar();
        assert_eq!(leer(&reg.lock().unwrap()), None);
        // Tras un final, el siguiente empieza de cero.
        let mut s = Seguimiento::empezar(&reg, 7);
        s.terminar(false);
        marcar_fallido(&reg, "x");
        assert_eq!(leer(&reg.lock().unwrap()).unwrap().fase, FaseLiberar::Terminado, "un final no se pisa");
        let s = Seguimiento::continuar(&reg, Some(5));
        assert_eq!(s.progreso().dias, 5);
    }

    #[test]
    fn marcar_fallido_conserva_lo_hecho() {
        let reg = Mutex::new(Registro::en_memoria().unwrap());
        let mut s = Seguimiento::empezar(&reg, 30);
        s.fase(FaseLiberar::Borrando, 100, 1000);
        s.contar(40, 2);
        std::thread::sleep(std::time::Duration::from_millis(550));
        s.avance(42, 400);
        latir(&reg);
        assert_eq!(leer(&reg.lock().unwrap()).unwrap().hechos, 42, "latir no cambia el avance");
        marcar_fallido(&reg, "borrando en iCloud: sin red");
        let p = leer(&reg.lock().unwrap()).unwrap();
        assert_eq!((p.fase, p.hechos, p.borrados, p.fallidos, p.bytes_hechos), (FaseLiberar::Fallido, 42, 40, 2, 400));
        assert_eq!(p.error.as_deref(), Some("borrando en iCloud: sin red"));
    }
}
