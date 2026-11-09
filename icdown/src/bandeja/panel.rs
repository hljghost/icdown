//! Lógica del panel de progreso de la bandeja (sin dibujo): textos, indicador, velocidad y
//! tiempo restante.
use crate::bandeja::estado::{miles, EstadoUi, Fase};
use crate::i18n::{decimal1, t, tf, T};
use crate::progreso_liberar::{FaseLiberar, ProgresoLiberar, Situacion};
use std::collections::VecDeque;

/// Indicador de la cabecera del panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Indicador {
    EnVivo,
    AlDia,
    EnEspera,
    EnPausa,
    InicieSesion,
    Atencion,
    Error,
}

impl Indicador {
    pub fn texto(self) -> &'static str {
        t(match self {
            Indicador::EnVivo => T::IndVivo,
            Indicador::AlDia => T::IndAlDia,
            Indicador::EnEspera => T::IndEspera,
            Indicador::EnPausa => T::IndPausa,
            Indicador::InicieSesion => T::IndSesion,
            Indicador::Atencion => T::IndAtencion,
            Indicador::Error => T::IndError,
        })
    }

    /// El punto late solo mientras hay trabajo en marcha.
    pub fn late(self) -> bool {
        self == Indicador::EnVivo
    }
}

pub fn indicador(e: &EstadoUi) -> Indicador {
    match &e.fase {
        Fase::Listando { .. } | Fase::Copiando { .. } | Fase::Verificando { .. } => Indicador::EnVivo,
        Fase::Inactiva if e.total == 0 || e.verificadas < e.total => Indicador::EnEspera,
        Fase::Inactiva => Indicador::AlDia,
        Fase::Pausada => Indicador::EnPausa,
        Fase::SesionCaducada => Indicador::InicieSesion,
        Fase::SinEspacio | Fase::DestinoAusente | Fase::SinDestino => Indicador::Atencion,
        Fase::Error(_) => Indicador::Error,
    }
}

/// Velocidad media de descarga sobre una ventana móvil de muestras (segundos, bytes acumulados).
#[derive(Debug, Default)]
pub struct Velocimetro {
    muestras: VecDeque<(f64, u64)>,
}

impl Velocimetro {
    const VENTANA: f64 = 8.0;

    pub fn registrar(&mut self, t: f64, bytes: u64) {
        if let Some(&(_, b)) = self.muestras.back() {
            if bytes < b {
                self.muestras.clear();
            }
        }
        self.muestras.push_back((t, bytes));
        while self.muestras.len() > 2 && t - self.muestras[1].0 >= Self::VENTANA {
            self.muestras.pop_front();
        }
    }

    /// Bytes por segundo; `None` hasta tener al menos un segundo de muestras.
    pub fn bytes_por_seg(&self) -> Option<f64> {
        self.medida()
    }

    /// Empieza de cero (nueva ronda).
    pub fn reiniciar(&mut self) {
        self.muestras.clear();
    }

    fn medida(&self) -> Option<f64> {
        let (&(t0, b0), &(t1, b1)) = (self.muestras.front()?, self.muestras.back()?);
        (t1 - t0 >= 1.0).then(|| (b1 - b0) as f64 / (t1 - t0))
    }
}

/// Número con coma decimal: un decimal por debajo de 10, entero a partir de ahí.
fn num(x: f64) -> String {
    if x < 10.0 && x.fract().abs() > 0.05 {
        decimal1(x)
    } else {
        miles(x.round() as u64)
    }
}

fn unidad(b: u64) -> (f64, &'static str) {
    const K: f64 = 1000.0;
    match b as f64 {
        x if x >= K * K * K * K => (K * K * K * K, "TB"),
        x if x >= K * K * K => (K * K * K, "GB"),
        x if x >= K * K => (K * K, "MB"),
        x if x >= K => (K, "kB"),
        _ => (1.0, "B"),
    }
}

/// "108 de 210 GB": ambas cifras en la unidad de la mayor.
pub fn par_bytes(a: u64, b: u64) -> String {
    let (u, nombre) = unidad(a.max(b));
    let (x, y) = (a as f64 / u, b as f64 / u);
    // Por debajo de 10, las dos cifras con un decimal para que se vea el avance.
    let f = |v: f64| if y < 10.0 && u > 1.0 { decimal1(v) } else { num(v) };
    tf(T::ParBytes, &[("a", &f(x)), ("b", &f(y)), ("u", &nombre)])
}

/// "12,4 MB/s"
pub fn velocidad(bps: f64) -> String {
    let (u, nombre) = unidad(bps.max(0.0) as u64);
    let x = bps.max(0.0) / u;
    let cifra = if x < 100.0 { decimal1(x) } else { miles(x.round() as u64) };
    format!("{cifra} {nombre}/s")
}

/// "~58 min", "~2 h 10 min", "~3 días", "menos de 1 min"
pub fn duracion(segundos: u64) -> String {
    let min = segundos.div_ceil(60);
    if segundos < 60 {
        t(T::MenosDeUnMin).into()
    } else if min < 60 {
        tf(T::DurMin, &[("m", &min)])
    } else if min < 48 * 60 {
        let (h, m) = (min / 60, min % 60);
        if m == 0 {
            tf(T::DurH, &[("h", &h)])
        } else {
            tf(T::DurHMin, &[("h", &h), ("m", &m)])
        }
    } else {
        tf(T::DurDias, &[("d", &min.div_ceil(24 * 60))])
    }
}

/// "~58 min restantes"
pub fn restante(segundos: u64) -> String {
    if segundos < 60 {
        t(T::RestanteMenos).into()
    } else {
        tf(T::Restantes, &[("d", &duracion(segundos))])
    }
}

/// "hace 5 min"
pub fn hace(segundos: i64) -> String {
    let s = segundos.max(0);
    match s {
        0..=59 => t(T::HaceMomento).into(),
        60..=3599 => tf(T::HaceMin, &[("n", &(s / 60))]),
        3600..=86399 => tf(T::HaceH, &[("n", &(s / 3600))]),
        _ if s / 86400 == 1 => t(T::HaceDia).into(),
        _ => tf(T::HaceDias, &[("n", &(s / 86400))]),
    }
}

/// ¿Empieza una ronda nueva? (para reiniciar la velocidad y el tiempo restante)
pub fn es_nueva_ronda(antes: &EstadoUi, despues: &EstadoUi) -> bool {
    let listando = matches!(despues.fase, Fase::Listando { .. });
    listando && !matches!(antes.fase, Fase::Listando { .. }) || despues.trabajando() && !antes.trabajando()
}

/// Acción del botón principal del panel, según el estado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Principal {
    Pausar,
    Reanudar,
    RevisarAhora,
    IniciarSesion,
}

impl Principal {
    pub fn texto(self) -> &'static str {
        t(match self {
            Principal::Pausar => T::BotPausar,
            Principal::Reanudar => T::BotReanudar,
            Principal::RevisarAhora => T::BotRevisar,
            Principal::IniciarSesion => T::BotSesion,
        })
    }
}

pub fn principal(e: &EstadoUi) -> Principal {
    match e.fase {
        _ if e.trabajando() => Principal::Pausar,
        Fase::Pausada => Principal::Reanudar,
        Fase::SesionCaducada => Principal::IniciarSesion,
        _ => Principal::RevisarAhora,
    }
}

/// Medidas que se toman en la interfaz (no vienen del motor).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Medidas {
    /// Velocidad de descarga actual (ventana móvil).
    pub bytes_por_seg: Option<f64>,
    /// Ritmo del listado de iCloud (ventana móvil).
    pub elementos_por_seg: Option<f64>,
    /// Velocidad media de la última ronda que descargó algo.
    pub media_ultima_ronda: Option<f64>,
}

/// Velocidad media de una ronda: bytes descargados entre que empieza y termina.
#[derive(Debug, Default)]
pub struct MediaRonda {
    inicio: Option<(f64, u64)>,
    media: Option<f64>,
}

impl MediaRonda {
    pub fn empezar(&mut self, t: f64, bytes: u64) {
        self.inicio = Some((t, bytes));
    }

    /// Al acabar la ronda; si no descargó nada, se conserva la media anterior.
    pub fn terminar(&mut self, t: f64, bytes: u64) {
        if let Some((t0, b0)) = self.inicio.take() {
            if bytes > b0 && t > t0 {
                self.media = Some((bytes - b0) as f64 / (t - t0));
            }
        }
    }

    pub fn media(&self) -> Option<f64> {
        self.media
    }
}

/// "~1.250 elementos/min"
pub fn ritmo(elementos_por_seg: f64) -> String {
    tf(T::Ritmo, &[("n", &miles((elementos_por_seg * 60.0).round() as u64))])
}

/// Todo lo que muestra el panel, ya en texto.
#[derive(Debug, Clone, PartialEq)]
pub struct Vista {
    pub indicador: Indicador,
    pub titulo: String,
    /// Fracción de la barra (0–1); `None` = barra indeterminada (listando).
    pub fraccion: Option<f64>,
    /// Avance de la ronda (o, en reposo, última y próxima revisión).
    pub linea: String,
    /// Totales de toda la fototeca.
    pub linea_total: String,
    pub ahora: String,
    /// "Velocidad", "Ritmo" (listando) o "Velocidad media" (en reposo).
    pub etiqueta_velocidad: &'static str,
    pub velocidad: String,
    pub listado: String,
    pub errores: String,
    pub principal: Principal,
    /// Sección de "liberar espacio" (solo si hay uno en marcha o terminado hace menos de un día).
    pub liberar: Option<VistaLiberar>,
    /// Botón "Liberar espacio…" en el pie: con todo copiado y verificado.
    pub boton_liberar: bool,
}

/// Color de la sección de liberar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TonoLiberar {
    /// En marcha (ámbar).
    Activo,
    /// Terminado bien (verde).
    Hecho,
    /// Falló o se interrumpió.
    Problema,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VistaLiberar {
    pub titulo: String,
    pub subtitulo: String,
    /// `None` = barra indeterminada.
    pub fraccion: Option<f64>,
    pub linea: String,
    pub linea2: String,
    pub tono: TonoLiberar,
}

/// "74,7 GB": un decimal por debajo de 100 en su unidad.
pub fn bytes_dec(b: u64) -> String {
    let (u, nombre) = unidad(b);
    format!("{} {nombre}", dec(b as f64 / u, u > 1.0))
}

/// "16,1 de 74,7 GB": ambas cifras en la unidad de la mayor, con un decimal por debajo de 100.
pub fn par_bytes_dec(a: u64, b: u64) -> String {
    let (u, nombre) = unidad(a.max(b));
    let grande = b as f64 / u >= 100.0;
    let f = |v: u64| dec(v as f64 / u, u > 1.0 && !grande);
    tf(T::ParBytes, &[("a", &f(a)), ("b", &f(b)), ("u", &nombre)])
}

fn dec(x: f64, decimal: bool) -> String {
    if decimal && x < 100.0 {
        decimal1(x)
    } else {
        miles(x.round() as u64)
    }
}

fn pct(p: &ProgresoLiberar) -> u64 {
    p.fraccion().map(|f| (f * 100.0).floor() as u64).unwrap_or(0)
}

/// Sección del panel para un liberar guardado en el registro; `None` si no hay nada que mostrar.
pub fn vista_liberar(p: &ProgresoLiberar, ahora_unix: i64) -> Option<VistaLiberar> {
    let sit = p.situacion(ahora_unix)?;
    let espera_apple = p.espera_hasta.filter(|_| p.fase == FaseLiberar::Borrando);
    let eta = p.restante_seg().filter(|_| !p.en_pausa && espera_apple.is_none()).map(|s| format!(" · {}", duracion(s))).unwrap_or_default();
    let cuando = hace(ahora_unix - p.actualizado_unix);
    let v = match sit {
        Situacion::EnCurso => {
            let (subtitulo, fraccion, linea) = match p.fase {
                FaseLiberar::Listando if p.total > 0 => (
                    T::LibConsultando,
                    p.fraccion(),
                    tf(T::LibListandoDe, &[("v", &miles(p.hechos)), ("t", &miles(p.total)), ("eta", &eta)]),
                ),
                FaseLiberar::Listando => (T::LibConsultando, None, tf(T::LibVistos, &[("n", &miles(p.hechos))])),
                _ => (
                    match p.fase {
                        FaseLiberar::Borrando if p.en_pausa => T::LibPausa,
                        FaseLiberar::Borrando => T::LibBorrando,
                        _ => T::LibComprobando,
                    },
                    p.fraccion().or(Some(0.0)),
                    format!("{} / {} · {} % · {}{eta}", miles(p.hechos), miles(p.total), pct(p), par_bytes_dec(p.bytes_hechos, p.bytes_total)),
                ),
            };
            let linea2 = match p.fase {
                FaseLiberar::Borrando if p.fallidos > 0 => tf(T::LibFallidosReintento, &[("n", &miles(p.fallidos))]),
                FaseLiberar::Borrando => t(T::LibVanAPapelera).into(),
                _ => tf(T::LibConserva, &[("d", &p.dias)]),
            };
            let subtitulo = match espera_apple {
                Some(h) if h > ahora_unix => tf(T::LibEsperaApple, &[("d", &duracion((h - ahora_unix) as u64))]),
                Some(_) => t(T::LibEsperaAppleYa).into(),
                None => t(subtitulo).into(),
            };
            VistaLiberar { titulo: t(T::LibTitulo).into(), subtitulo, fraccion, linea, linea2, tono: TonoLiberar::Activo }
        }
        Situacion::Confirmando => VistaLiberar {
            titulo: t(T::LibTitulo).into(),
            subtitulo: t(T::LibEsperando).into(),
            fraccion: Some(0.0),
            linea: tf(T::LibListos, &[("n", &miles(p.total)), ("b", &bytes_dec(p.bytes_total))]),
            linea2: t(T::LibConfirma).into(),
            tono: TonoLiberar::Activo,
        },
        Situacion::Terminado if p.cancelado => VistaLiberar {
            titulo: t(T::LibTituloCancelado).into(),
            subtitulo: tf(T::LibCancelada, &[("hace", &cuando)]),
            fraccion: p.fraccion(),
            linea: tf(T::LibLiberadosParcial, &[("b", &bytes_dec(p.bytes_hechos)), ("h", &miles(p.borrados)), ("t", &miles(p.total))]),
            linea2: t(T::LibVacia).into(),
            tono: TonoLiberar::Hecho,
        },
        Situacion::Terminado => {
            let mut subtitulo = tf(T::LibTerminado, &[("hace", &cuando)]);
            if p.fallidos > 0 {
                subtitulo.push_str(&tf(T::LibNoBorrados, &[("n", &miles(p.fallidos))]));
            }
            let liberados = if p.borrados == 1 { T::LibLiberados1 } else { T::LibLiberadosN };
            VistaLiberar {
                titulo: t(T::LibTituloHecho).into(),
                subtitulo,
                fraccion: Some(1.0),
                linea: tf(liberados, &[("b", &bytes_dec(p.bytes_hechos)), ("n", &miles(p.borrados))]),
                linea2: t(T::LibVacia).into(),
                tono: TonoLiberar::Hecho,
            }
        }
        Situacion::Fallido | Situacion::Interrumpido => {
            let subtitulo = match (&p.error, sit) {
                (Some(e), Situacion::Fallido) => tf(T::LibFalloMotivo, &[("hace", &cuando), ("e", e)]),
                (None, Situacion::Fallido) => tf(T::LibFallo, &[("hace", &cuando)]),
                _ => tf(T::LibDetenido, &[("hace", &cuando)]),
            };
            let linea = if p.borrados > 0 {
                tf(T::LibBorradosDe, &[("h", &miles(p.borrados)), ("t", &miles(p.total)), ("b", &bytes_dec(p.bytes_hechos))])
            } else {
                t(T::LibNadaBorrado).into()
            };
            VistaLiberar {
                titulo: t(T::LibTituloProblema).into(),
                subtitulo,
                fraccion: p.fraccion().filter(|_| p.borrados > 0),
                linea,
                linea2: t(T::LibReintentar).into(),
                tono: TonoLiberar::Problema,
            }
        }
    };
    Some(VistaLiberar { fraccion: v.fraccion.map(|f| f.clamp(0.0, 1.0)), ..v })
}

/// Texto del tooltip mientras se libera espacio (solo en marcha); `None` en otro caso.
pub fn texto_bandeja_liberar(p: &ProgresoLiberar, ahora_unix: i64) -> Option<String> {
    let (h, total) = (miles(p.hechos), miles(p.total));
    let texto = match (p.situacion(ahora_unix)?, p.fase) {
        (Situacion::Confirmando, _) => t(T::TipConfirmacion).to_string(),
        (Situacion::EnCurso, FaseLiberar::Listando) => tf(T::TipConsultando, &[("n", &h)]),
        (Situacion::EnCurso, FaseLiberar::Reverificando) => tf(T::TipComprobando, &[("h", &h), ("t", &total)]),
        (Situacion::EnCurso, _) if p.en_pausa => tf(T::TipEnPausa, &[("h", &h), ("t", &total)]),
        (Situacion::EnCurso, _) if p.espera_hasta.is_some() => tf(T::TipEsperaApple, &[("h", &h), ("t", &total)]),
        (Situacion::EnCurso, _) => {
            let eta = p.restante_seg().map(|s| format!(" · {}", duracion(s))).unwrap_or_default();
            tf(T::TipBorrando, &[("h", &h), ("t", &total), ("p", &pct(p)), ("eta", &eta)])
        }
        _ => return None,
    };
    Some(texto)
}

/// Texto en singular o plural con el número en `{n}`.
fn plural(n: u64, uno: T, varios: T) -> String {
    tf(if n == 1 { uno } else { varios }, &[("n", &miles(n))])
}

/// Se recalcula entera en cada refresco a partir del estado vivo (nada se guarda entre llamadas).
pub fn vista(e: &EstadoUi, m: Medidas, ahora_unix: i64) -> Vista {
    let d = &e.detalle;
    let ind = indicador(e);
    let vel = m.bytes_por_seg.filter(|v| e.trabajando() && *v > 0.0);
    let ritmo_listado = m.elementos_por_seg.filter(|v| *v > 0.0);
    let titulo = match &e.fase {
        Fase::Listando { .. } if d.renovando_enlaces => t(T::TitRenovando).to_string(),
        Fase::Listando { vistos: 0 } => t(T::TitRevisando).to_string(),
        Fase::Listando { vistos } => tf(T::TitRevisandoN, &[("n", &miles(*vistos as u64))]),
        Fase::Copiando { total, .. } => {
            let n = d.ronda.as_ref().map(|r| r.archivos).unwrap_or(*total as u64);
            plural(n, T::TitCopiando1, T::TitCopiandoN)
        }
        Fase::Verificando { .. } => t(T::TitVerificando).into(),
        Fase::Inactiva if ind == Indicador::AlDia && e.total > 0 => t(T::TitAlDia).into(),
        Fase::Inactiva if e.total == 0 => t(T::TitVacio).into(),
        Fase::Inactiva => t(T::TitEspera).into(),
        Fase::Pausada => t(T::TitPausa).into(),
        Fase::SesionCaducada => t(T::TitSesion).into(),
        Fase::SinEspacio => t(T::TitSinEspacio).into(),
        Fase::DestinoAusente => t(T::TitSinDestino).into(),
        Fase::SinDestino => t(T::TitElige).into(),
        Fase::Error(_) => t(T::TitFallo).into(),
    };
    let fraccion_total = if d.bytes_total > 0 { d.bytes_verificados as f64 / d.bytes_total as f64 } else { 0.0 };
    let revisiones = || {
        let mut partes = Vec::new();
        if let Some(u) = d.ultimo_listado {
            partes.push(tf(T::UltimaRevision, &[("hace", &hace(ahora_unix - u))]));
        }
        if let Some(p) = d.proxima.filter(|&p| p > ahora_unix) {
            let clave = if partes.is_empty() { T::ProximaMay } else { T::ProximaMin };
            partes.push(tf(clave, &[("d", &duracion((p - ahora_unix) as u64))]));
        }
        if partes.is_empty() {
            t(T::NuncaRevisado).to_string()
        } else {
            partes.join(" · ")
        }
    };
    let (fraccion, linea) = match &e.fase {
        // Con un listado anterior, su tamaño sirve de total previsto y la barra avanza.
        Fase::Listando { vistos } => match d.listado_previsto.filter(|&p| p > 0) {
            Some(p) => {
                let v = *vistos as u64;
                let mut l = tf(T::ListandoPrevisto, &[("v", &miles(v)), ("p", &miles(p))]);
                if let (Some(r), true) = (ritmo_listado, v < p) {
                    l.push_str(" · ");
                    l.push_str(&restante(((p - v) as f64 / r) as u64));
                }
                (Some(v as f64 / p as f64), l)
            }
            None => (None, t(T::BuscandoNuevos).to_string()),
        },
        Fase::Copiando { hechas, total } => match &d.ronda {
            Some(r) if r.bytes > 0 => {
                let hecho = d.bytes_verificados.saturating_sub(r.bytes_base).min(r.bytes);
                let f = hecho as f64 / r.bytes as f64;
                let mut l = format!("{} % · {}", (f * 100.0).floor() as u64, par_bytes(hecho, r.bytes));
                if let (Some(v), true) = (vel, hecho < r.bytes) {
                    l.push_str(" · ");
                    l.push_str(&restante(((r.bytes - hecho) as f64 / v) as u64));
                }
                (Some(f), l)
            }
            _ => {
                let f = if *total > 0 { *hechas as f64 / *total as f64 } else { 0.0 };
                (Some(f), tf(T::ArchivosDe, &[("h", &miles(*hechas as u64)), ("t", &miles(*total as u64))]))
            }
        },
        Fase::Verificando { hechas, total } => (
            Some(if *total > 0 { *hechas as f64 / *total as f64 } else { 0.0 }),
            tf(T::RevisadosDe, &[("h", &miles(*hechas as u64)), ("t", &miles(*total as u64))]),
        ),
        Fase::Error(m) => (Some(fraccion_total), m.chars().take(90).collect()),
        _ => (Some(fraccion_total), revisiones()),
    };
    let linea_total = if e.total > 0 {
        tf(T::Fototeca, &[("v", &miles(e.verificadas)), ("t", &miles(e.total)), ("b", &par_bytes(d.bytes_verificados, d.bytes_total))])
    } else {
        t(T::FototecaVacia).into()
    };
    let pendientes = e.total.saturating_sub(e.verificadas);
    let ahora = match (&e.fase, &d.actual) {
        (Fase::Listando { .. }, _) if d.renovando_enlaces => t(T::AhoraRenovando).to_string(),
        (Fase::Listando { .. }, _) => t(T::AhoraConsultando).into(),
        (Fase::Copiando { .. }, Some(a)) => a.clone(),
        (Fase::Copiando { .. }, None) => t(T::AhoraPreparando).into(),
        (Fase::Verificando { .. }, _) => t(T::AhoraComprobando).into(),
        (Fase::Inactiva, _) if e.total == 0 => t(T::AhoraPrimera).into(),
        (Fase::Inactiva, _) if pendientes == 0 => t(T::AhoraNada).into(),
        (Fase::Inactiva | Fase::Pausada, _) if pendientes > 0 => plural(pendientes, T::Pendiente1, T::PendientesN),
        (Fase::Pausada, _) => t(T::AhoraNada).into(),
        (Fase::SesionCaducada, _) => t(T::AhoraSesion).into(),
        (Fase::SinEspacio, _) => t(T::AhoraEspacio).into(),
        (Fase::DestinoAusente, _) => t(T::AhoraDisco).into(),
        (Fase::SinDestino, _) => t(T::AhoraMenu).into(),
        (Fase::Error(_), _) => t(T::AhoraReintento).into(),
        (Fase::Inactiva, _) => t(T::AhoraNada).into(),
    };
    let (etiqueta_velocidad, velocidad_txt) = match &e.fase {
        Fase::Listando { .. } => (t(T::EtqRitmo), ritmo_listado.map(ritmo).unwrap_or_else(|| t(T::Midiendo).into())),
        Fase::Copiando { .. } => (t(T::EtqVelocidad), vel.map(velocidad).unwrap_or_else(|| t(T::Midiendo).into())),
        Fase::Verificando { .. } => (t(T::EtqVelocidad), t(T::SinDescargasComprueba).into()),
        _ => (
            t(T::EtqMedia),
            m.media_ultima_ronda.map(|v| tf(T::UltimaRonda, &[("v", &velocidad(v))])).unwrap_or_else(|| t(T::SinDescargas).into()),
        ),
    };
    Vista {
        indicador: ind,
        titulo,
        fraccion: fraccion.map(|f| f.clamp(0.0, 1.0)),
        linea,
        linea_total,
        ahora,
        etiqueta_velocidad,
        velocidad: velocidad_txt,
        listado: d.ultimo_listado.map(|u| hace(ahora_unix - u)).unwrap_or_else(|| t(T::Nunca).into()),
        errores: miles(d.con_error),
        principal: principal(e),
        liberar: None,
        boton_liberar: ind == Indicador::AlDia,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bandeja::estado::{Detalle, Ronda};
    use crate::registro::Progreso;

    #[test]
    fn formatos() {
        assert_eq!(par_bytes(108_400_000_000, 210_000_000_000), "108 de 210 GB");
        assert_eq!(par_bytes(1_500_000_000, 2_000_000_000), "1,5 de 2,0 GB");
        assert_eq!(par_bytes(0, 0), "0 de 0 B");
        assert_eq!(velocidad(12_400_000.0), "12,4 MB/s");
        assert_eq!(velocidad(850_000.0), "850 kB/s");
        assert_eq!(velocidad(2_000_000.0), "2,0 MB/s");
        assert_eq!(restante(30), "menos de 1 min restante");
        assert_eq!(restante(58 * 60), "~58 min restantes");
        assert_eq!(restante(2 * 3600 + 10 * 60), "~2 h 10 min restantes");
        assert_eq!(restante(3 * 86400), "~3 días restantes");
        assert_eq!(duracion(3 * 3600 + 55 * 60), "~3 h 55 min");
        assert_eq!(hace(10), "hace un momento");
        assert_eq!(hace(5 * 60 + 3), "hace 5 min");
        assert_eq!(hace(3 * 3600), "hace 3 h");
        assert_eq!(hace(86400 + 5), "hace 1 día");
        assert_eq!(hace(3 * 86400), "hace 3 días");
    }

    #[test]
    fn velocimetro_ventana_movil_y_reinicio() {
        let mut v = Velocimetro::default();
        v.registrar(0.0, 0);
        assert_eq!(v.bytes_por_seg(), None);
        v.registrar(1.0, 1_000_000);
        assert_eq!(v.bytes_por_seg(), Some(1_000_000.0));
        for s in 2..=20 {
            v.registrar(s as f64, 1_000_000 + (s as u64 - 1) * 3_000_000);
        }
        // Las muestras viejas (a 1 MB/s) ya salieron de la ventana.
        assert_eq!(v.bytes_por_seg(), Some(3_000_000.0));
        // Un contador que vuelve atrás reinicia la medida.
        v.registrar(21.0, 5);
        assert_eq!(v.bytes_por_seg(), None);
        // Nueva ronda: no arrastra la velocidad de la anterior.
        v.registrar(22.0, 9_000_005);
        assert!(v.bytes_por_seg().is_some());
        v.reiniciar();
        v.registrar(100.0, 9_000_005);
        assert_eq!(v.bytes_por_seg(), None);
    }

    fn progreso(verificadas: u64, partes: u64, bv: u64, bt: u64) -> Progreso {
        Progreso { partes, verificadas, con_error: 0, bytes_total: bt, bytes_verificados: bv, ultimo_listado: Some(1000) }
    }

    #[test]
    fn primera_copia_listado_copia_y_reposo() {
        const GB: u64 = 1_000_000_000;
        // Listando: barra indeterminada y contador de vistos.
        let mut e = EstadoUi::con_progreso(Fase::Listando { vistos: 1200 }, &progreso(0, 0, 0, 0));
        let v = vista(&e, Medidas::default(), 1000);
        assert_eq!((v.titulo.as_str(), v.fraccion, v.principal), ("Revisando iCloud… (1.200 vistos)", None, Principal::Pausar));
        assert_eq!(v.linea_total, "Fototeca: aún vacía");
        assert_eq!(v.indicador, Indicador::EnVivo);
        // Copiando: la barra sigue el trabajo de la ronda.
        e.fase = Fase::Copiando { hechas: 5, total: 210 };
        e.actualizar_progreso(&progreso(108, 210, 108 * GB, 210 * GB));
        e.detalle.ronda = Some(Ronda { archivos: 210, bytes: 210 * GB, bytes_base: 0 });
        e.detalle.actual = Some("IMG_1 (original)".into());
        let v = vista(&e, Medidas { bytes_por_seg: Some(29_310_000.0), ..Medidas::default() }, 1300);
        assert_eq!(v.titulo, "Copiando 210 nuevos");
        assert_eq!(v.linea, "51 % · 108 de 210 GB · ~58 min restantes");
        assert_eq!(v.linea_total, "Fototeca: 108 de 210 archivos · 108 de 210 GB");
        assert_eq!((v.ahora.as_str(), v.velocidad.as_str(), v.listado.as_str()), ("IMG_1 (original)", "29,3 MB/s", "hace 5 min"));
        // Reposo: al día, con la última y la próxima revisión; botón "Revisar ahora".
        let mut e = EstadoUi::con_progreso(Fase::Inactiva, &progreso(210, 210, 210 * GB, 210 * GB));
        e.detalle.proxima = Some(1000 + 4 * 3600);
        let v = vista(&e, Medidas { bytes_por_seg: Some(5.0), ..Medidas::default() }, 1000 + 5 * 60);
        assert_eq!((v.titulo.as_str(), v.indicador, v.velocidad.as_str()), ("Al día", Indicador::AlDia, "sin descargas recientes"));
        assert_eq!(v.linea, "Última revisión hace 5 min · próxima en ~3 h 55 min");
        assert_eq!((v.fraccion, v.principal), (Some(1.0), Principal::RevisarAhora));
    }

    #[test]
    fn segunda_ronda_con_fotos_nuevas_muestra_su_propio_avance() {
        const MB: u64 = 1_000_000;
        // Ya estaba todo copiado (1000 archivos, 5000 MB); aparecen 20 nuevos de 4 MB.
        let antes = EstadoUi::con_progreso(Fase::Inactiva, &progreso(1000, 1000, 5000 * MB, 5000 * MB));
        let listando = EstadoUi::con_progreso(Fase::Listando { vistos: 0 }, &progreso(1000, 1000, 5000 * MB, 5000 * MB));
        assert!(es_nueva_ronda(&antes, &listando));
        let mut e = EstadoUi::con_progreso(Fase::Copiando { hechas: 5, total: 20 }, &progreso(1005, 1020, 5020 * MB, 5080 * MB));
        assert!(!es_nueva_ronda(&listando, &e));
        e.detalle.ronda = Some(Ronda { archivos: 20, bytes: 80 * MB, bytes_base: 5000 * MB });
        let v = vista(&e, Medidas::default(), 2000);
        assert_eq!(v.titulo, "Copiando 20 nuevos");
        // La ronda va por el 25 % (no el 99,9 % de toda la fototeca)...
        assert_eq!(v.fraccion, Some(0.25));
        assert_eq!(v.linea, "25 % · 20 de 80 MB");
        // ...y los totales crecieron con lo nuevo.
        assert_eq!(v.linea_total, "Fototeca: 1.005 de 1.020 archivos · 5,0 de 5,1 GB");
        // Terminar y volver a reposo no es empezar ronda; volver a listar sí.
        let fin = EstadoUi::con_progreso(Fase::Inactiva, &progreso(1020, 1020, 5080 * MB, 5080 * MB));
        assert!(!es_nueva_ronda(&e, &fin));
        assert!(es_nueva_ronda(&e, &listando));
    }

    #[test]
    fn indicadores_y_boton_principal() {
        let base = progreso(1, 2, 1, 2);
        let con = |f: Fase| EstadoUi::con_progreso(f, &base);
        assert_eq!(indicador(&con(Fase::Inactiva)), Indicador::EnEspera);
        assert_eq!((indicador(&con(Fase::Pausada)), principal(&con(Fase::Pausada))), (Indicador::EnPausa, Principal::Reanudar));
        assert_eq!((indicador(&con(Fase::SesionCaducada)), principal(&con(Fase::SesionCaducada))), (Indicador::InicieSesion, Principal::IniciarSesion));
        assert_eq!(indicador(&con(Fase::SinEspacio)), Indicador::Atencion);
        assert_eq!(indicador(&con(Fase::Error("x".into()))), Indicador::Error);
        assert_eq!(principal(&con(Fase::Verificando { hechas: 1, total: 2 })), Principal::Pausar);
        let sin = EstadoUi { detalle: Detalle { ultimo_listado: None, ..Detalle::default() }, ..con(Fase::Inactiva) };
        assert_eq!(vista(&sin, Medidas::default(), 0).linea, "Todavía no se ha revisado iCloud");
    }

    #[test]
    fn relistado_tras_caducar_enlaces_se_explica() {
        let mut e = EstadoUi::con_progreso(Fase::Listando { vistos: 300 }, &progreso(1, 2, 1, 2));
        e.detalle.renovando_enlaces = true;
        let v = vista(&e, Medidas::default(), 1000);
        assert_eq!((v.titulo.as_str(), v.indicador), ("Renovando enlaces de iCloud…", Indicador::EnVivo));
        assert_eq!(e.texto(), "Renovando enlaces de iCloud… (300 vistos)");
    }

    #[test]
    fn listando_con_listado_anterior_da_barra_ritmo_y_tiempo() {
        let mut e = EstadoUi::con_progreso(Fase::Listando { vistos: 3225 }, &progreso(19_000, 19_570, 1, 2));
        e.detalle.listado_previsto = Some(19_570);
        let m = Medidas { elementos_por_seg: Some(1000.0 / 60.0 * 1.0), ..Medidas::default() };
        let v = vista(&e, m, 1000);
        assert_eq!(v.ahora, "Consultando tu fototeca en iCloud");
        assert_eq!((v.etiqueta_velocidad, v.velocidad.as_str()), ("Ritmo", "~1.000 elementos/min"));
        assert_eq!(v.linea, "3.225 de ~19.570 · ~17 min restantes");
        assert!((v.fraccion.unwrap() - 3225.0 / 19570.0).abs() < 1e-9);
        // Sin ritmo medido todavía.
        let v = vista(&e, Medidas::default(), 1000);
        assert_eq!((v.linea.as_str(), v.velocidad.as_str()), ("3.225 de ~19.570", "midiendo…"));
        // Primer listado (sin anterior): barra indeterminada.
        e.detalle.listado_previsto = None;
        let v = vista(&e, m, 1000);
        assert_eq!((v.fraccion, v.linea.as_str()), (None, "Buscando fotos y vídeos nuevos"));
        // Renovando enlaces.
        e.detalle.renovando_enlaces = true;
        assert_eq!(vista(&e, m, 1000).ahora, "Renovando enlaces de iCloud");
    }

    #[test]
    fn en_reposo_las_filas_tienen_contenido() {
        let al_dia = EstadoUi::con_progreso(Fase::Inactiva, &progreso(10, 10, 10, 10));
        let v = vista(&al_dia, Medidas::default(), 1000);
        assert_eq!((v.ahora.as_str(), v.etiqueta_velocidad, v.velocidad.as_str()), ("Nada pendiente", "Velocidad media", "sin descargas recientes"));
        let m = Medidas { media_ultima_ronda: Some(12_300_000.0), ..Medidas::default() };
        assert_eq!(vista(&al_dia, m, 1000).velocidad, "12,3 MB/s (última ronda)");
        let espera = EstadoUi::con_progreso(Fase::Inactiva, &progreso(7, 10, 7, 10));
        assert_eq!(vista(&espera, m, 1000).ahora, "3 archivos pendientes");
        let pausa = EstadoUi::con_progreso(Fase::Pausada, &progreso(9, 10, 9, 10));
        assert_eq!(vista(&pausa, m, 1000).ahora, "1 archivo pendiente");
        let vacio = EstadoUi::con_progreso(Fase::Inactiva, &progreso(0, 0, 0, 0));
        assert_eq!(vista(&vacio, m, 1000).ahora, "Esperando la primera revisión");
        for f in [Fase::SesionCaducada, Fase::SinEspacio, Fase::DestinoAusente, Fase::SinDestino, Fase::Error("x".into())] {
            let v = vista(&EstadoUi::con_progreso(f, &progreso(1, 2, 1, 2)), m, 1000);
            assert!(!v.ahora.contains('—') && !v.velocidad.contains('—'), "{v:?}");
        }
        // Copiando sin medida aún: "midiendo…", nunca una raya.
        let c = EstadoUi::con_progreso(Fase::Copiando { hechas: 0, total: 3 }, &progreso(0, 3, 0, 3));
        let v = vista(&c, Medidas::default(), 1000);
        assert_eq!((v.ahora.as_str(), v.velocidad.as_str()), ("Preparando la copia", "midiendo…"));
    }

    #[test]
    fn media_de_la_ronda() {
        let mut m = MediaRonda::default();
        assert_eq!(m.media(), None);
        m.empezar(10.0, 1_000);
        m.terminar(20.0, 101_000);
        assert_eq!(m.media(), Some(10_000.0));
        // Una ronda sin descargas no borra la media anterior.
        m.empezar(30.0, 101_000);
        m.terminar(40.0, 101_000);
        assert_eq!(m.media(), Some(10_000.0));
        // Terminar sin haber empezado no hace nada.
        m.terminar(50.0, 999_999);
        assert_eq!(m.media(), Some(10_000.0));
    }

    fn liberar(fase: FaseLiberar, hechos: u64, total: u64) -> ProgresoLiberar {
        ProgresoLiberar {
            fase,
            hechos,
            total,
            bytes_hechos: 16_100_000_000,
            bytes_total: 74_700_000_000,
            inicio_unix: 900_000,
            actualizado_unix: 1_000_000,
            borrados: hechos,
            fallidos: 0,
            dias: 30,
            por_seg: Some((total - hechos) as f64 / (14.0 * 60.0)),
            ..ProgresoLiberar::nuevo(30, 0)
        }
    }

    #[test]
    fn en_ingles() {
        use crate::i18n::{con_idioma, Idioma};
        con_idioma(Idioma::En, || {
            let p = liberar(FaseLiberar::Borrando, 2_350, 11_029);
            let v = vista_liberar(&p, 1_000_005).unwrap();
            assert_eq!((v.titulo.as_str(), v.subtitulo.as_str()), ("Freeing up iCloud space", "Deleting from iCloud…"));
            assert_eq!(v.linea, "2,350 / 11,029 · 21 % · 16.1 of 74.7 GB · ~14 min");
            assert_eq!(hace(3 * 3600), "3 h ago");
            let e = EstadoUi::con_progreso(Fase::Inactiva, &progreso(10, 10, 10, 10));
            assert_eq!(vista(&e, Medidas::default(), 1000).titulo, "Up to date");
        });
    }

    #[test]
    fn formatos_de_bytes_con_decimal() {
        assert_eq!(par_bytes_dec(16_100_000_000, 74_700_000_000), "16,1 de 74,7 GB");
        assert_eq!(par_bytes_dec(16_100_000_000, 174_700_000_000), "16 de 175 GB");
        assert_eq!(bytes_dec(74_700_000_000), "74,7 GB");
        assert_eq!(bytes_dec(1_260_000_000_000), "1,3 TB");
        assert_eq!(bytes_dec(512), "512 B");
    }

    #[test]
    fn liberar_borrando_con_barra_y_tiempo() {
        let p = liberar(FaseLiberar::Borrando, 2_350, 11_029);
        let v = vista_liberar(&p, 1_000_005).unwrap();
        assert_eq!((v.titulo.as_str(), v.subtitulo.as_str()), ("Liberando espacio en iCloud", "Borrando en iCloud…"));
        assert_eq!(v.linea, "2.350 / 11.029 · 21 % · 16,1 de 74,7 GB · ~14 min");
        assert_eq!(v.linea2, "Van a «Eliminado recientemente» · el PC no se toca");
        assert_eq!(v.tono, TonoLiberar::Activo);
        assert!((v.fraccion.unwrap() - 2350.0 / 11029.0).abs() < 1e-9);
        assert_eq!(texto_bandeja_liberar(&p, 1_000_005).unwrap(), "Liberando iCloud: 2.350 de 11.029 borrados (21 %) · ~14 min");
        let con_fallos = ProgresoLiberar { fallidos: 12, ..p.clone() };
        assert_eq!(vista_liberar(&con_fallos, 1_000_005).unwrap().linea2, "12 no se pudieron borrar · se reintentarán");
        // Sin noticias más de 2 min: interrumpido, y el tooltip deja de hablar de él.
        let v = vista_liberar(&p, 1_000_000 + 600).unwrap();
        assert_eq!((v.titulo.as_str(), v.subtitulo.as_str()), ("No se terminó de liberar espacio", "Se detuvo hace 10 min sin terminar"));
        assert_eq!((v.linea.as_str(), v.tono), ("Borrados 2.350 de 11.029 · 16,1 GB", TonoLiberar::Problema));
        assert_eq!(texto_bandeja_liberar(&p, 1_000_000 + 600), None);
        // Apple pidió esperar: cuenta atrás y sin tiempo restante.
        let espera = ProgresoLiberar { espera_hasta: Some(1_000_005 + 150), ..p.clone() };
        let v = vista_liberar(&espera, 1_000_005).unwrap();
        assert_eq!((v.subtitulo.as_str(), v.linea.as_str()), ("Apple pidió esperar · reanudando en ~3 min", "2.350 / 11.029 · 21 % · 16,1 de 74,7 GB"));
        assert_eq!(texto_bandeja_liberar(&espera, 1_000_005).unwrap(), "Liberando iCloud: Apple pidió esperar (2.350 de 11.029)");
        // En pausa: sin tiempo restante.
        let pausa = ProgresoLiberar { en_pausa: true, ..p.clone() };
        let v = vista_liberar(&pausa, 1_000_005).unwrap();
        assert_eq!((v.subtitulo.as_str(), v.linea.as_str()), ("En pausa", "2.350 / 11.029 · 21 % · 16,1 de 74,7 GB"));
        // Cancelado tras unos lotes.
        let c = ProgresoLiberar { fase: FaseLiberar::Terminado, cancelado: true, ..p };
        let v = vista_liberar(&c, 1_000_060).unwrap();
        assert_eq!((v.titulo.as_str(), v.linea.as_str()), ("Liberación cancelada", "Liberados 16,1 GB (2.350 de 11.029 elementos)"));
    }

    #[test]
    fn liberar_fases_previas() {
        let mut p = liberar(FaseLiberar::Listando, 3_225, 19_570);
        p.por_seg = None;
        let v = vista_liberar(&p, 1_000_000).unwrap();
        assert_eq!((v.subtitulo.as_str(), v.linea.as_str()), ("Consultando iCloud…", "3.225 de ~19.570 elementos"));
        assert_eq!(v.linea2, "Se conservan los últimos 30 días y la Fototeca Compartida");
        p.total = 0;
        let v = vista_liberar(&p, 1_000_000).unwrap();
        assert_eq!((v.fraccion, v.linea.as_str()), (None, "3.225 elementos vistos"));
        assert_eq!(texto_bandeja_liberar(&p, 1_000_000).unwrap(), "Liberando iCloud: consultando… (3.225 vistos)");
        let p = ProgresoLiberar { por_seg: None, ..liberar(FaseLiberar::Reverificando, 100, 400) };
        let v = vista_liberar(&p, 1_000_000).unwrap();
        assert_eq!((v.subtitulo.as_str(), v.linea.as_str()), ("Comprobando archivos en disco…", "100 / 400 · 25 % · 16,1 de 74,7 GB"));
        let p = liberar(FaseLiberar::Confirmando, 0, 11_029);
        let v = vista_liberar(&p, 1_000_000).unwrap();
        assert_eq!((v.subtitulo.as_str(), v.linea.as_str()), ("Esperando tu confirmación…", "Listos para borrar: 11.029 elementos · 74,7 GB"));
        assert_eq!(vista_liberar(&p, 1_000_000 + 600), None, "consola cerrada sin confirmar");
    }

    #[test]
    fn liberar_resumen_final_un_dia() {
        let mut p = liberar(FaseLiberar::Terminado, 11_029, 11_029);
        p.bytes_hechos = 74_700_000_000;
        p.por_seg = None;
        let v = vista_liberar(&p, 1_000_000 + 5 * 60).unwrap();
        assert_eq!((v.titulo.as_str(), v.subtitulo.as_str()), ("Espacio liberado en iCloud", "Terminado hace 5 min"));
        assert_eq!(v.linea, "Liberados 74,7 GB (11.029 elementos)");
        assert_eq!(v.linea2, "Vacía «Eliminado recientemente» para recuperarlo");
        assert_eq!((v.fraccion, v.tono), (Some(1.0), TonoLiberar::Hecho));
        assert_eq!(texto_bandeja_liberar(&p, 1_000_000 + 5 * 60), None);
        p.fallidos = 3;
        assert_eq!(vista_liberar(&p, 1_000_000 + 5 * 60).unwrap().subtitulo, "Terminado hace 5 min · 3 no se pudieron borrar");
        assert_eq!(vista_liberar(&p, 1_000_000 + 24 * 3600), None);
        let f = ProgresoLiberar { fase: FaseLiberar::Fallido, borrados: 0, error: Some("sesión caducada".into()), ..p };
        let v = vista_liberar(&f, 1_000_000 + 60).unwrap();
        assert_eq!((v.subtitulo.as_str(), v.linea.as_str()), ("Falló hace 1 min: sesión caducada", "No se borró nada de iCloud"));
    }
}
