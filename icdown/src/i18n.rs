//! Textos de la interfaz en español e inglés, y formato de números y fechas según el idioma.
//! El idioma sale de `ICDOWN_IDIOMA` (`es`/`en`) o de `LC_ALL`/`LANG`; si no es español, inglés.
use std::cell::Cell;
use std::fmt::Display;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Idioma {
    Es,
    En,
}

impl Idioma {
    pub fn desde_codigo(s: &str) -> Option<Idioma> {
        match s.trim().to_lowercase().get(..2) {
            Some("es") => Some(Idioma::Es),
            Some("en") => Some(Idioma::En),
            _ => None,
        }
    }

    /// Locale POSIX (`es_ES.UTF-8`, `en_US`, `C`…): `es*` → español, el resto inglés.
    pub fn desde_locale(l: &str) -> Idioma {
        if l.trim().to_lowercase().starts_with("es") {
            Idioma::Es
        } else {
            Idioma::En
        }
    }
}

thread_local! {
    static FORZADO: Cell<Option<Idioma>> = const { Cell::new(None) };
}

fn detectar() -> Idioma {
    if let Some(i) = std::env::var("ICDOWN_IDIOMA").ok().and_then(|s| Idioma::desde_codigo(&s)) {
        return i;
    }
    // Las pruebas de la biblioteca se escriben en español, sea cual sea el idioma de la máquina.
    if cfg!(test) {
        return Idioma::Es;
    }
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(v) = std::env::var(var) {
            if !v.is_empty() {
                return Idioma::desde_locale(&v);
            }
        }
    }
    Idioma::En
}

pub fn idioma() -> Idioma {
    static GLOBAL: OnceLock<Idioma> = OnceLock::new();
    FORZADO.with(|f| f.get()).unwrap_or_else(|| *GLOBAL.get_or_init(detectar))
}

/// Ejecuta `f` con otro idioma en este hilo (para pruebas y capturas).
pub fn con_idioma<R>(i: Idioma, f: impl FnOnce() -> R) -> R {
    let antes = FORZADO.with(|c| c.replace(Some(i)));
    let r = f();
    FORZADO.with(|c| c.set(antes));
    r
}

macro_rules! textos {
    ($($clave:ident => $es:expr, $en:expr;)*) => {
        /// Clave de un texto de la interfaz.
        #[allow(non_camel_case_types)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum T { $($clave),* }

        impl T {
            pub const TODAS: &'static [T] = &[$(T::$clave),*];

            pub fn en(self, i: Idioma) -> &'static str {
                match (self, i) {
                    $((T::$clave, Idioma::Es) => $es, (T::$clave, Idioma::En) => $en,)*
                }
            }
        }
    };
}

/// Texto en el idioma actual.
pub fn t(c: T) -> &'static str {
    c.en(idioma())
}

/// Texto con huecos `{nombre}` rellenados.
pub fn tf(c: T, args: &[(&str, &dyn Display)]) -> String {
    let mut s = t(c).to_string();
    for (k, v) in args {
        s = s.replace(&format!("{{{k}}}"), &v.to_string());
    }
    s
}

/// 12430 → "12.430" (español) o "12,430" (inglés).
pub fn miles(n: u64) -> String {
    let sep = if idioma() == Idioma::Es { '.' } else { ',' };
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(sep);
        }
        out.push(c);
    }
    out
}

/// Número con un decimal: "16,1" (español) o "16.1" (inglés).
pub fn decimal1(x: f64) -> String {
    let s = format!("{x:.1}");
    if idioma() == Idioma::Es {
        s.replace('.', ",")
    } else {
        s
    }
}

/// Mes abreviado y año de una fecha unix (hora local): "mar 2019" / "Mar 2019".
pub fn mes_anio(unix: i64) -> String {
    use chrono::Datelike;
    const ES: [&str; 12] = ["ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sept", "oct", "nov", "dic"];
    const EN: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let f = chrono::DateTime::from_timestamp(unix, 0).unwrap_or_default().with_timezone(&chrono::Local);
    let m = f.month0() as usize;
    format!("{} {}", if idioma() == Idioma::Es { ES[m] } else { EN[m] }, f.year())
}

textos! {
    // Panel: cabecera y botones
    IndVivo => "En vivo", "Live";
    IndAlDia => "Al día", "Up to date";
    IndEspera => "En espera", "Waiting";
    IndPausa => "En pausa", "Paused";
    IndSesion => "Inicia sesión", "Sign in";
    IndAtencion => "Atención", "Attention";
    IndError => "Error", "Error";
    BotPausar => "Pausar", "Pause";
    BotReanudar => "Reanudar", "Resume";
    BotRevisar => "Revisar ahora", "Check now";
    BotSesion => "Iniciar sesión", "Sign in";
    BotCarpeta => "Abrir carpeta", "Open folder";
    BotLiberar => "Liberar espacio", "Free up space";
    FilaAhora => "Ahora", "Now";
    FilaListado => "Último listado de iCloud", "Last iCloud scan";
    FilaErrores => "Errores", "Errors";
    // Duraciones
    MenosDeUnMin => "menos de 1 min", "less than 1 min";
    DurMin => "~{m} min", "~{m} min";
    DurH => "~{h} h", "~{h} h";
    DurHMin => "~{h} h {m} min", "~{h} h {m} min";
    DurDias => "~{d} días", "~{d} days";
    RestanteMenos => "menos de 1 min restante", "less than 1 min left";
    Restantes => "{d} restantes", "{d} left";
    HaceMomento => "hace un momento", "just now";
    HaceMin => "hace {n} min", "{n} min ago";
    HaceH => "hace {n} h", "{n} h ago";
    HaceDia => "hace 1 día", "1 day ago";
    HaceDias => "hace {n} días", "{n} days ago";
    ParBytes => "{a} de {b} {u}", "{a} of {b} {u}";
    Ritmo => "~{n} elementos/min", "~{n} items/min";
    // Panel: títulos
    TitRenovando => "Renovando enlaces de iCloud…", "Renewing iCloud links…";
    TitRevisando => "Revisando iCloud…", "Checking iCloud…";
    TitRenovandoN => "Renovando enlaces de iCloud… ({n} vistos)", "Renewing iCloud links… ({n} seen)";
    TitRevisandoN => "Revisando iCloud… ({n} vistos)", "Checking iCloud… ({n} seen)";
    TitCopiando1 => "Copiando {n} nuevo", "Copying {n} new item";
    TitCopiandoN => "Copiando {n} nuevos", "Copying {n} new items";
    TitVerificando => "Verificando tu copia", "Verifying your copy";
    TitAlDia => "Al día", "Up to date";
    TitVacio => "Aún no hay nada copiado", "Nothing copied yet";
    TitEspera => "En espera", "Waiting";
    TitPausa => "Copia en pausa", "Copy paused";
    TitSesion => "Tienes que iniciar sesión en iCloud", "You need to sign in to iCloud";
    TitSinEspacio => "No queda espacio en el disco", "The disk is full";
    TitSinDestino => "No encuentro la carpeta destino", "Can't find the destination folder";
    TitElige => "Elige una carpeta destino", "Choose a destination folder";
    TitFallo => "Algo ha fallado", "Something went wrong";
    UltimaRevision => "Última revisión {hace}", "Last check {hace}";
    ProximaMay => "Próxima en {d}", "Next in {d}";
    ProximaMin => "próxima en {d}", "next in {d}";
    NuncaRevisado => "Todavía no se ha revisado iCloud", "iCloud hasn't been checked yet";
    ListandoPrevisto => "{v} de ~{p}", "{v} of ~{p}";
    BuscandoNuevos => "Buscando fotos y vídeos nuevos", "Looking for new photos and videos";
    ArchivosDe => "{h} de {t} archivos", "{h} of {t} files";
    RevisadosDe => "{h} de {t} archivos revisados", "{h} of {t} files checked";
    Fototeca => "Fototeca: {v} de {t} archivos · {b}", "Library: {v} of {t} files · {b}";
    FototecaVacia => "Fototeca: aún vacía", "Library: still empty";
    AhoraRenovando => "Renovando enlaces de iCloud", "Renewing iCloud links";
    AhoraConsultando => "Consultando tu fototeca en iCloud", "Reading your iCloud library";
    AhoraPreparando => "Preparando la copia", "Preparing the copy";
    AhoraComprobando => "Comprobando los archivos copiados", "Checking the copied files";
    AhoraPrimera => "Esperando la primera revisión", "Waiting for the first check";
    AhoraNada => "Nada pendiente", "Nothing pending";
    Pendiente1 => "{n} archivo pendiente", "{n} file pending";
    PendientesN => "{n} archivos pendientes", "{n} files pending";
    AhoraSesion => "Esperando a que inicies sesión", "Waiting for you to sign in";
    AhoraEspacio => "Libera espacio en el disco destino", "Free up space on the destination disk";
    AhoraDisco => "Conecta el disco o elige otra carpeta", "Connect the disk or choose another folder";
    AhoraMenu => "Elige la carpeta en el menú del icono", "Choose the folder from the icon menu";
    AhoraReintento => "Se reintentará en la próxima ronda", "It will retry on the next round";
    EtqRitmo => "Ritmo", "Pace";
    EtqVelocidad => "Velocidad", "Speed";
    EtqMedia => "Velocidad media", "Average speed";
    Midiendo => "midiendo…", "measuring…";
    SinDescargasComprueba => "sin descargas (solo comprueba)", "no downloads (checking only)";
    UltimaRonda => "{v} (última ronda)", "{v} (last round)";
    SinDescargas => "sin descargas recientes", "no recent downloads";
    Nunca => "nunca", "never";
    // Panel: liberar espacio
    LibTitulo => "Liberando espacio en iCloud", "Freeing up iCloud space";
    LibTituloHecho => "Espacio liberado en iCloud", "iCloud space freed up";
    LibTituloProblema => "No se terminó de liberar espacio", "Freeing up space didn't finish";
    LibTituloCancelado => "Liberación cancelada", "Freeing up space cancelled";
    LibConsultando => "Consultando iCloud…", "Checking iCloud…";
    LibComprobando => "Comprobando archivos en disco…", "Checking files on disk…";
    LibBorrando => "Borrando en iCloud…", "Deleting from iCloud…";
    LibPausa => "En pausa", "Paused";
    LibEsperaApple => "Apple pidió esperar · reanudando en {d}", "Apple asked to wait · resuming in {d}";
    LibEsperaAppleYa => "Apple pidió esperar · reanudando…", "Apple asked to wait · resuming…";
    LibEsperando => "Esperando tu confirmación…", "Waiting for your confirmation…";
    LibTerminado => "Terminado {hace}", "Finished {hace}";
    LibNoBorrados => " · {n} no se pudieron borrar", " · {n} couldn't be deleted";
    LibCancelada => "Cancelada {hace}", "Cancelled {hace}";
    LibFalloMotivo => "Falló {hace}: {e}", "Failed {hace}: {e}";
    LibFallo => "Falló {hace}", "Failed {hace}";
    LibDetenido => "Se detuvo {hace} sin terminar", "Stopped {hace} before finishing";
    LibListandoDe => "{v} de ~{t} elementos{eta}", "{v} of ~{t} items{eta}";
    LibVistos => "{n} elementos vistos", "{n} items seen";
    LibListos => "Listos para borrar: {n} elementos · {b}", "Ready to delete: {n} items · {b}";
    LibLiberados1 => "Liberados {b} ({n} elemento)", "Freed up {b} ({n} item)";
    LibLiberadosN => "Liberados {b} ({n} elementos)", "Freed up {b} ({n} items)";
    LibLiberadosParcial => "Liberados {b} ({h} de {t} elementos)", "Freed up {b} ({h} of {t} items)";
    LibBorradosDe => "Borrados {h} de {t} · {b}", "Deleted {h} of {t} · {b}";
    LibNadaBorrado => "No se borró nada de iCloud", "Nothing was deleted from iCloud";
    LibFallidosReintento => "{n} no se pudieron borrar · se reintentarán", "{n} couldn't be deleted · they'll be retried";
    LibVanAPapelera => "Van a «Eliminado recientemente» · el PC no se toca", "They go to “Recently Deleted” · your PC isn't touched";
    LibConserva => "Se conservan los últimos {d} días y la Fototeca Compartida", "Keeps the last {d} days and the Shared Library";
    LibConfirma => "Confirma para empezar a borrar", "Confirm to start deleting";
    LibVacia => "Vacía «Eliminado recientemente» para recuperarlo", "Empty “Recently Deleted” to get it back";
    LibReintentar => "Puedes volver a intentarlo desde el menú del icono", "You can try again from the icon menu";
    TipConfirmacion => "Liberando iCloud: esperando tu confirmación", "Freeing iCloud: waiting for your confirmation";
    TipConsultando => "Liberando iCloud: consultando… ({n} vistos)", "Freeing iCloud: checking… ({n} seen)";
    TipComprobando => "Liberando iCloud: comprobando {h} de {t}", "Freeing iCloud: checking {h} of {t}";
    TipBorrando => "Liberando iCloud: {h} de {t} borrados ({p} %){eta}", "Freeing iCloud: {h} of {t} deleted ({p} %){eta}";
    TipEsperaApple => "Liberando iCloud: Apple pidió esperar ({h} de {t})", "Freeing iCloud: Apple asked to wait ({h} of {t})";
    TipEnPausa => "Liberando iCloud: en pausa ({h} de {t})", "Freeing iCloud: paused ({h} of {t})";
    // Estado (menú y tooltip)
    EstAlDia => "Al día — {p}", "Up to date — {p}";
    EstEspera => "En espera — {p}", "Waiting — {p}";
    EstCopiando => "Copiando {h} de {t}…{pct}", "Copying {h} of {t}…{pct}";
    EstDelTotal => " · {p} % del total", " · {p} % of total";
    EstVerificando => "Verificando {h} de {t}…", "Verifying {h} of {t}…";
    EstPausa => "En pausa — {p}", "Paused — {p}";
    EstSesion => "Inicia sesión en iCloud (menú → Iniciar sesión)", "Sign in to iCloud (menu → Sign in)";
    EstSinEspacio => "Sin espacio en el disco destino", "No space on the destination disk";
    EstSinDestino => "No encuentro la carpeta destino (¿disco desconectado?)", "Can't find the destination folder (disk disconnected?)";
    EstElige => "Elige una carpeta destino (menú → Cambiar destino)", "Choose a destination folder (menu → Change destination)";
    EstError => "Error: {m}", "Error: {m}";
    EstNada => "nada copiado aún", "nothing copied yet";
    EstArchivos => "{v} de {t} archivos ({p}%)", "{v} of {t} files ({p}%)";
    // Menú del icono
    MenuIniciando => "icdown — iniciando…", "icdown — starting…";
    MenuCopiar => "Copiar ahora", "Copy now";
    MenuPausar => "Pausar", "Pause";
    MenuReanudar => "Reanudar", "Resume";
    MenuVerificar => "Verificar todo", "Verify everything";
    MenuLiberar => "Liberar espacio en iCloud…", "Free up iCloud space…";
    MenuAbrir => "Abrir carpeta de fotos", "Open photos folder";
    MenuDestino => "Cambiar destino…", "Change destination…";
    MenuSesion => "Iniciar sesión en iCloud…", "Sign in to iCloud…";
    MenuAuto => "Iniciar con la sesión", "Start at login";
    MenuBuscarAct => "Buscar actualizaciones", "Check for updates";
    MenuActualizarA => "Reiniciar y actualizar a {v}", "Restart and update to {v}";
    MenuActAuto => "Actualizar automáticamente", "Update automatically";
    MenuSalir => "Salir", "Quit";
    DlgDestino => "icdown: elige dónde guardar las fotos", "icdown: choose where to save the photos";
    DlgNuevoDestino => "icdown: nueva carpeta destino", "icdown: new destination folder";
    // Avisos
    AvCopiados => "{n} archivos nuevos copiados y verificados", "{n} new files copied and verified";
    AvLoginCmd => "Abre una terminal y ejecuta `icdown login` para iniciar sesión en iCloud.", "Open a terminal and run `icdown login` to sign in to iCloud.";
    AvSesion => "La sesión de iCloud caducó. Menú del icono → Iniciar sesión en iCloud…", "Your iCloud session expired. Icon menu → Sign in to iCloud…";
    AvSinEspacio => "No queda espacio en {d}", "No space left on {d}";
    AvCopiaEnMarcha => "Hay una copia en marcha; vuelve a intentarlo luego.", "A copy is running; try again later.";
    AvVerificado => "Verificación completa: {n} archivos en perfecto estado", "Verification complete: {n} files in perfect condition";
    AvDanados => "{n} archivos dañados o ausentes; se volverán a descargar", "{n} files damaged or missing; they'll be downloaded again";
    AvNoVerificado => "No se pudo verificar: {e}", "Couldn't verify: {e}";
    AvRenovando => "Renovando enlaces ({n})…", "Renewing links ({n})…";
    AvLiberado => "Espacio liberado: {b} ({n} elementos). Vacía «Eliminado recientemente» en el iPhone para recuperarlo.", "Space freed up: {b} ({n} items). Empty “Recently Deleted” on your iPhone to get it back.";
    AvLiberarFallo => "No se pudo liberar espacio: {e}", "Couldn't free up space: {e}";
    AvActTitulo => "Actualización de iCopy", "iCopy update";
    AvActLista => "iCopy {v} está lista. Se instalará cuando no esté copiando.", "iCopy {v} is ready. It will install when nothing is being copied.";
    AvActListaManual => "iCopy {v} está lista. Menú del icono → Reiniciar y actualizar.", "iCopy {v} is ready. Icon menu → Restart and update.";
    AvActAlDia => "Tienes la última versión ({v}).", "You have the latest version ({v}).";
    AvActError => "No se pudo buscar la actualización: {e}", "Couldn't check for updates: {e}";
    AvActFalloInstalar => "No se pudo instalar iCopy {v}. La copia sigue con la versión actual.", "Couldn't install iCopy {v}. Copying continues with the current version.";
    AvActLiberando =>"Se actualizará al terminar de liberar espacio.", "It will update after freeing up space finishes.";
    AvRestaurados =>"Restaurados {n} elementos en iCloud", "Restored {n} items in iCloud";
    // Inicio de sesión
    LogVentana => "Iniciar sesión · icdown", "Sign in · icdown";
    LogTitulo => "Inicia sesión en iCloud", "Sign in to iCloud";
    LogSubCred => "Usa tu Apple ID. La contraseña solo se envía a Apple; icdown no la guarda.", "Use your Apple ID. The password is only sent to Apple; icdown doesn't store it.";
    LogSubCodigo => "Escribe el código de 6 dígitos que apareció en tu iPhone.", "Enter the 6-digit code that appeared on your iPhone.";
    LogComprobando => "Comprobando tu sesión…", "Checking your session…";
    LogComprobandoSub => "Mirando si la sesión de {id} sigue activa…", "Checking whether {id}'s session is still active…";
    LogYa => "Ya tienes sesión iniciada", "You're already signed in";
    LogYaSub => "Sesión activa como {id}. icdown te avisará cuando caduque.", "Signed in as {id}. icdown will let you know when it expires.";
    LogAppleId => "Apple ID", "Apple ID";
    LogContrasena => "Contraseña", "Password";
    LogCodigo => "Código de verificación", "Verification code";
    LogCancelar => "Cancelar", "Cancel";
    LogEntrar => "Entrar", "Sign in";
    LogVerificar => "Verificar", "Verify";
    LogCambiar => "Cambiar de cuenta", "Use another account";
    LogCerrar => "Cerrar", "Close";
    LogFaltaId => "Escribe tu Apple ID.", "Enter your Apple ID.";
    LogFaltaPw => "Escribe la contraseña.", "Enter the password.";
    LogConectando => "Conectando con Apple…", "Connecting to Apple…";
    LogSeisDigitos => "El código son 6 dígitos.", "The code is 6 digits.";
    LogVerificando => "Verificando el código…", "Verifying the code…";
    LogCaducada => "Tu sesión caducó. Vuelve a escribir la contraseña.", "Your session expired. Enter the password again.";
    LogNoComprobada => "No pude comprobar la sesión: {m}", "Couldn't check the session: {m}";
    LogHecho => "Sesión iniciada. Dura unos 2 meses; icdown te avisará cuando caduque.", "Signed in. It lasts about 2 months; icdown will let you know when it expires.";
    LogCodigoEnviado => "Apple ha enviado un código a tu iPhone.", "Apple sent a code to your iPhone.";
    LogNoGuardada => "No pude guardar la configuración: {e}", "Couldn't save the settings: {e}";
    LogEmpezar => "Vuelve a empezar.", "Start again.";
    LogIncorrectos => "Apple ID o contraseña incorrectos", "Wrong Apple ID or password";
    LogSinConexion => "No pude conectar con Apple: {e}", "Couldn't connect to Apple: {e}";
    LogDemasiados => "Demasiados códigos incorrectos. Vuelve a empezar.", "Too many wrong codes. Start again.";
    LogCodigoMal => "Código incorrecto, prueba otra vez.", "Wrong code, try again.";
    LogNoVerificado => "No pude verificar el código: {e}", "Couldn't verify the code: {e}";
    LogNoValida => "La sesión no quedó válida: {e}", "The session isn't valid: {e}";
    // Asistente de liberar espacio
    AsVentana => "Liberar espacio en iCloud", "Free up iCloud space";
    AsPaso1 => "Elegir", "Choose";
    AsPaso2 => "Revisar", "Review";
    AsPaso3 => "Confirmar", "Confirm";
    AsPaso4 => "En marcha", "Running";
    AsPaso5 => "Resultado", "Result";
    AsElegirSub => "Elige qué borrar de iCloud. Solo se borra lo que ya está copiado y verificado en tu PC.", "Choose what to delete from iCloud. Only what's already copied and verified on your PC is deleted.";
    AsModoDias => "Conservar en iCloud lo más reciente", "Keep the most recent in iCloud";
    AsModoAnios => "Por años de captura", "By year taken";
    AsUltimos => "Conservar los últimos:", "Keep the last:";
    Preset30 => "30 días", "30 days";
    Preset90 => "3 meses", "3 months";
    Preset180 => "6 meses", "6 months";
    Preset365 => "1 año", "1 year";
    Preset730 => "2 años", "2 years";
    Preset1095 => "3 años", "3 years";
    AsAnioCifras => "{n} · {b}", "{n} · {b}";
    AsSinInventario => "Aún no hay un listado de iCloud. Espera a que termine la primera copia.", "There's no iCloud scan yet. Wait for the first copy to finish.";
    AsSeLiberaran => "Se liberarán ~{n} elementos · {b}", "~{n} items will be freed up · {b}";
    AsNadaSeleccionado => "Con esta selección no se libera nada", "Nothing is freed up with this selection";
    AsRango => "De {desde} a {hasta}", "From {desde} to {hasta}";
    AsNoSeTocan => "No se tocan:", "Left untouched:";
    AsTodoCopiado => "Todo lo del rango elegido está copiado y verificado en tu PC", "Everything in the chosen range is copied and verified on your PC";
    AsExclNoCopiados => "{n} sin copiar aún", "{n} not copied yet";
    AsExclCompartida => "{n} de la Fototeca Compartida", "{n} in the Shared Library";
    AsExclNoLiberables => "{n} que icdown aún no sabe copiar", "{n} icdown can't copy yet";
    AsExclRecientes => "{n} añadidos hace menos de 30 días", "{n} added less than 30 days ago";
    AsAtipicos => "{n} con estructura atípica confirmada (se liberan)", "{n} with a confirmed unusual structure (freed up)";
    AsCancelar => "Cancelar", "Cancel";
    AsRevisarBtn => "Revisar", "Review";
    AsAtras => "Atrás", "Back";
    AsContinuar => "Continuar", "Continue";
    AsRevisandoTit => "Comprobando con iCloud y con tus archivos", "Checking with iCloud and your files";
    AsRevisandoSub => "Se vuelve a listar iCloud y se re-verifica en disco cada archivo antes de proponer nada.", "iCloud is listed again and every file is re-verified on disk before anything is proposed.";
    AsListoTit => "Listo para liberar", "Ready to free up";
    AsListoLinea => "{n} elementos · {b}", "{n} items · {b}";
    AsReverificados => "Re-verificados en disco: los {n} coinciden byte a byte con lo copiado", "Re-verified on disk: all {n} match the copy byte for byte";
    AsExcluidosReverif => "{n} excluidos por fallar la re-verificación (se volverán a copiar)", "{n} excluded for failing re-verification (they'll be copied again)";
    AsNadaQueLiberar => "Nada que liberar con este criterio", "Nothing to free up with this choice";
    AsVerLista => "Ver lista", "View list";
    AsAbrirCarpetas => "Abrir carpeta", "Open folder";
    AsRevisionFallo => "No se pudo revisar: {e}", "Couldn't review: {e}";
    AsConfirmarTit => "Confirma para liberar {b}", "Confirm to free up {b}";
    AsConfirmarTexto => "Se moverán {n} elementos de iCloud a «Eliminado recientemente», también en tu iPhone y tus otros dispositivos. Tus archivos del PC no se tocan.", "{n} items will be moved from iCloud to “Recently Deleted”, also on your iPhone and other devices. The files on your PC aren't touched.";
    AsEntiendo => "Entiendo que se moverán a «Eliminado recientemente» y que tengo 30 días para recuperarlas", "I understand they'll be moved to “Recently Deleted” and that I have 30 days to recover them";
    AsMantener => "Liberar {b}", "Free up {b}";
    AsMantenerAyuda => "Mantén pulsado 2 s (o la barra espaciadora)", "Press and hold for 2 s (or the space bar)";
    AsMantenerMarca => "Marca la casilla para poder liberar", "Tick the box to be able to free up";
    AsEnMarchaTit => "Liberando espacio en iCloud", "Freeing up iCloud space";
    AsEnMarchaSub => "Puedes cerrar esta ventana: el borrado sigue y lo verás en el panel del icono.", "You can close this window: deletion continues and you'll see it in the icon panel.";
    AsPausar => "Pausar", "Pause";
    AsReanudar => "Reanudar", "Resume";
    AsCancelarBorrado => "Cancelar", "Cancel";
    AsCancelando => "Cancelando tras el lote en curso…", "Cancelling after the current batch…";
    AsResultadoTit => "Liberados {b}", "Freed up {b}";
    AsResultadoCancelado => "Cancelado: liberados {b}", "Cancelled: freed up {b}";
    AsResultadoElementos => "{n} elementos movidos a «Eliminado recientemente»", "{n} items moved to “Recently Deleted”";
    AsResultadoFallidos => "{n} no se pudieron borrar; se reintentarán la próxima vez", "{n} couldn't be deleted; they'll be retried next time";
    AsResultadoVaciar => "Para recuperar el espacio ya: en el iPhone, Fotos → Álbumes → Eliminado recientemente → Eliminar todo.", "To get the space back now: on your iPhone, Photos → Albums → Recently Deleted → Delete All.";
    AsResultadoNada => "No se borró nada", "Nothing was deleted";
    AsVerBorrados => "Ver borrados.csv", "View borrados.csv";
    AsRestaurar => "Restaurar este borrado", "Restore this deletion";
    AsRestaurarSeguro => "Pulsa otra vez para restaurar", "Click again to restore";
    AsRestaurando => "Restaurando…", "Restoring…";
    AsRestaurados => "Restaurados {n} elementos", "Restored {n} items";
    AsRestauradosFallos => "Restaurados {n}; {f} no se pudieron restaurar", "Restored {n}; {f} couldn't be restored";
    AsRestaurarFallo => "No se pudo restaurar: {e}", "Couldn't restore: {e}";
    AsCerrar => "Cerrar", "Close";
    AsOcupado => "El borrado anterior aún no ha terminado", "The previous deletion hasn't finished yet";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn huecos(s: &str) -> Vec<&str> {
        let mut v: Vec<&str> = s.match_indices('{').filter_map(|(i, _)| s[i + 1..].split_once('}').map(|(k, _)| k)).collect();
        v.sort();
        v
    }

    #[test]
    fn todas_las_claves_estan_en_los_dos_idiomas_con_los_mismos_huecos() {
        assert!(T::TODAS.len() > 150);
        for c in T::TODAS {
            let (es, en) = (c.en(Idioma::Es), c.en(Idioma::En));
            assert!(!es.trim().is_empty() && !en.trim().is_empty(), "{c:?}");
            assert_eq!(huecos(es), huecos(en), "{c:?}");
        }
    }

    #[test]
    fn idioma_detectado_y_forzado() {
        assert_eq!(Idioma::desde_locale("es_ES.UTF-8"), Idioma::Es);
        assert_eq!(Idioma::desde_locale("es_MX"), Idioma::Es);
        assert_eq!(Idioma::desde_locale("en_US.UTF-8"), Idioma::En);
        assert_eq!(Idioma::desde_locale("fr_FR.UTF-8"), Idioma::En, "francés → inglés");
        assert_eq!(Idioma::desde_locale("C"), Idioma::En);
        assert_eq!(Idioma::desde_codigo("EN-us"), Some(Idioma::En));
        assert_eq!(Idioma::desde_codigo("xx"), None);
        assert_eq!(idioma(), Idioma::Es);
        con_idioma(Idioma::En, || {
            assert_eq!(t(T::MenuSalir), "Quit");
            assert_eq!(miles(1_234_567), "1,234,567");
            assert_eq!(decimal1(16.14), "16.1");
            assert_eq!(tf(T::HaceMin, &[("n", &5)]), "5 min ago");
        });
        assert_eq!(miles(1_234_567), "1.234.567");
        assert_eq!(decimal1(16.14), "16,1");
        assert_eq!(tf(T::ParBytes, &[("a", &"1"), ("b", &"2"), ("u", &"GB")]), "1 de 2 GB");
    }

    #[test]
    fn meses_segun_idioma() {
        let t0 = chrono::NaiveDate::from_ymd_opt(2019, 3, 15).unwrap().and_hms_opt(12, 0, 0).unwrap().and_local_timezone(chrono::Local).unwrap().timestamp();
        assert_eq!(mes_anio(t0), "mar 2019");
        assert_eq!(con_idioma(Idioma::En, || mes_anio(t0)), "Mar 2019");
    }
}
