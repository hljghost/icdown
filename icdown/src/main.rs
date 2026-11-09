use anyhow::{anyhow, bail, Context, Result};
use icdown::auditoria;
use icdown::cerrojo;
use icdown::config::{self, Config};
use icdown::criterio::Criterio;
use icdown::descarga;
use icdown::energia::SinSuspension;
use icdown::fuente::FuenteIcloud;
use icdown::modelo::BIBLIOTECA_PERSONAL;
use icdown::liberar;
use icdown::login::{FlujoLogin, Paso};
use icdown::motor::{self, Avance, FinRonda, Listado, OpcionesRonda};
use icdown::progreso_liberar::{self, FaseLiberar, Seguimiento};
use icdown::puente::Puente;
use icdown::registro::Registro;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

const AYUDA: &str = "icdown — copia tus fotos de iCloud al PC

Uso:
  icdown --version               Muestra la versión
  icdown destino <carpeta>       Fija la carpeta donde se guardan las fotos
  icdown login                   Inicia sesión en iCloud (contraseña + código 2FA)
  icdown diagnostico             Comprueba la sesión y que las descargas funcionan
  icdown copiar [--mes AAAA-MM]  Copia y verifica lo que falte
  icdown estado                  Resumen de lo copiado
  icdown verificar               Vuelve a comprobar todo lo copiado
  icdown liberar [--dias N | --anios 2011-2019,2021 | --hasta-anio 2025]
                                 Borra de iCloud lo ya copiado y verificado (pide confirmación;
                                 siempre conserva al menos los últimos 30 días)
  icdown restaurar --ultima | --sesion ID
                                 Saca de \"Eliminado recientemente\" lo borrado en una liberación
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let pausa = args.iter().any(|a| a == "--pausa");
    let args: Vec<String> = args.into_iter().filter(|a| a != "--pausa").collect();
    let r = ejecutar(&args);
    if let Err(e) = &r {
        eprintln!("\nError: {e:#}");
    }
    if pausa {
        eprint!("\nPulsa Enter para cerrar…");
        let _ = io::stdin().lock().read_line(&mut String::new());
    }
    if r.is_err() {
        std::process::exit(1);
    }
}

fn ejecutar(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("--version") | Some("-V") => {
            println!("icdown {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("destino") => cmd_destino(args.get(1)),
        Some("login") => cmd_login(),
        Some("diagnostico") => cmd_diagnostico(),
        Some("copiar") => cmd_copiar(args),
        Some("estado") => cmd_estado(),
        Some("verificar") => cmd_verificar(),
        Some("liberar") => cmd_liberar(args),
        Some("restaurar") => cmd_restaurar(args),
        _ => {
            print!("{AYUDA}");
            Ok(())
        }
    }
}

fn opcion<'a>(args: &'a [String], nombre: &str) -> Option<&'a str> {
    args.iter().position(|a| a == nombre).and_then(|i| args.get(i + 1)).map(String::as_str)
}

fn preguntar(texto: &str) -> Result<String> {
    print!("{texto}");
    io::stdout().flush()?;
    let mut s = String::new();
    io::stdin().lock().read_line(&mut s)?;
    Ok(s.trim().to_string())
}

fn abrir_puente(cfg: &Config) -> Result<Puente> {
    let apple_id = cfg.apple_id.as_deref().context("No hay Apple ID: ejecuta `icdown login`")?;
    Puente::iniciar(apple_id, &config::dir_sesion(), &config::log_puente())
}

fn cmd_destino(ruta: Option<&String>) -> Result<()> {
    let ruta = PathBuf::from(ruta.context("Uso: icdown destino <carpeta>")?);
    std::fs::create_dir_all(&ruta).with_context(|| format!("no puedo crear {}", ruta.display()))?;
    let mut cfg = Config::cargar()?;
    cfg.destino = Some(ruta.clone());
    cfg.guardar()?;
    println!("Destino: {}", ruta.display());
    Ok(())
}

fn cmd_login() -> Result<()> {
    let mut cfg = Config::cargar()?;
    let defecto = cfg.apple_id.clone().unwrap_or_default();
    let escrito = preguntar(&format!("Apple ID [{defecto}]: "))?;
    let apple_id = if escrito.is_empty() { defecto } else { escrito };
    if apple_id.is_empty() {
        bail!("Hace falta el Apple ID");
    }
    let password = rpassword::prompt_password("Contraseña (no se guarda): ")?;
    let puente = Puente::iniciar(&apple_id, &config::dir_sesion(), &config::log_puente())?;
    let mut flujo = FlujoLogin::new(puente);
    let mut paso = flujo.entrar(&password);
    while paso == Paso::PideCodigo {
        let codigo = preguntar("Código de 6 dígitos que apareció en tu iPhone: ")?;
        paso = match flujo.verificar(&codigo) {
            Paso::Reintentar(m) => {
                println!("{m}");
                Paso::PideCodigo
            }
            otro => otro,
        };
    }
    match paso {
        Paso::Hecho => {}
        Paso::Reintentar(m) | Paso::Fallo(m) => bail!("{m}"),
        Paso::PideCodigo => unreachable!(),
    }
    cfg.apple_id = Some(apple_id);
    cfg.guardar()?;
    println!("Sesión iniciada. Dura unos 2 meses; icdown te avisará cuando caduque.");
    Ok(())
}

fn cmd_diagnostico() -> Result<()> {
    let cfg = Config::cargar()?;
    let mut puente = abrir_puente(&cfg)?;
    puente.sesion().map_err(|e| anyhow!("{e}"))?;
    println!("Sesión: válida");
    let (mut assets, mut partes, mut bytes, mut compartidos) = (0u64, 0u64, 0u64, 0u64);
    let mut primera: Option<(String, u64)> = None;
    let total = puente
        .listar(&mut |a| {
            assets += 1;
            if a.biblioteca != BIBLIOTECA_PERSONAL {
                compartidos += 1;
            }
            for p in &a.partes {
                partes += 1;
                bytes += p.tamano;
                if primera.is_none() {
                    primera = Some((p.url.clone(), p.tamano));
                }
            }
            if assets % 500 == 0 {
                eprint!("\r  listados {assets}…");
            }
            true
        })
        .map_err(|e| anyhow!("{e}"))?;
    println!(
        "\nEn iCloud: {total} elementos ({compartidos} de Fototeca Compartida), {partes} archivos, {:.1} GB",
        bytes as f64 / 1e9
    );
    let (url, tamano) = primera.context("La biblioteca está vacía")?;
    let fin = tamano.min(1024).saturating_sub(1);
    let resp = reqwest::blocking::Client::new().get(&url).header("Range", format!("bytes=0-{fin}")).send()?;
    let estado = resp.status().as_u16();
    let rango = resp.headers().get("content-range").and_then(|v| v.to_str().ok()).unwrap_or("-").to_string();
    println!("Descarga de prueba sin cookies: HTTP {estado}, Content-Range: {rango}");
    match estado {
        206 => println!("OK: las descargas funcionan y se pueden reanudar."),
        200 => println!("AVISO: funciona, pero el servidor ignora Range (no se podrá reanudar a mitad de archivo)."),
        _ => bail!("La descarga directa falló: revisar el diseño antes de seguir."),
    }
    Ok(())
}

fn destino_de(cfg: &Config) -> Result<PathBuf> {
    cfg.destino.clone().context("No hay destino: ejecuta `icdown destino <carpeta>`")
}

fn abrir_registro(destino: &Path) -> Result<Registro> {
    Registro::abrir(&destino.join(".icdown").join("registro.db"))
}

fn imprimir_resumen(reg: &Registro) -> Result<()> {
    let r = reg.resumen()?;
    println!("Elementos de iCloud conocidos: {} (completos: {})", r.assets, r.assets_completos);
    println!(
        "Archivos verificados: {} de {} ({:.1} de {:.1} GB)",
        r.verificadas, r.partes, r.bytes_verificados as f64 / 1e9, r.bytes_total as f64 / 1e9
    );
    println!("Pendientes: {}   Con error: {}   Borrados de iCloud: {}", r.pendientes, r.con_error, r.borrados_icloud);
    if r.estructura_atipica > 0 {
        println!("Con estructura atípica (confirmadas por doble descarga): {}", r.estructura_atipica);
    }
    if let Some(m) = reg.marca_lista()? {
        if let Some(f) = chrono::DateTime::from_timestamp(m, 0) {
            println!("Último listado de iCloud: {}", f.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M"));
        }
    }
    let errores = reg.errores(10)?;
    if !errores.is_empty() {
        println!("Últimos errores:");
        for e in errores {
            println!("  - {e}");
        }
    }
    Ok(())
}

fn cmd_estado() -> Result<()> {
    let cfg = Config::cargar()?;
    let destino = destino_de(&cfg)?;
    println!("Destino: {}", destino.display());
    imprimir_resumen(&abrir_registro(&destino)?)
}

fn cmd_copiar(args: &[String]) -> Result<()> {
    let cfg = Config::cargar()?;
    let destino = destino_de(&cfg)?;
    let mes = opcion(args, "--mes").map(str::to_string);
    let dir_datos = destino.join(".icdown");
    let _cerrojo = cerrojo::intentar(&dir_datos)?
        .context("Ya hay otra copia en marcha (¿el icono de la bandeja?). Páusala primero.")?;
    let _energia = SinSuspension::activar();
    let registro = Mutex::new(abrir_registro(&destino)?);
    registro.lock().unwrap().reintentar_ya()?;
    let http = descarga::cliente();
    let cancelar = AtomicBool::new(false);
    let avance = |a: Avance| match a {
        Avance::Listando { vistos } => eprint!("\rRevisando iCloud… {vistos} elementos      "),
        Avance::Copiando { hechas, total, actual } => eprint!("\r[{hechas}/{total}] {actual}                    "),
        Avance::RenovandoEnlaces { n, hechas, total } => eprint!("\r[{hechas}/{total}] Renovando enlaces ({n})…                    "),
    };
    loop {
        let mut puente = abrir_puente(&cfg)?;
        let op = OpcionesRonda { concurrencia: cfg.concurrencia, mes: mes.clone() };
        let r = motor::ejecutar_ronda(&destino, &registro, &mut puente, &http, &op, &cancelar, &avance)?;
        drop(puente);
        eprintln!();
        println!("Ronda: {} archivos copiados y verificados, {} fallidos", r.verificadas, r.fallidas);
        if r.aplazadas > 0 {
            println!("{} archivos aplazados: sus enlaces de descarga caducan", r.aplazadas);
        }
        match r.fin {
            FinRonda::Relistar => {
                println!("Algunos enlaces de descarga caducaron; pido enlaces nuevos a iCloud…");
                continue;
            }
            FinRonda::SesionCaducada => bail!("La sesión de iCloud caducó: ejecuta `icdown login`"),
            FinRonda::SinEspacio => bail!("No queda espacio suficiente en {}", destino.display()),
            FinRonda::DestinoAusente => bail!("No encuentro {}", destino.display()),
            FinRonda::Cancelada => return Ok(()),
            FinRonda::Completa => {}
        }
        let prox = registro.lock().unwrap().proximo_reintento(mes.as_deref())?;
        match prox {
            Some(t) if t - motor::ahora() <= 3600 => {
                let s = (t - motor::ahora()).max(1) as u64;
                println!("Reintento de lo fallido en {s} s…");
                std::thread::sleep(Duration::from_secs(s));
            }
            _ => break,
        }
    }
    let res = imprimir_resumen(&registro.lock().unwrap());
    res
}

fn cmd_verificar() -> Result<()> {
    let cfg = Config::cargar()?;
    let destino = destino_de(&cfg)?;
    let dir_datos = destino.join(".icdown");
    let _cerrojo = cerrojo::intentar(&dir_datos)?.context("Hay una copia en marcha; vuelve a intentarlo luego.")?;
    let registro = Mutex::new(abrir_registro(&destino)?);
    let r = auditoria::verificar_todo(&destino, &registro, &AtomicBool::new(false), &|h, t| {
        if h % 100 == 0 || h == t {
            eprint!("\rVerificando {h}/{t}…");
        }
    })?;
    eprintln!();
    if r.malas.is_empty() {
        println!("Todo correcto: {} archivos verificados.", r.revisadas);
    } else {
        println!("{} archivos dañados o ausentes (se volverán a descargar en la próxima copia):", r.malas.len());
        for (ruta, m) in r.malas.iter().take(20) {
            println!("  - {ruta}: {m}");
        }
    }
    Ok(())
}

fn cmd_restaurar(args: &[String]) -> Result<()> {
    let cfg = Config::cargar()?;
    let destino = destino_de(&cfg)?;
    let dir_datos = destino.join(".icdown");
    let _cerrojo = match cerrojo::intentar(&dir_datos)? {
        Some(c) => c,
        None => {
            println!("Esperando a que termine la copia en curso…");
            cerrojo::esperar(&dir_datos)?
        }
    };
    let registro = Mutex::new(abrir_registro(&destino)?);
    let sesion = match (opcion(args, "--sesion"), args.iter().any(|a| a == "--ultima")) {
        (Some(s), _) => s.to_string(),
        (None, true) => registro.lock().unwrap().ultima_sesion_borrado()?.context("No hay ninguna liberación que restaurar")?,
        (None, false) => bail!("Uso: icdown restaurar --ultima | --sesion ID"),
    };
    let ids = registro.lock().unwrap().ids_de_sesion(&sesion)?;
    if ids.is_empty() {
        println!("La liberación {sesion} no tiene nada borrado que restaurar.");
        return Ok(());
    }
    println!("Restaurar {} elementos de la liberación {sesion} (vuelven de \"Eliminado recientemente\" a la fototeca).", ids.len());
    if preguntar("Escribe RESTAURAR para confirmar: ")? != "RESTAURAR" {
        println!("Cancelado.");
        return Ok(());
    }
    let mut puente = abrir_puente(&cfg)?;
    let r = liberar::restaurar(&destino, &registro, &mut puente, &ids)?;
    println!("Restaurados: {}", r.restaurados.len());
    if !r.fallidos.is_empty() {
        println!("No se pudieron restaurar {}:", r.fallidos.len());
        for (id, e) in r.fallidos.iter().take(10) {
            println!("  - {id}: {e}");
        }
    }
    Ok(())
}

fn cmd_liberar(args: &[String]) -> Result<()> {
    let cfg = Config::cargar()?;
    let destino = destino_de(&cfg)?;
    let criterio = match (opcion(args, "--anios"), opcion(args, "--hasta-anio"), opcion(args, "--dias")) {
        (Some(a), _, _) => Criterio::Anios(icdown::criterio::parsear_anios(a).map_err(|e| anyhow!("--anios: {e}"))?),
        (None, Some(h), _) => Criterio::Anios(icdown::criterio::hasta_anio(h.trim().parse().context("--hasta-anio: un año, p. ej. 2025")?)),
        (None, None, Some(d)) => Criterio::ConservarDias(d.parse().context("--dias debe ser un número")?),
        (None, None, None) => Criterio::ConservarDias(cfg.mantener_dias),
    };
    let dir_datos = destino.join(".icdown");
    let _cerrojo = match cerrojo::intentar(&dir_datos)? {
        Some(c) => c,
        None => {
            println!("Esperando a que termine la copia en curso…");
            cerrojo::esperar(&dir_datos)?
        }
    };
    let registro = Mutex::new(abrir_registro(&destino)?);
    // El avance queda en el registro para que la bandeja lo muestre; un error lo deja como fallido.
    let r = liberar_con_registro(&cfg, &destino, &registro, &criterio);
    if let Err(e) = &r {
        progreso_liberar::marcar_fallido(&registro, &format!("{e:#}"));
    }
    r
}

/// Mientras se espera la confirmación, renueva el avance cada 20 s (si no, la bandeja lo daría por abandonado).
fn preguntar_con_latido(texto: &str, registro: &Mutex<Registro>) -> Result<String> {
    let fin = AtomicBool::new(false);
    std::thread::scope(|s| {
        s.spawn(|| {
            let mut n = 0;
            while !fin.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(250));
                n += 1;
                if n % 80 == 0 {
                    progreso_liberar::latir(registro);
                }
            }
        });
        let r = preguntar(texto);
        fin.store(true, Ordering::Relaxed);
        r
    })
}

fn liberar_con_registro(cfg: &Config, destino: &Path, registro: &Mutex<Registro>, criterio: &Criterio) -> Result<()> {
    let dias = criterio.dias();
    let dir_datos = destino.join(".icdown");
    let previsto = registro.lock().unwrap().assets_ultimo_listado()?.unwrap_or(0);
    let mut seg = Seguimiento::empezar(registro, dias);
    seg.fase(FaseLiberar::Listando, previsto, 0);
    let mut puente = abrir_puente(cfg)?;
    let cancelar = AtomicBool::new(false);
    println!("Consultando iCloud para trabajar con datos frescos…");
    let seg = Mutex::new(seg);
    let avance_lista = |a: Avance| {
        if let Avance::Listando { vistos } = a {
            seg.lock().unwrap().avance(vistos as u64, 0);
            eprint!("\r  {vistos} elementos…");
        }
    };
    match motor::listar_en_registro(registro, &mut puente, &cancelar, &avance_lista)? {
        Listado::Completo(n) => eprintln!("\r  {n} elementos en iCloud."),
        Listado::Interrumpido(FinRonda::SesionCaducada, _) => bail!("La sesión de iCloud caducó: ejecuta `icdown login`"),
        Listado::Interrumpido(..) => bail!("El listado de iCloud no terminó; no se borra nada."),
    }
    println!("Re-verificando en disco cada archivo antes de proponer nada…");
    let plan = liberar::preparar_criterio(destino, registro, criterio, motor::ahora(), &|h, t| {
        if h % 100 == 0 || h == t {
            eprint!("\r  {h}/{t}");
        }
    })?;
    eprintln!();
    println!("\n=== Liberar espacio en iCloud ===");
    println!("Listos para borrar de iCloud (y del iPhone): {} elementos, {:.1} GB", plan.listos.len(), plan.bytes as f64 / 1e9);
    if let (Some(a), Some(b)) = (plan.listos.first(), plan.listos.last()) {
        println!("Fechas: {} → {}", &a.fecha[..10], &b.fecha[..10]);
    }
    if plan.atipicos > 0 {
        println!("De ellos, {} con estructura atípica (confirmadas por doble descarga)", plan.atipicos);
    }
    if let Some(a) = criterio.anios() {
        let (min, max) = (a.first().copied().unwrap_or_default(), a.last().copied().unwrap_or_default());
        if a.len() as i32 == max - min + 1 && min <= 1900 {
            println!("Años de captura elegidos: hasta {max} incluido");
        } else {
            println!("Años de captura elegidos: {}", a.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", "));
        }
    }
    println!("Se conservan en iCloud los últimos {dias} días y todo lo de la Fototeca Compartida.");
    if plan.no_verificados > 0 {
        println!("No incluidos por no estar copiados y verificados del todo: {}", plan.no_verificados);
    }
    if plan.no_liberables > 0 {
        println!("No incluidos: contienen datos que icdown aún no sabe copiar: {}", plan.no_liberables);
    }
    if !plan.excluidos.is_empty() {
        println!("EXCLUIDOS por fallar la re-verificación: {}", plan.excluidos.len());
        for (c, m) in plan.excluidos.iter().take(10) {
            println!("  - {} ({}): {m}", c.nombre_base, &c.fecha[..10]);
        }
    }
    if plan.listos.is_empty() {
        println!("Nada que borrar.");
        return Ok(());
    }
    println!("\nLo borrado irá a \"Eliminado recientemente\" (recuperable 30 días). Tus archivos del PC no se tocan.");
    if preguntar_con_latido("Escribe BORRAR para confirmar: ", registro)? != "BORRAR" {
        Seguimiento::continuar(registro, None).descartar();
        println!("Cancelado. No se borró nada.");
        return Ok(());
    }
    let r = liberar::ejecutar(destino, registro, &mut puente, &plan.listos, &|h, t| eprint!("\r  borrados {h}/{t}"))?;
    eprintln!();
    println!("Borrados de iCloud: {}", r.borrados);
    if !r.fallidos.is_empty() {
        println!("No se pudieron borrar {} (se reintentará la próxima vez):", r.fallidos.len());
        for (id, e) in r.fallidos.iter().take(10) {
            println!("  - {id}: {e}");
        }
    }
    if !r.errores.is_empty() {
        println!("ATENCIÓN: {} borrados no se pudieron anotar del todo:", r.errores.len());
        for (id, e) in r.errores.iter().take(10) {
            println!("  - {id}: {e}");
        }
    }
    println!("Registro de lo borrado: {} (liberación {})", dir_datos.join("borrados.csv").display(), r.sesion);
    println!("Para deshacerla: icdown restaurar --sesion {}", r.sesion);
    println!("Para liberar el espacio ya: iPhone → Fotos → Eliminado recientemente → Eliminar todo.");
    Ok(())
}
