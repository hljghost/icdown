//! Asistente "Liberar espacio en iCloud" (sin dibujo): pasos, selección, controles, su
//! disposición en pantalla, teclado y el botón de mantener pulsado. Lo dibuja `asistente_gdi`
//! y lo aloja la bandeja (`asistente_ventana`), que hace el trabajo en el hilo del motor.
use crate::criterio::{self, Anio, Criterio, FilaInventario, Seleccion, PRESETS_DIAS};
use crate::i18n::{t, T};
use crate::progreso_liberar::ProgresoLiberar;
use std::collections::BTreeSet;
use std::path::PathBuf;

/// Tamaño del área de la ventana a 96 ppp.
pub const ANCHO: i32 = 560;
pub const ALTO: i32 = 640;
pub const M: i32 = 28;
pub const CONTENIDO_Y: i32 = 116;
pub const PIE_Y: i32 = ALTO - 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Paso {
    Elegir,
    Revisar,
    Confirmar,
    EnMarcha,
    Resultado,
}

impl Paso {
    pub const TODOS: [Paso; 5] = [Paso::Elegir, Paso::Revisar, Paso::Confirmar, Paso::EnMarcha, Paso::Resultado];

    pub fn nombre(self) -> &'static str {
        t(match self {
            Paso::Elegir => T::AsPaso1,
            Paso::Revisar => T::AsPaso2,
            Paso::Confirmar => T::AsPaso3,
            Paso::EnMarcha => T::AsPaso4,
            Paso::Resultado => T::AsPaso5,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modo {
    Dias,
    Anios,
}

/// Lo que dejó lista la revisión (listado + re-verificación) en el motor.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ResumenPlan {
    pub elementos: u64,
    pub bytes: u64,
    pub desde: Option<i64>,
    pub hasta: Option<i64>,
    /// Candidatos re-verificados en disco (los listos más los excluidos).
    pub reverificados: u64,
    pub excluidos_reverificacion: u64,
    pub no_verificados: u64,
    pub no_liberables: u64,
    pub atipicos: u64,
    /// `.icdown\plan_liberar.csv`
    pub lista: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ResumenFinal {
    pub borrados: u64,
    pub bytes: u64,
    pub fallidos: u64,
    pub cancelado: bool,
    pub sesion: String,
    /// `.icdown\borrados.csv`
    pub csv: Option<PathBuf>,
}

/// Lo que cuenta el motor al asistente.
#[derive(Debug, Clone, PartialEq)]
pub enum EventoLiberar {
    Plan(ResumenPlan),
    RevisionFallida(String),
    Terminado(ResumenFinal),
    Fallido(String),
    Restaurado { restaurados: u64, fallidos: u64 },
    RestaurarFallido(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Revision {
    EnCurso,
    Lista(ResumenPlan),
    Error(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Restauracion {
    Nada,
    /// Primer clic: pide un segundo para confirmar.
    Confirmar,
    EnCurso,
    Hecha { restaurados: u64, fallidos: u64 },
    Error(String),
}

/// Lo que el asistente pide a la bandeja.
#[derive(Debug, Clone, PartialEq)]
pub enum Accion {
    Revisar(Criterio),
    CancelarRevision,
    /// Volver a elegir o cerrar sin borrar: el motor olvida el plan.
    Descartar,
    Ejecutar,
    Pausar(bool),
    CancelarBorrado,
    AbrirArchivo(PathBuf),
    AbrirCarpeta,
    Restaurar(String),
    Cerrar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    ModoDias,
    Preset(usize),
    ModoAnios,
    Anio(i32),
    VerLista,
    AbrirCarpeta,
    Entendido,
    Mantener,
    VerBorrados,
    Restaurar,
    /// Botones del pie.
    Secundario,
    Primario,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn contiene(&self, x: f64, y: f64) -> bool {
        x >= self.x as f64 && x < (self.x + self.w) as f64 && y >= self.y as f64 && y < (self.y + self.h) as f64
    }
}

/// Botón de mantener pulsado: se dispara al cumplirse `DURACION` sin soltar; soltar antes cancela.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mantener {
    desde: Option<f64>,
    hecho: bool,
}

impl Mantener {
    pub const DURACION: f64 = 2.0;

    pub fn pulsar(&mut self, t: f64) {
        if !self.hecho && self.desde.is_none() {
            self.desde = Some(t);
        }
    }

    pub fn soltar(&mut self) {
        if !self.hecho {
            self.desde = None;
        }
    }

    pub fn pulsado(&self) -> bool {
        self.desde.is_some() && !self.hecho
    }

    /// 0–1: cuánto se ha llenado.
    pub fn avance(&self, t: f64) -> f64 {
        if self.hecho {
            return 1.0;
        }
        self.desde.map(|d| ((t - d) / Self::DURACION).clamp(0.0, 1.0)).unwrap_or(0.0)
    }

    /// `true` una sola vez, cuando se completa.
    pub fn tick(&mut self, t: f64) -> bool {
        if !self.hecho && self.desde.is_some_and(|d| t - d >= Self::DURACION) {
            self.hecho = true;
            return true;
        }
        false
    }

    pub fn reiniciar(&mut self) {
        *self = Mantener::default();
    }
}

pub struct Asistente {
    pub paso: Paso,
    pub modo: Modo,
    pub preset: usize,
    pub anios_elegidos: BTreeSet<i32>,
    pub inventario: Vec<FilaInventario>,
    pub anios: Vec<Anio>,
    pub ahora: i64,
    seleccion: Seleccion,
    pub revision: Revision,
    pub entendido: bool,
    pub mantener: Mantener,
    pub en_pausa: bool,
    pub cancelando: bool,
    pub resultado: Option<ResumenFinal>,
    pub restauracion: Restauracion,
    /// Avance leído del registro (lo escribe el motor).
    pub progreso: Option<ProgresoLiberar>,
    pub foco: Option<Control>,
    pub hover: Option<Control>,
    /// Control sobre el que se pulsó el ratón (el clic cuenta al soltar encima).
    pub pulsado: Option<Control>,
}

impl Asistente {
    pub fn nuevo(inventario: Vec<FilaInventario>, ahora: i64) -> Asistente {
        let anios = criterio::por_anios(&inventario, ahora);
        let mut a = Asistente {
            paso: Paso::Elegir,
            modo: Modo::Dias,
            preset: 0,
            anios_elegidos: BTreeSet::new(),
            inventario,
            anios,
            ahora,
            seleccion: Seleccion::default(),
            revision: Revision::EnCurso,
            entendido: false,
            mantener: Mantener::default(),
            en_pausa: false,
            cancelando: false,
            resultado: None,
            restauracion: Restauracion::Nada,
            progreso: None,
            foco: None,
            hover: None,
            pulsado: None,
        };
        a.recalcular();
        a
    }

    /// Vuelve a empezar con un inventario nuevo (al abrir el asistente otra vez).
    pub fn reiniciar(&mut self, inventario: Vec<FilaInventario>, ahora: i64) {
        let (modo, preset, anios) = (self.modo, self.preset, std::mem::take(&mut self.anios_elegidos));
        *self = Asistente::nuevo(inventario, ahora);
        self.modo = modo;
        self.preset = preset;
        self.anios_elegidos = anios.into_iter().filter(|a| self.anios.iter().any(|x| x.anio == *a)).collect();
        self.recalcular();
    }

    pub fn criterio(&self) -> Criterio {
        match self.modo {
            Modo::Dias => Criterio::ConservarDias(PRESETS_DIAS[self.preset]),
            Modo::Anios => Criterio::Anios(self.anios_elegidos.clone()),
        }
    }

    pub fn seleccion(&self) -> &Seleccion {
        &self.seleccion
    }

    fn recalcular(&mut self) {
        self.seleccion = criterio::seleccionar(&self.inventario, &self.criterio(), self.ahora);
    }

    /// ¿Hay un borrado en marcha (o pendiente de confirmar) que no se debe abandonar?
    pub fn ocupado(&self) -> bool {
        self.paso == Paso::EnMarcha
    }

    // ---- Disposición (96 ppp) ----

    /// Columnas de la rejilla de años: 3, o más si hay muchos años (5 filas como mucho).
    fn columnas_anios(&self) -> i32 {
        ((self.anios.len() as i32 + 4) / 5).clamp(3, 5)
    }

    pub fn rect_anio(&self, i: usize) -> Rect {
        let cols = self.columnas_anios();
        let ancho = ANCHO - 2 * M - 28;
        let w = (ancho - 8 * (cols - 1)) / cols;
        let (f, c) = (i as i32 / cols, i as i32 % cols);
        Rect::new(M + 28 + c * (w + 8), 240 + f * 44, w, 40)
    }

    pub fn rect_preset(&self, i: usize) -> Rect {
        let w = 72;
        Rect::new(M + 28 + i as i32 * (w + 8), 204, w, 30)
    }

    /// Botones del pie: (secundario, primario).
    pub fn rect_pie(&self, c: Control) -> Rect {
        let (w, h, y) = (128, 32, PIE_Y + 16);
        match c {
            Control::Primario => Rect::new(ANCHO - M - w, y, w, h),
            _ => Rect::new(ANCHO - M - 2 * w - 8, y, w, h),
        }
    }

    /// "Por años": bajo los días si se ven sus opciones; si no, justo debajo.
    pub fn rect_modo_anios(&self) -> Rect {
        Rect::new(M, if self.modo == Modo::Dias { 246 } else { 204 }, ANCHO - 2 * M, 28)
    }

    pub const RECT_MANTENER: Rect = Rect::new(M, 324, ANCHO - 2 * M, 56);

    /// Controles visibles del paso actual, en orden de tabulación, con su rectángulo.
    pub fn controles(&self) -> Vec<(Control, Rect)> {
        let mut v = Vec::new();
        match self.paso {
            Paso::Elegir => {
                v.push((Control::ModoDias, Rect::new(M, 168, ANCHO - 2 * M, 28)));
                if self.modo == Modo::Dias {
                    for i in 0..PRESETS_DIAS.len() {
                        v.push((Control::Preset(i), self.rect_preset(i)));
                    }
                }
                v.push((Control::ModoAnios, self.rect_modo_anios()));
                if self.modo == Modo::Anios {
                    for (i, a) in self.anios.iter().enumerate() {
                        v.push((Control::Anio(a.anio), self.rect_anio(i)));
                    }
                }
            }
            Paso::Revisar => {
                if let Revision::Lista(r) = &self.revision {
                    if r.lista.is_some() && r.elementos > 0 {
                        v.push((Control::VerLista, Rect::new(M, 448, 140, 32)));
                        v.push((Control::AbrirCarpeta, Rect::new(M + 148, 448, 140, 32)));
                    }
                }
            }
            Paso::Confirmar => {
                v.push((Control::Entendido, Rect::new(M, 244, ANCHO - 2 * M, 44)));
                v.push((Control::Mantener, Self::RECT_MANTENER));
            }
            Paso::EnMarcha => {}
            Paso::Resultado => {
                if self.resultado.as_ref().is_some_and(|r| r.csv.is_some()) {
                    v.push((Control::VerBorrados, Rect::new(M, 336, 172, 32)));
                }
                if self.resultado.as_ref().is_some_and(|r| r.borrados > 0) && !matches!(self.restauracion, Restauracion::Hecha { .. }) {
                    v.push((Control::Restaurar, Rect::new(M + 180, 336, 220, 32)));
                }
            }
        }
        if self.texto_pie(Control::Secundario).is_some() {
            v.push((Control::Secundario, self.rect_pie(Control::Secundario)));
        }
        if self.texto_pie(Control::Primario).is_some() {
            v.push((Control::Primario, self.rect_pie(Control::Primario)));
        }
        v
    }

    pub fn control_en(&self, x: f64, y: f64) -> Option<Control> {
        self.controles().into_iter().find(|(_, r)| r.contiene(x, y)).map(|(c, _)| c)
    }

    /// Rótulo de los botones del pie (`None` = no hay).
    pub fn texto_pie(&self, c: Control) -> Option<&'static str> {
        let (sec, pri) = match self.paso {
            Paso::Elegir => (Some(T::AsCancelar), Some(T::AsRevisarBtn)),
            Paso::Revisar => match self.revision {
                Revision::EnCurso => (None, Some(T::AsCancelar)),
                Revision::Lista(ref r) if r.elementos > 0 => (Some(T::AsAtras), Some(T::AsContinuar)),
                _ => (None, Some(T::AsAtras)),
            },
            Paso::Confirmar => (None, Some(T::AsAtras)),
            Paso::EnMarcha => (Some(if self.en_pausa { T::AsReanudar } else { T::AsPausar }), Some(T::AsCancelarBorrado)),
            Paso::Resultado => (None, Some(T::AsCerrar)),
        };
        match c {
            Control::Secundario => sec.map(t),
            Control::Primario => pri.map(t),
            _ => None,
        }
    }

    /// ¿Se puede usar? (los desactivados se ven atenuados y no reciben el foco).
    pub fn habilitado(&self, c: Control) -> bool {
        match c {
            Control::Primario => match self.paso {
                Paso::Elegir => self.seleccion.elementos > 0,
                Paso::EnMarcha => !self.cancelando,
                _ => true,
            },
            Control::Secundario if self.paso == Paso::EnMarcha => !self.cancelando,
            Control::Mantener => self.entendido,
            Control::Anio(a) => self.anios.iter().any(|x| x.anio == a && x.elementos > 0),
            Control::Restaurar => !matches!(self.restauracion, Restauracion::EnCurso),
            _ => true,
        }
    }

    /// Nombre para la accesibilidad (y el título de la ventana con el foco).
    pub fn nombre(&self, c: Control) -> String {
        match c {
            Control::ModoDias => t(T::AsModoDias).into(),
            Control::ModoAnios => t(T::AsModoAnios).into(),
            Control::Preset(i) => format!("{} {}", t(T::AsUltimos), texto_preset(i)),
            Control::Anio(a) => a.to_string(),
            Control::VerLista => t(T::AsVerLista).into(),
            Control::AbrirCarpeta => t(T::AsAbrirCarpetas).into(),
            Control::Entendido => t(T::AsEntiendo).into(),
            Control::Mantener => crate::i18n::tf(T::AsMantener, &[("b", &crate::bandeja::panel::bytes_dec(self.bytes_plan()))]),
            Control::VerBorrados => t(T::AsVerBorrados).into(),
            Control::Restaurar => t(T::AsRestaurar).into(),
            Control::Secundario | Control::Primario => self.texto_pie(c).unwrap_or_default().into(),
        }
    }

    pub fn bytes_plan(&self) -> u64 {
        match &self.revision {
            Revision::Lista(r) => r.bytes,
            _ => 0,
        }
    }

    // ---- Interacción ----

    /// Activa un control (clic, Espacio o Intro). Devuelve lo que hay que pedir a la bandeja.
    pub fn activar(&mut self, c: Control) -> Option<Accion> {
        if !self.habilitado(c) {
            return None;
        }
        self.foco = Some(c);
        match (self.paso, c) {
            (_, Control::ModoDias) => {
                self.modo = Modo::Dias;
                self.recalcular();
            }
            (_, Control::ModoAnios) => {
                self.modo = Modo::Anios;
                self.recalcular();
            }
            (_, Control::Preset(i)) => {
                self.modo = Modo::Dias;
                self.preset = i.min(PRESETS_DIAS.len() - 1);
                self.recalcular();
            }
            (_, Control::Anio(a)) => {
                self.modo = Modo::Anios;
                if !self.anios_elegidos.remove(&a) {
                    self.anios_elegidos.insert(a);
                }
                self.recalcular();
            }
            (Paso::Elegir, Control::Secundario) => return Some(Accion::Cerrar),
            (Paso::Elegir, Control::Primario) => {
                self.paso = Paso::Revisar;
                self.revision = Revision::EnCurso;
                self.progreso = None;
                self.foco = Some(Control::Primario);
                return Some(Accion::Revisar(self.criterio()));
            }
            (Paso::Revisar, Control::Primario) if self.revision == Revision::EnCurso => {
                self.paso = Paso::Elegir;
                self.foco = Some(Control::Primario);
                return Some(Accion::CancelarRevision);
            }
            (Paso::Revisar, Control::Primario) if matches!(&self.revision, Revision::Lista(r) if r.elementos > 0) => {
                self.paso = Paso::Confirmar;
                self.entendido = false;
                self.mantener.reiniciar();
                self.foco = Some(Control::Entendido);
            }
            (Paso::Revisar, Control::Primario | Control::Secundario) => {
                self.paso = Paso::Elegir;
                self.foco = Some(Control::Primario);
                return Some(Accion::Descartar);
            }
            (_, Control::VerLista) => {
                if let Revision::Lista(r) = &self.revision {
                    return r.lista.clone().map(Accion::AbrirArchivo);
                }
            }
            (_, Control::AbrirCarpeta) => return Some(Accion::AbrirCarpeta),
            (_, Control::Entendido) => {
                self.entendido = !self.entendido;
                self.mantener.reiniciar();
            }
            (Paso::Confirmar, Control::Primario) => {
                self.paso = Paso::Revisar;
                self.foco = Some(Control::Primario);
            }
            (Paso::EnMarcha, Control::Secundario) => {
                self.en_pausa = !self.en_pausa;
                return Some(Accion::Pausar(self.en_pausa));
            }
            (Paso::EnMarcha, Control::Primario) => {
                self.cancelando = true;
                return Some(Accion::CancelarBorrado);
            }
            (_, Control::VerBorrados) => return self.resultado.as_ref().and_then(|r| r.csv.clone()).map(Accion::AbrirArchivo),
            (_, Control::Restaurar) => match self.restauracion {
                Restauracion::Confirmar => {
                    self.restauracion = Restauracion::EnCurso;
                    return self.resultado.as_ref().map(|r| Accion::Restaurar(r.sesion.clone()));
                }
                _ => self.restauracion = Restauracion::Confirmar,
            },
            (Paso::Resultado, Control::Primario) => return Some(Accion::Cerrar),
            _ => {}
        }
        None
    }

    /// Empieza a mantener pulsado el botón de liberar (ratón o teclado).
    pub fn pulsar_mantener(&mut self, t: f64) {
        if self.paso == Paso::Confirmar && self.entendido {
            self.mantener.pulsar(t);
        }
    }

    pub fn soltar_mantener(&mut self) {
        self.mantener.soltar();
    }

    /// Avanza el tiempo: al completarse la pulsación empieza el borrado.
    pub fn tick(&mut self, t: f64) -> Option<Accion> {
        if self.paso == Paso::Confirmar && self.mantener.tick(t) {
            self.paso = Paso::EnMarcha;
            self.en_pausa = false;
            self.cancelando = false;
            self.progreso = None;
            self.foco = Some(Control::Secundario);
            return Some(Accion::Ejecutar);
        }
        None
    }

    /// Tab / Mayús+Tab: siguiente control habilitado.
    pub fn mover_foco(&mut self, atras: bool) {
        let lista: Vec<Control> = self.controles().into_iter().map(|(c, _)| c).filter(|&c| self.habilitado(c)).collect();
        if lista.is_empty() {
            self.foco = None;
            return;
        }
        let i = self.foco.and_then(|f| lista.iter().position(|&c| c == f));
        let n = lista.len();
        let j = match (i, atras) {
            (None, false) => 0,
            (None, true) => n - 1,
            (Some(i), false) => (i + 1) % n,
            (Some(i), true) => (i + n - 1) % n,
        };
        self.foco = Some(lista[j]);
    }

    /// Flechas: dentro de los días o de los años, mueve (y elige, en los días) el vecino.
    pub fn flecha(&mut self, dx: i32, dy: i32) {
        match self.foco {
            Some(Control::Preset(i)) => {
                let j = (i as i32 + dx + dy).clamp(0, PRESETS_DIAS.len() as i32 - 1) as usize;
                self.activar(Control::Preset(j));
            }
            Some(Control::Anio(a)) => {
                let cols = self.columnas_anios();
                let Some(i) = self.anios.iter().position(|x| x.anio == a) else { return };
                let j = i as i32 + dx + dy * cols;
                if (0..self.anios.len() as i32).contains(&j) {
                    self.foco = Some(Control::Anio(self.anios[j as usize].anio));
                }
            }
            Some(Control::ModoDias) if dy > 0 => self.foco = Some(Control::ModoAnios),
            Some(Control::ModoAnios) if dy < 0 => self.foco = Some(Control::ModoDias),
            _ => {}
        }
    }

    pub fn evento(&mut self, e: EventoLiberar) {
        match e {
            EventoLiberar::Plan(r) => {
                if self.paso == Paso::Revisar {
                    self.revision = Revision::Lista(r);
                }
            }
            EventoLiberar::RevisionFallida(m) => {
                if self.paso == Paso::Revisar {
                    self.revision = Revision::Error(m);
                }
            }
            EventoLiberar::Terminado(r) => {
                self.paso = Paso::Resultado;
                self.resultado = Some(r);
                self.restauracion = Restauracion::Nada;
                self.foco = Some(Control::Primario);
            }
            EventoLiberar::Fallido(m) => {
                self.paso = Paso::Revisar;
                self.revision = Revision::Error(m);
            }
            EventoLiberar::Restaurado { restaurados, fallidos } => self.restauracion = Restauracion::Hecha { restaurados, fallidos },
            EventoLiberar::RestaurarFallido(m) => self.restauracion = Restauracion::Error(m),
        }
    }
}

pub fn texto_preset(i: usize) -> &'static str {
    t([T::Preset30, T::Preset90, T::Preset180, T::Preset365, T::Preset730, T::Preset1095][i.min(5)])
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIA: i64 = 86_400;
    const AHORA: i64 = 2_000_000_000;

    fn fila(anio: i32, hace_dias: i64) -> FilaInventario {
        FilaInventario { anio, fecha_unix: AHORA - hace_dias * DIA, agregado_unix: None, personal: true, liberable: true, completo: true, bytes: 1_000, atipico: false }
    }

    fn asistente() -> Asistente {
        Asistente::nuevo(vec![fila(2026, 5), fila(2025, 200), fila(2024, 600), fila(2024, 650), fila(2019, 2500)], AHORA)
    }

    #[test]
    fn mantener_pulsado_dos_segundos() {
        let mut m = Mantener::default();
        m.pulsar(10.0);
        assert!(!m.tick(11.0));
        assert!((m.avance(11.0) - 0.5).abs() < 1e-9);
        m.soltar();
        assert_eq!(m.avance(11.5), 0.0, "soltar antes cancela");
        m.pulsar(20.0);
        m.pulsar(21.0); // repetición de tecla: no reinicia
        assert!(m.pulsado());
        assert!(m.tick(22.0));
        assert!(!m.tick(23.0), "solo una vez");
        m.soltar();
        assert_eq!(m.avance(30.0), 1.0, "hecho se queda hecho");
    }

    #[test]
    fn elegir_dias_y_anios_cambia_la_seleccion_al_momento() {
        let mut a = asistente();
        assert_eq!(a.seleccion().elementos, 4, "30 días: todo menos lo de hace 5 días");
        a.activar(Control::Preset(3)); // 1 año
        assert_eq!((a.criterio(), a.seleccion().elementos), (Criterio::ConservarDias(365), 3));
        a.activar(Control::ModoAnios);
        assert_eq!(a.seleccion().elementos, 0);
        assert!(!a.habilitado(Control::Primario), "sin años elegidos no se puede seguir");
        assert!(!a.habilitado(Control::Anio(2026)), "2026 no tiene nada liberable aún");
        a.activar(Control::Anio(2024));
        assert_eq!((a.seleccion().elementos, a.seleccion().bytes), (2, 2_000));
        a.activar(Control::Anio(2024));
        assert!(a.anios_elegidos.is_empty(), "segundo clic desmarca");
        assert_eq!(a.anios.iter().map(|x| x.anio).collect::<Vec<_>>(), vec![2026, 2025, 2024, 2019]);
    }

    #[test]
    fn flujo_completo_con_acciones() {
        let mut a = asistente();
        let Some(Accion::Revisar(c)) = a.activar(Control::Primario) else { panic!() };
        assert_eq!(c, Criterio::ConservarDias(30));
        assert_eq!((a.paso, a.texto_pie(Control::Primario)), (Paso::Revisar, Some("Cancelar")));
        a.evento(EventoLiberar::Plan(ResumenPlan { elementos: 4, bytes: 4_000, lista: Some("plan.csv".into()), ..Default::default() }));
        assert!(a.controles().iter().any(|(c, _)| *c == Control::VerLista));
        assert_eq!(a.activar(Control::VerLista), Some(Accion::AbrirArchivo("plan.csv".into())));
        a.activar(Control::Primario);
        assert_eq!(a.paso, Paso::Confirmar);
        // Sin la casilla, mantener no hace nada.
        a.pulsar_mantener(0.0);
        assert_eq!(a.tick(5.0), None);
        a.activar(Control::Entendido);
        a.pulsar_mantener(10.0);
        assert_eq!(a.tick(11.0), None);
        assert_eq!(a.tick(12.0), Some(Accion::Ejecutar));
        assert_eq!(a.paso, Paso::EnMarcha);
        assert_eq!(a.activar(Control::Secundario), Some(Accion::Pausar(true)));
        assert_eq!(a.texto_pie(Control::Secundario), Some("Reanudar"));
        assert_eq!(a.activar(Control::Primario), Some(Accion::CancelarBorrado));
        assert!(!a.habilitado(Control::Primario));
        a.evento(EventoLiberar::Terminado(ResumenFinal { borrados: 3, bytes: 3_000, sesion: "s1".into(), cancelado: true, ..Default::default() }));
        assert_eq!(a.paso, Paso::Resultado);
        assert_eq!(a.activar(Control::Restaurar), None, "primero pide confirmar");
        assert_eq!(a.activar(Control::Restaurar), Some(Accion::Restaurar("s1".into())));
        a.evento(EventoLiberar::Restaurado { restaurados: 3, fallidos: 0 });
        assert!(!a.controles().iter().any(|(c, _)| *c == Control::Restaurar));
        assert_eq!(a.activar(Control::Primario), Some(Accion::Cerrar));
    }

    #[test]
    fn volver_atras_descarta_el_plan_y_errores() {
        let mut a = asistente();
        a.activar(Control::Primario);
        a.evento(EventoLiberar::Plan(ResumenPlan::default()));
        assert_eq!(a.texto_pie(Control::Primario), Some("Atrás"), "nada que liberar: solo atrás");
        assert_eq!(a.activar(Control::Primario), Some(Accion::Descartar));
        assert_eq!(a.paso, Paso::Elegir);
        a.activar(Control::Primario);
        assert_eq!(a.activar(Control::Primario), Some(Accion::CancelarRevision));
        a.activar(Control::Primario);
        a.evento(EventoLiberar::RevisionFallida("sin red".into()));
        assert_eq!(a.revision, Revision::Error("sin red".into()));
    }

    #[test]
    fn teclado_tab_y_flechas() {
        let mut a = asistente();
        a.mover_foco(false);
        assert_eq!(a.foco, Some(Control::ModoDias));
        a.mover_foco(false);
        assert_eq!(a.foco, Some(Control::Preset(0)));
        a.flecha(1, 0);
        assert_eq!((a.foco, a.preset), (Some(Control::Preset(1)), 1));
        a.mover_foco(true);
        a.mover_foco(true);
        a.mover_foco(true);
        assert_eq!(a.foco, Some(Control::Primario), "Mayús+Tab da la vuelta");
        a.activar(Control::ModoAnios);
        a.mover_foco(false);
        // Los años sin nada liberable no reciben el foco.
        assert_eq!(a.foco, Some(Control::Anio(2025)));
        a.flecha(1, 0);
        assert_eq!(a.foco, Some(Control::Anio(2024)));
        assert!(!a.nombre(Control::Preset(2)).is_empty());
    }

    #[test]
    fn controles_dentro_de_la_ventana_y_sin_solaparse() {
        let mut inv = Vec::new();
        for anio in 2004..2027 {
            inv.push(fila(anio, (2027 - anio as i64) * 365));
        }
        let mut a = Asistente::nuevo(inv, AHORA);
        a.activar(Control::ModoAnios);
        for paso in Paso::TODOS {
            a.paso = paso;
            a.revision = Revision::Lista(ResumenPlan { elementos: 1, lista: Some("x".into()), ..Default::default() });
            a.resultado = Some(ResumenFinal { borrados: 1, csv: Some("y".into()), ..Default::default() });
            let cs = a.controles();
            for (i, (c, r)) in cs.iter().enumerate() {
                assert!(r.x >= M && r.x + r.w <= ANCHO - M + 1 && r.y >= CONTENIDO_Y && r.y + r.h <= ALTO, "{paso:?} {c:?} {r:?}");
                for (c2, r2) in &cs[i + 1..] {
                    let solapa = r.x < r2.x + r2.w && r2.x < r.x + r.w && r.y < r2.y + r2.h && r2.y < r.y + r.h;
                    assert!(!solapa, "{paso:?}: {c:?} y {c2:?}");
                }
            }
        }
        let ultimo = a.rect_anio(a.anios.len() - 1);
        assert!(ultimo.y + ultimo.h <= 460, "la rejilla de años cabe sobre el resumen: {ultimo:?}");
    }
}
