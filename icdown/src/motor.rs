//! Una ronda: listar iCloud → registrar → copiar y verificar lo pendiente.
use crate::bitacora;
use crate::descarga::{self, ErrorDescarga};
use crate::disco;
use crate::fuente::{ErrorFuente, FuenteIcloud};
use crate::layout;
use crate::registro::{Registro, TrabajoParte};
use crate::verificar::{self, FalloVerificacion};
use reqwest::blocking::Client;
use crate::modelo::TipoParte;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinRonda {
    Completa,
    Relistar,
    SesionCaducada,
    SinEspacio,
    DestinoAusente,
    Cancelada,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResultadoRonda {
    pub fin: FinRonda,
    pub listados: usize,
    pub verificadas: usize,
    pub fallidas: usize,
    /// No se intentaron (o se cortaron) porque su enlace caducaba: siguen pendientes, sin penalizar.
    pub aplazadas: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Avance {
    Listando { vistos: usize },
    Copiando { hechas: usize, total: usize, actual: String },
    /// Se están pidiendo enlaces nuevos para los próximos `n` assets (sin relistar).
    RenovandoEnlaces { n: usize, hechas: usize, total: usize },
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpcionesRonda {
    pub concurrencia: usize,
    pub mes: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Listado {
    Completo(usize),
    Interrumpido(FinRonda, usize),
}

#[derive(Debug)]
pub enum ErrorParte {
    Descarga(ErrorDescarga),
    Verificacion(FalloVerificacion),
    Ruta(String),
    Disco(String),
    Registro(String),
    /// Estructura atípica y la segunda descarga no dio los mismos bytes que la primera.
    NoCoinciden,
    Cancelada,
}

/// Parte copiada y verificada. `atipica`: estructura no estándar en iCloud, confirmada por doble descarga.
#[derive(Debug)]
struct Verificada {
    sha: String,
    atipica: bool,
}

impl fmt::Display for ErrorParte {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorParte::Descarga(e) => write!(f, "{e}"),
            ErrorParte::Verificacion(e) => write!(f, "verificación: {e}"),
            ErrorParte::Ruta(m) => write!(f, "ruta: {m}"),
            ErrorParte::Disco(m) => write!(f, "disco: {m}"),
            ErrorParte::Registro(m) => write!(f, "registro: {m}"),
            ErrorParte::NoCoinciden => write!(f, "las dos descargas no coinciden"),
            ErrorParte::Cancelada => write!(f, "cancelada"),
        }
    }
}

pub fn ahora() -> i64 {
    chrono::Utc::now().timestamp()
}

fn resultado(fin: FinRonda, listados: usize) -> ResultadoRonda {
    ResultadoRonda { fin, listados, verificadas: 0, fallidas: 0, aplazadas: 0 }
}

/// No se empieza una descarga si su enlace caduca antes de `ahora + MARGEN + tamaño / RITMO_PRUDENTE`.
const MARGEN_CADUCIDAD: i64 = 120;
const RITMO_PRUDENTE: u64 = 2 * 1024 * 1024;
/// Un rechazo (401/403/404/410) con un listado más reciente que esto es un fallo real del asset.
pub const LISTADO_RECIENTE: i64 = 600;
/// Partes cuyos enlaces se renuevan de una vez (las próximas de la cola que van a caducar).
pub const LOTE_RENOVAR: usize = 200;
/// Con el enlace de una parte se renuevan también los de la cola que caducan en este plazo.
const HORIZONTE_RENOVAR: i64 = 600;
/// Veces que una misma parte puede pedir enlace nuevo en una ronda (descargas muy largas).
const MAX_RENOVACIONES_PARTE: u32 = 5;

/// Resultado de pedir un enlace nuevo para una parte.
#[allow(clippy::large_enum_variant)]
enum Renovado {
    Nuevo(TrabajoParte),
    /// Ya no está en iCloud (o el puente no la conoce): sigue pendiente hasta el próximo listado.
    Falta,
    /// No se pudo renovar (puente sin `refrescar`, error...): se vuelve a relistar como antes.
    NoDisponible,
}

/// Lo compartido entre los hilos para renovar enlaces sin relistar.
struct Renovador<'a> {
    /// Un solo hilo renueva a la vez (el puente atiende una petición cada vez).
    fuente: Mutex<&'a mut dyn FuenteIcloud>,
    roto: AtomicBool,
    /// Por parte: cuándo se renovó su enlace (para juzgar sus rechazos) y cuántas veces lo pidió ella.
    renovadas: Mutex<HashMap<(String, TipoParte), (i64, u32)>>,
    lotes: AtomicUsize,
    enlaces: AtomicUsize,
}

/// Caducidad (unix) del enlace de iCloud: parámetro `e` de la query. `None` si no lo trae.
pub fn caducidad_url(url: &str) -> Option<i64> {
    let sin_fragmento = url.split('#').next()?;
    let (_, query) = sin_fragmento.split_once('?')?;
    query.split('&').find_map(|kv| kv.strip_prefix("e=")).and_then(|v| v.parse().ok())
}

/// El enlace caducará antes de que dé tiempo a bajar `tamano` bytes a un ritmo prudente.
pub fn caduca_pronto(url: &str, tamano: u64, ahora: i64) -> bool {
    caducidad_url(url).is_some_and(|e| e <= ahora + MARGEN_CADUCIDAD + (tamano / RITMO_PRUDENTE) as i64)
}

/// Tras un rechazo del servidor: ¿se debe a que el enlace caducó (se pide otro sin penalizar)
/// o es un fallo real del asset (cuenta como intento, para no reintentar sin fin)?
/// `fiar_de_e` es falso cuando el reloj no cuadra con las caducidades (ver `ejecutar_ronda`).
pub fn rechazo_por_caducidad(url: &str, fin_listado: i64, ahora: i64, fiar_de_e: bool) -> bool {
    ahora - fin_listado > LISTADO_RECIENTE || (fiar_de_e && caducidad_url(url).is_some_and(|e| e <= ahora + MARGEN_CADUCIDAD))
}

/// Lista iCloud y lo vuelca al registro. Solo guarda la marca si el listado terminó bien.
pub fn listar_en_registro(
    registro: &Mutex<Registro>,
    fuente: &mut dyn FuenteIcloud,
    cancelar: &AtomicBool,
    avance: &(dyn Fn(Avance) + Sync),
) -> anyhow::Result<Listado> {
    let marca = ahora();
    let mut error_registro: Option<anyhow::Error> = None;
    let mut vistos = 0usize;
    let r = fuente.listar(&mut |asset| {
        if cancelar.load(Ordering::Relaxed) {
            return false;
        }
        if let Err(e) = registro.lock().unwrap().upsert_asset(&asset, marca) {
            error_registro = Some(e);
            return false;
        }
        vistos += 1;
        if vistos.is_multiple_of(25) {
            avance(Avance::Listando { vistos });
        }
        true
    });
    if let Some(e) = error_registro {
        return Err(e);
    }
    match r {
        Ok(_) => {
            registro.lock().unwrap().guardar_marca_lista(marca)?;
            avance(Avance::Listando { vistos });
            Ok(Listado::Completo(vistos))
        }
        Err(ErrorFuente::SesionCaducada) => Ok(Listado::Interrumpido(FinRonda::SesionCaducada, vistos)),
        Err(ErrorFuente::Cancelado) => Ok(Listado::Interrumpido(FinRonda::Cancelada, vistos)),
        Err(e) => Err(anyhow::anyhow!("listando iCloud: {e}")),
    }
}

pub fn ejecutar_ronda(
    destino: &Path,
    registro: &Mutex<Registro>,
    fuente: &mut dyn FuenteIcloud,
    http: &Client,
    op: &OpcionesRonda,
    cancelar: &AtomicBool,
    avance: &(dyn Fn(Avance) + Sync),
) -> anyhow::Result<ResultadoRonda> {
    if !destino.is_dir() {
        return Ok(resultado(FinRonda::DestinoAusente, 0));
    }
    let dir_datos = destino.join(".icdown");
    let listados = match listar_en_registro(registro, fuente, cancelar, avance)? {
        Listado::Completo(n) => n,
        Listado::Interrumpido(fin, n) => return Ok(resultado(fin, n)),
    };
    let fin_listado = ahora();

    let trabajos = registro.lock().unwrap().partes_pendientes(ahora(), op.mes.as_deref())?;
    // Si nada más listar todos los enlaces parecen ya caducados, relistar no arreglaría nada
    // (reloj desajustado o listado más largo que la vida de los enlaces): se ignora `e` y se intenta.
    let mut con_e = trabajos.iter().filter(|t| caducidad_url(&t.url).is_some()).peekable();
    let fiar_de_e = con_e.peek().is_none() || !con_e.all(|t| caduca_pronto(&t.url, t.tamano, fin_listado));
    if !fiar_de_e {
        bitacora::escribir(&dir_datos, "AVISO: todos los enlaces parecen caducados nada más listar (¿reloj desajustado?); se intentan igualmente");
    }
    let total = trabajos.len();
    let cola = Mutex::new(VecDeque::from(trabajos));
    let hechas = AtomicUsize::new(0);
    let verificadas = AtomicUsize::new(0);
    let fallidas = AtomicUsize::new(0);
    let aplazadas = AtomicUsize::new(0);
    let relistar = AtomicBool::new(false);
    // Un enlace caducó a media descarga y no se pudo renovar: lo que queda en la cola espera al próximo listado.
    let parar = AtomicBool::new(false);
    let sin_espacio = AtomicBool::new(false);
    let renovador = Renovador {
        fuente: Mutex::new(fuente),
        roto: AtomicBool::new(false),
        renovadas: Mutex::new(HashMap::new()),
        lotes: AtomicUsize::new(0),
        enlaces: AtomicUsize::new(0),
    };
    let aviso_renovar = |n: usize| avance(Avance::RenovandoEnlaces { n, hechas: hechas.load(Ordering::Relaxed), total });
    let renovar = |t: &TrabajoParte| renovar_enlaces(&renovador, t, &cola, registro, &dir_datos, cancelar, &aplazadas, &aviso_renovar);

    std::thread::scope(|s| {
        for _ in 0..op.concurrencia.max(1) {
            s.spawn(|| loop {
                if cancelar.load(Ordering::Relaxed) || sin_espacio.load(Ordering::Relaxed) || parar.load(Ordering::Relaxed) {
                    break;
                }
                let Some(mut t) = cola.lock().unwrap().pop_front() else { break };
                // Enlace a punto de caducar: se piden enlaces nuevos para este y los próximos de la
                // cola, sin relistar. Si no se puede, no se descarga ni cuenta como fallo: queda
                // pendiente y se sigue con los demás (los que aún valen se aprovechan antes de relistar).
                if fiar_de_e && caduca_pronto(&t.url, t.tamano, ahora()) {
                    match renovar(&t) {
                        Renovado::Nuevo(n) => t = n,
                        otro => {
                            if matches!(otro, Renovado::NoDisponible) {
                                relistar.store(true, Ordering::Relaxed);
                            }
                            aplazadas.fetch_add(1, Ordering::Relaxed);
                            hechas.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                    }
                }
                if !disco::cabe(destino, t.tamano) {
                    sin_espacio.store(true, Ordering::Relaxed);
                    break;
                }
                let actual = format!("{} ({})", t.nombre_base, t.tipo.como_str());
                avance(Avance::Copiando { hechas: hechas.load(Ordering::Relaxed), total, actual: actual.clone() });
                let renovada = renovador.renovadas.lock().unwrap().get(&(t.asset_id.clone(), t.tipo)).copied();
                match procesar_parte(destino, registro, http, &t, cancelar) {
                    Ok(v) => {
                        if let Err(e) = registro.lock().unwrap().marcar_verificada_con(&t.asset_id, t.tipo, &v.sha, v.atipica) {
                            bitacora::escribir(&dir_datos, &format!("ERROR registro {}: {e:#}", t.asset_id));
                        }
                        verificadas.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(ErrorParte::Cancelada) => break,
                    Err(ErrorParte::Descarga(ErrorDescarga::UrlCaducada(_)))
                        if rechazo_por_caducidad(&t.url, renovada.map_or(fin_listado, |r| r.0), ahora(), fiar_de_e) =>
                    {
                        // Se pide otro enlace y se reanuda (el `.part` se conserva); si no se puede, se aplaza.
                        let otro = if renovada.map_or(0, |r| r.1) < MAX_RENOVACIONES_PARTE { renovar(&t) } else { Renovado::Falta };
                        match otro {
                            Renovado::Nuevo(n) => {
                                cola.lock().unwrap().push_front(n);
                                continue;
                            }
                            Renovado::Falta => {
                                aplazadas.fetch_add(1, Ordering::Relaxed);
                            }
                            Renovado::NoDisponible => {
                                aplazadas.fetch_add(1, Ordering::Relaxed);
                                relistar.store(true, Ordering::Relaxed);
                                parar.store(true, Ordering::Relaxed);
                            }
                        }
                    }
                    Err(e) => {
                        let msg = match e {
                            // Enlace recién pedido y aun así rechazado: fallo real, con otro texto que
                            // "enlace de descarga caducado" para que relistar no lo perdone.
                            ErrorParte::Descarga(ErrorDescarga::UrlCaducada(c)) => {
                                relistar.store(true, Ordering::Relaxed);
                                format!("iCloud rechazó un enlace recién pedido (HTTP {c})")
                            }
                            otro => otro.to_string(),
                        };
                        bitacora::escribir(&dir_datos, &format!("FALLO {} {} ({}): {msg}", t.nombre_base, t.tipo.como_str(), t.asset_id));
                        let _ = registro.lock().unwrap().registrar_fallo(&t.asset_id, t.tipo, &msg, ahora());
                        fallidas.fetch_add(1, Ordering::Relaxed);
                    }
                }
                let n = hechas.fetch_add(1, Ordering::Relaxed) + 1;
                avance(Avance::Copiando { hechas: n, total, actual });
            });
        }
    });

    if parar.load(Ordering::Relaxed) && !cancelar.load(Ordering::Relaxed) && !sin_espacio.load(Ordering::Relaxed) {
        aplazadas.fetch_add(cola.lock().unwrap().len(), Ordering::Relaxed);
    }
    let (lotes, enlaces) = (renovador.lotes.load(Ordering::Relaxed), renovador.enlaces.load(Ordering::Relaxed));
    if lotes > 0 {
        bitacora::escribir(&dir_datos, &format!("ENLACES RENOVADOS: {enlaces} assets en {lotes} lotes, sin relistar"));
    }
    let aplazadas = aplazadas.load(Ordering::Relaxed);
    if aplazadas > 0 {
        bitacora::escribir(&dir_datos, &format!("ENLACES CADUCAN: {aplazadas} archivos aplazados hasta el próximo listado"));
    }

    let fin = if cancelar.load(Ordering::Relaxed) {
        FinRonda::Cancelada
    } else if sin_espacio.load(Ordering::Relaxed) {
        FinRonda::SinEspacio
    } else if relistar.load(Ordering::Relaxed) {
        FinRonda::Relistar
    } else {
        FinRonda::Completa
    };
    Ok(ResultadoRonda {
        fin,
        listados,
        verificadas: verificadas.load(Ordering::Relaxed),
        fallidas: fallidas.load(Ordering::Relaxed),
        aplazadas,
    })
}

/// Pide enlaces nuevos para `t` (ya sacado de la cola) y para las próximas partes de la cola que
/// caducan pronto, sin relistar. Actualiza el registro (sin tocar la marca del listado) y la cola.
/// Las partes de assets que ya no están salen de la cola y cuentan como aplazadas.
#[allow(clippy::too_many_arguments)]
fn renovar_enlaces(
    r: &Renovador,
    t: &TrabajoParte,
    cola: &Mutex<VecDeque<TrabajoParte>>,
    registro: &Mutex<Registro>,
    dir_datos: &Path,
    cancelar: &AtomicBool,
    aplazadas: &AtomicUsize,
    aviso: &dyn Fn(usize),
) -> Renovado {
    if r.roto.load(Ordering::Relaxed) {
        return Renovado::NoDisponible;
    }
    let mut fuente = r.fuente.lock().unwrap();
    if r.roto.load(Ordering::Relaxed) {
        return Renovado::NoDisponible;
    }
    let inicio = ahora();
    let mut ids: Vec<String> = vec![t.asset_id.clone()];
    {
        let c = cola.lock().unwrap();
        let mut vistos: HashSet<&str> = HashSet::from([t.asset_id.as_str()]);
        for j in c.iter().filter(|j| caduca_pronto(&j.url, j.tamano, inicio + HORIZONTE_RENOVAR)).take(LOTE_RENOVAR - 1) {
            if vistos.insert(j.asset_id.as_str()) {
                ids.push(j.asset_id.clone());
            }
        }
    }
    aviso(ids.len());
    let mut renovados: HashSet<String> = HashSet::new();
    let mut error_registro: Option<anyhow::Error> = None;
    let resultado = fuente.refrescar(&ids, &mut |a| {
        if cancelar.load(Ordering::Relaxed) {
            return false;
        }
        if let Err(e) = registro.lock().unwrap().actualizar_enlaces(&a) {
            error_registro = Some(e);
            return false;
        }
        renovados.insert(a.id);
        true
    });
    drop(fuente);
    let fallo = match (&resultado, &error_registro) {
        (_, Some(e)) => Some(format!("registro: {e:#}")),
        (Err(e), None) => Some(e.to_string()),
        // Nada renovado (p. ej. un puente que no conoce el último listado): mejor relistar.
        (Ok(_), None) if renovados.is_empty() => Some("el puente no devolvió ningún asset".into()),
        _ => None,
    };
    if let Some(m) = fallo {
        if !cancelar.load(Ordering::Relaxed) {
            bitacora::escribir(dir_datos, &format!("AVISO: no se pudieron renovar los enlaces ({m}); se volverá a listar"));
        }
        r.roto.store(true, Ordering::Relaxed);
        return Renovado::NoDisponible;
    }
    r.lotes.fetch_add(1, Ordering::Relaxed);
    r.enlaces.fetch_add(renovados.len(), Ordering::Relaxed);
    let faltan: HashSet<&str> = ids.iter().map(String::as_str).filter(|i| !renovados.contains(*i)).collect();
    if !faltan.is_empty() {
        bitacora::escribir(dir_datos, &format!("AVISO: {} assets ya no están en iCloud; esperan al próximo listado", faltan.len()));
    }
    // Trabajos frescos desde el registro (url, tamaño, checksum, ruta... ya actualizados).
    let mut frescos: HashMap<(String, TipoParte), TrabajoParte> = HashMap::new();
    {
        let reg = registro.lock().unwrap();
        for id in &renovados {
            for j in reg.trabajos_de(id).unwrap_or_default() {
                frescos.insert((j.asset_id.clone(), j.tipo), j);
            }
        }
    }
    {
        let mut renovadas = r.renovadas.lock().unwrap();
        for clave in frescos.keys() {
            renovadas.entry(clave.clone()).or_insert((0, 0)).0 = inicio;
        }
        if let Some(e) = renovadas.get_mut(&(t.asset_id.clone(), t.tipo)) {
            e.1 += 1;
        }
    }
    {
        let mut c = cola.lock().unwrap();
        let antes = c.len();
        c.retain(|j| !faltan.contains(j.asset_id.as_str()));
        aplazadas.fetch_add(antes - c.len(), Ordering::Relaxed);
        for j in c.iter_mut() {
            if let Some(n) = frescos.get(&(j.asset_id.clone(), j.tipo)) {
                *j = n.clone();
            }
        }
    }
    match frescos.remove(&(t.asset_id.clone(), t.tipo)) {
        // Recién renovado es lo más fresco que hay: se intenta aunque el archivo sea tan grande que el
        // enlace pueda caducar antes de terminar (la descarga se reanudaría con otro enlace).
        Some(n) if caducidad_url(&n.url).is_none_or(|e| e > ahora() + MARGEN_CADUCIDAD) => Renovado::Nuevo(n),
        _ => Renovado::Falta,
    }
}

fn procesar_parte(destino: &Path, registro: &Mutex<Registro>, http: &Client, t: &TrabajoParte, cancelar: &AtomicBool) -> Result<Verificada, ErrorParte> {
    let rel = match &t.ruta {
        Some(r) => r.clone(),
        None => {
            let mut reg = registro.lock().unwrap();
            let rel = {
                let consulta = &*reg;
                layout::elegir_ruta(&t.nombre_base, &t.asset_id, &t.fecha, t.tipo, &t.extension, &mut |clave| {
                    consulta.ruta_ocupada(clave).unwrap_or(true)
                        || layout::existe_sin_mayusculas(destino, clave)
                        || layout::existe_sin_mayusculas(destino, &format!("{clave}.part"))
                })
                .map_err(|e| ErrorParte::Ruta(e.to_string()))?
            };
            reg.asignar_ruta(&t.asset_id, t.tipo, &rel).map_err(|e| ErrorParte::Ruta(e.to_string()))?;
            rel
        }
    };
    let final_ = layout::en_disco(destino, &rel);
    if let Some(p) = final_.parent() {
        std::fs::create_dir_all(p).map_err(|e| ErrorParte::Disco(e.to_string()))?;
    }
    // Recuperación tras un corte entre "renombrar" y "anotar como verificado": solo se adopta
    // el archivo final si es exactamente el `.part` que se verificó (mismo SHA-256 anotado).
    if let Some(esperado) = &t.sha_pendiente {
        if final_.exists() {
            let v = if t.estructura_atipica {
                verificar::verificar_archivo_sin_estructura(&final_, t.tamano)
            } else {
                verificar::verificar_archivo(&final_, t.tamano, &t.extension)
            };
            if let Ok(sha) = v {
                if &sha == esperado {
                    return Ok(Verificada { sha, atipica: t.estructura_atipica });
                }
            }
        }
    }
    let part = descarga::ruta_part(&final_);
    let identidad = format!("{}:{}", t.tamano, t.checksum);
    descarga::descargar(http, &t.url, &part, t.tamano, &identidad, cancelar).map_err(error_descarga)?;
    let (sha, atipica) = match verificar::verificar_archivo(&part, t.tamano, &t.extension) {
        Ok(sha) => (sha, false),
        // Tamaño exacto pero estructura no estándar: puede ser el original tal como lo guarda Apple
        // (p. ej. JPEG sin EOI guardados desde WhatsApp). Se acepta solo si una segunda descarga
        // independiente da exactamente los mismos bytes.
        Err(FalloVerificacion::Estructura(_)) => (confirmar_por_doble_descarga(destino, http, t, &part, &final_, &identidad, cancelar)?, true),
        Err(f) => {
            descarga::borrar_part(&part);
            return Err(ErrorParte::Verificacion(f));
        }
    };
    registro
        .lock()
        .unwrap()
        .anotar_sha_pendiente(&t.asset_id, t.tipo, &sha, atipica)
        .map_err(|e| ErrorParte::Registro(e.to_string()))?;
    std::fs::rename(&part, &final_).map_err(|e| ErrorParte::Disco(e.to_string()))?;
    let _ = std::fs::remove_file(descarga::ruta_meta(&part));
    Ok(Verificada { sha, atipica })
}

fn error_descarga(e: ErrorDescarga) -> ErrorParte {
    match e {
        ErrorDescarga::Cancelada => ErrorParte::Cancelada,
        otro => ErrorParte::Descarga(otro),
    }
}

/// `<final>.part2`: segunda descarga independiente para confirmar un archivo de estructura atípica.
pub fn ruta_part2(final_: &Path) -> std::path::PathBuf {
    let mut s = final_.as_os_str().to_owned();
    s.push(".part2");
    s.into()
}

/// Descarga otra vez la parte desde el byte 0 en `<final>.part2` (petición nueva, sin reanudar) y
/// compara SHA-256. Iguales: devuelve el SHA (el `.part` queda listo para renombrar). Distintas: borra
/// las dos. Si la segunda descarga falla (red, enlace caducado, cancelada), el `.part` se conserva
/// para el siguiente intento y el `.part2` se descarta.
fn confirmar_por_doble_descarga(
    destino: &Path,
    http: &Client,
    t: &TrabajoParte,
    part: &Path,
    final_: &Path,
    identidad: &str,
    cancelar: &AtomicBool,
) -> Result<String, ErrorParte> {
    let part2 = ruta_part2(final_);
    descarga::borrar_part(&part2);
    let segunda = descarga::descargar(http, &t.url, &part2, t.tamano, &format!("{identidad}:2"), cancelar);
    let comparar = segunda.map_err(error_descarga).and_then(|_| {
        let a = verificar::verificar_archivo_sin_estructura(part, t.tamano).map_err(ErrorParte::Verificacion)?;
        let b = verificar::verificar_archivo_sin_estructura(&part2, t.tamano).map_err(ErrorParte::Verificacion)?;
        Ok((a, b))
    });
    descarga::borrar_part(&part2);
    let (a, b) = comparar?;
    let dir_datos = destino.join(".icdown");
    let quien = format!("{} ({})", t.nombre_base, t.tipo.como_str());
    if a == b {
        bitacora::escribir(&dir_datos, &format!("ATIPICO {quien}: estructura no estándar en iCloud; confirmado por doble descarga"));
        Ok(a)
    } else {
        bitacora::escribir(&dir_datos, &format!("ATIPICO {quien}: estructura no estándar en iCloud; las dos descargas no coinciden"));
        descarga::borrar_part(part);
        Err(ErrorParte::NoCoinciden)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://cvws-h2.icloud-content.com/B/AbC/${f}?o=AtX&v=1&x=3&a=CAog&e=1762538400&fl=&r=d1&k=$k&ckc=com.apple.photos.cloud";

    #[test]
    fn lee_la_caducidad_del_enlace() {
        assert_eq!(caducidad_url(URL), Some(1_762_538_400));
        assert_eq!(caducidad_url("http://x/a?e=5"), Some(5));
        assert_eq!(caducidad_url("http://x/a?le=5&e=7#frag"), Some(7));
        assert_eq!(caducidad_url("http://x/a"), None);
        assert_eq!(caducidad_url("http://x/a?e="), None);
        assert_eq!(caducidad_url("http://x/a?v=1"), None);
    }

    #[test]
    fn caduca_pronto_cuenta_margen_y_tamano() {
        let e = 1_762_538_400;
        assert!(!caduca_pronto(URL, 1000, e - 121));
        assert!(caduca_pronto(URL, 1000, e - 120));
        // 200 MiB a 2 MiB/s = 100 s más de margen.
        assert!(caduca_pronto(URL, 200 * 1024 * 1024, e - 220));
        assert!(!caduca_pronto(URL, 200 * 1024 * 1024, e - 221));
        assert!(!caduca_pronto("http://x/sin-caducidad", u64::MAX / 2, 0), "sin `e` no se aplaza nunca");
    }

    #[test]
    fn rechazo_reciente_es_fallo_real_y_tardio_es_caducidad() {
        let e = 1_762_538_400;
        // Listado reciente y `e` lejos: el asset está roto de verdad.
        assert!(!rechazo_por_caducidad(URL, e - 3000, e - 2500, true));
        // `e` alcanzado (o a punto): caducidad.
        assert!(rechazo_por_caducidad(URL, e - 500, e - 60, true));
        assert!(rechazo_por_caducidad(URL, e - 500, e + 10, true));
        // Listado de hace más de 10 min: caducidad aunque no haya `e`.
        assert!(rechazo_por_caducidad("http://x/a", 1000, 1000 + 601, true));
        assert!(!rechazo_por_caducidad("http://x/a", 1000, 1000 + 600, true));
        // Sin fiarse de `e` (reloj desajustado) solo cuenta la antigüedad del listado.
        assert!(!rechazo_por_caducidad(URL, e + 5000, e + 5100, false));
        assert!(rechazo_por_caducidad(URL, e + 5000, e + 5700, false));
    }
}
