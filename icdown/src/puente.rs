//! Cliente del proceso puente-icloud (Python): JSON por líneas sobre stdin/stdout.
use crate::fuente::{ErrorFuente, FuenteIcloud, ResultadoBorrado};
use crate::modelo::AssetIcloud;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

pub struct Puente {
    hijo: Child,
    entrada: ChildStdin,
    salida: BufReader<ChildStdout>,
}

/// Orden de búsqueda: `ICDOWN_PUENTE` (ruta a ejecutable o .py; con .py usa `ICDOWN_PYTHON` o `python3`),
/// luego `puente-icloud` junto al ejecutable actual y, para desarrollo, `../puente/puente_icloud.py`
/// (relativo al manifiesto del crate o al ejecutable) con `python3`.
pub fn comando_puente() -> anyhow::Result<Command> {
    if let Ok(ruta) = std::env::var("ICDOWN_PUENTE") {
        return Ok(comando_de(&ruta));
    }
    let exe = std::env::current_exe()?;
    let junto = exe.with_file_name("puente-icloud");
    if junto.exists() {
        return Ok(Command::new(junto));
    }
    let mut candidatos: Vec<std::path::PathBuf> = Vec::new();
    candidatos.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../puente/puente_icloud.py"));
    if let Some(dir) = exe.parent() {
        candidatos.push(dir.join("../puente/puente_icloud.py"));
        candidatos.push(dir.join("../../../puente/puente_icloud.py"));
        candidatos.push(dir.join("../../../../puente/puente_icloud.py"));
    }
    if let Some(p) = candidatos.into_iter().find(|p| p.exists()) {
        return Ok(comando_de(&p.to_string_lossy()));
    }
    anyhow::bail!("No encuentro puente-icloud junto a {} ni ../puente/puente_icloud.py", exe.display())
}

fn comando_de(ruta: &str) -> Command {
    if ruta.to_lowercase().ends_with(".py") {
        let python = std::env::var("ICDOWN_PYTHON").unwrap_or_else(|_| "python3".into());
        let mut c = Command::new(python);
        c.arg("-I").arg(ruta);
        c
    } else {
        Command::new(ruta)
    }
}

/// Directorio de la sesión de iCloud (JSON con session token y trust token, y cookies).
///
/// `create_dir_all` usa el modo `0777` enmascarado por el umask: con `022` queda en `0755`,
/// y otro usuario local puede leer los secretos si los padres se pueden atravesar. pyicloud
/// después hace `makedirs(..., 0o700, exist_ok=True)`, que no cambia un directorio ya creado.
/// Aquí el directorio queda en `0700` y los archivos que ya hubiera, si están más abiertos, en `0600`.
fn asegurar_permisos_sesion(dir: &Path) -> std::io::Result<()> {
    if !dir.is_dir() {
        if let Some(padre) = dir.parent() {
            if !padre.as_os_str().is_empty() {
                std::fs::create_dir_all(padre)?;
            }
        }
        match crear_directorio_sesion(dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && dir.is_dir() => {}
            Err(e) => return Err(e),
        }
    }
    #[cfg(unix)]
    cerrar_directorio_y_archivos(dir)?;
    Ok(())
}

fn crear_directorio_sesion(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(0o700).create(dir)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir(dir)
    }
}

/// Quita bits de más. Un modo ya más cerrado (p. ej. `0400`) se deja igual.
#[cfg(unix)]
fn cerrar_si_mas_abierto(path: &Path, modo: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(path)?;
    let actual = meta.permissions().mode() & 0o777;
    if actual & !modo != 0 {
        let mut permisos = meta.permissions();
        permisos.set_mode(modo);
        std::fs::set_permissions(path, permisos)?;
    }
    Ok(())
}

#[cfg(unix)]
fn cerrar_directorio_y_archivos(dir: &Path) -> std::io::Result<()> {
    cerrar_si_mas_abierto(dir, 0o700)?;
    for entrada in std::fs::read_dir(dir)? {
        let entrada = entrada?;
        // `file_type` no sigue enlaces: no se cambia el destino de un atajo.
        if entrada.file_type()?.is_file() {
            cerrar_si_mas_abierto(&entrada.path(), 0o600)?;
        }
    }
    Ok(())
}

fn comprobar(v: &Value) -> Result<(), ErrorFuente> {
    if v["ok"].as_bool() == Some(true) {
        return Ok(());
    }
    let error = v["error"].as_str().unwrap_or("otro").to_string();
    let detalle = v["detalle"].as_str().unwrap_or("").to_string();
    Err(match error.as_str() {
        "sesion_caducada" => ErrorFuente::SesionCaducada,
        "credenciales" | "codigo_incorrecto" => ErrorFuente::Credenciales(format!("{error} {detalle}").trim().to_string()),
        _ => ErrorFuente::Otro(if detalle.is_empty() { error } else { detalle }),
    })
}

impl Puente {
    /// Con el dominio de iCloud de la configuración (`com`, o `cn` para iCloud China).
    pub fn iniciar(apple_id: &str, dir_sesion: &Path, log: &Path) -> anyhow::Result<Puente> {
        let mut cmd = comando_puente()?;
        let dominio = crate::config::Config::cargar().map(|c| c.dominio).unwrap_or_default();
        // Solo si no es el normal: un puente antiguo no conoce la opción.
        if dominio != crate::config::Dominio::Com {
            cmd.arg("--dominio").arg(dominio.como_str());
        }
        Self::iniciar_con(cmd, apple_id, dir_sesion, log)
    }

    pub fn iniciar_con(mut cmd: Command, apple_id: &str, dir_sesion: &Path, log: &Path) -> anyhow::Result<Puente> {
        // El directorio lo crea este proceso, antes de que pyicloud haga
        // makedirs(..., 0o700, exist_ok=True). Ese makedirs no corrige uno que ya existe.
        asegurar_permisos_sesion(dir_sesion)?;
        if let Some(p) = log.parent() {
            std::fs::create_dir_all(p)?;
        }
        let stderr = std::fs::OpenOptions::new().create(true).append(true).open(log)?;
        cmd.arg("--cookies")
            .arg(dir_sesion)
            .arg("--apple-id")
            .arg(apple_id)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(stderr));
        let mut hijo = cmd.spawn().map_err(|e| anyhow::anyhow!("no pude lanzar el puente: {e}"))?;
        let entrada = hijo.stdin.take().expect("stdin del puente");
        let salida = BufReader::new(hijo.stdout.take().expect("stdout del puente"));
        Ok(Puente { hijo, entrada, salida })
    }

    fn enviar(&mut self, v: &Value) -> Result<(), ErrorFuente> {
        let mut linea = v.to_string();
        linea.push('\n');
        self.entrada
            .write_all(linea.as_bytes())
            .and_then(|_| self.entrada.flush())
            .map_err(|e| ErrorFuente::Otro(format!("no puedo escribir al puente: {e}")))
    }

    fn leer(&mut self) -> Result<Value, ErrorFuente> {
        let mut linea = String::new();
        let n = self
            .salida
            .read_line(&mut linea)
            .map_err(|e| ErrorFuente::Otro(format!("no puedo leer del puente: {e}")))?;
        if n == 0 {
            return Err(ErrorFuente::Otro("el puente terminó inesperadamente (ver puente.log)".into()));
        }
        serde_json::from_str(&linea).map_err(|e| ErrorFuente::Otro(format!("respuesta inválida del puente: {e}: {linea}")))
    }

    pub fn sesion(&mut self) -> Result<(), ErrorFuente> {
        self.enviar(&json!({"cmd": "sesion"}))?;
        comprobar(&self.leer()?)
    }

    /// Devuelve `true` si además hace falta el código 2FA.
    pub fn login(&mut self, password: &str) -> Result<bool, ErrorFuente> {
        self.enviar(&json!({"cmd": "login", "password": password}))?;
        let v = self.leer()?;
        comprobar(&v)?;
        Ok(v["requiere_2fa"].as_bool().unwrap_or(false))
    }

    pub fn codigo_2fa(&mut self, codigo: &str) -> Result<(), ErrorFuente> {
        self.enviar(&json!({"cmd": "codigo_2fa", "codigo": codigo}))?;
        comprobar(&self.leer()?)
    }
}

impl FuenteIcloud for Puente {
    fn listar(&mut self, en_asset: &mut dyn FnMut(AssetIcloud) -> bool) -> Result<usize, ErrorFuente> {
        self.enviar(&json!({"cmd": "listar"}))?;
        loop {
            let v = self.leer()?;
            if let Some(a) = v.get("asset") {
                let asset: AssetIcloud = serde_json::from_value(a.clone())
                    .map_err(|e| ErrorFuente::Otro(format!("asset inválido del puente: {e}")))?;
                if !en_asset(asset) {
                    let _ = self.hijo.kill();
                    return Err(ErrorFuente::Cancelado);
                }
                continue;
            }
            if v.get("aviso").is_some() {
                continue;
            }
            comprobar(&v)?;
            return Ok(v["total"].as_u64().unwrap_or(0) as usize);
        }
    }

    fn refrescar(&mut self, ids: &[String], en_asset: &mut dyn FnMut(AssetIcloud) -> bool) -> Result<Vec<String>, ErrorFuente> {
        self.enviar(&json!({"cmd": "refrescar", "ids": ids}))?;
        loop {
            let v = self.leer()?;
            if let Some(a) = v.get("asset") {
                let asset: AssetIcloud = serde_json::from_value(a.clone())
                    .map_err(|e| ErrorFuente::Otro(format!("asset inválido del puente: {e}")))?;
                if !en_asset(asset) {
                    let _ = self.hijo.kill();
                    return Err(ErrorFuente::Cancelado);
                }
                continue;
            }
            if v.get("aviso").is_some() {
                continue;
            }
            comprobar(&v)?;
            return Ok(v["faltan"]
                .as_array()
                .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .unwrap_or_default());
        }
    }

    fn borrar(&mut self, ids: &[String]) -> Result<ResultadoBorrado, ErrorFuente> {
        self.enviar(&json!({"cmd": "borrar", "ids": ids}))?;
        let v = self.leer()?;
        comprobar(&v)?;
        Ok(resultado(&v, "borrados"))
    }

    fn restaurar(&mut self, ids: &[String]) -> Result<ResultadoBorrado, ErrorFuente> {
        self.enviar(&json!({"cmd": "restaurar", "ids": ids}))?;
        let v = self.leer()?;
        comprobar(&v)?;
        Ok(resultado(&v, "restaurados"))
    }
}

/// `{"<hechos>": [...], "fallidos": [{"id", "error"}]}` del puente.
fn resultado(v: &Value, hechos: &str) -> ResultadoBorrado {
    let borrados = v[hechos]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let fallidos = v["fallidos"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|x| (x["id"].as_str().unwrap_or("").to_string(), x["error"].as_str().unwrap_or("").to_string()))
                .collect()
        })
        .unwrap_or_default();
    ResultadoBorrado { borrados, fallidos }
}

impl Drop for Puente {
    fn drop(&mut self) {
        let _ = self.hijo.kill();
        let _ = self.hijo.wait();
    }
}

#[cfg(all(test, unix))]
mod pruebas_permisos {
    use super::asegurar_permisos_sesion;
    use std::os::unix::fs::PermissionsExt;

    fn modo(path: &std::path::Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    fn poner_modo(path: &std::path::Path, modo: u32) {
        let mut permisos = std::fs::metadata(path).unwrap().permissions();
        permisos.set_mode(modo);
        std::fs::set_permissions(path, permisos).unwrap();
    }

    #[test]
    fn directorio_nuevo_queda_en_0700() {
        let base = tempfile::tempdir().unwrap();
        let dir = base.path().join("sesion");
        asegurar_permisos_sesion(&dir).unwrap();
        assert_eq!(modo(&dir), 0o700);
    }

    #[test]
    fn directorio_y_secretos_abiertos_se_cierran_sin_tocar_el_contenido() {
        let base = tempfile::tempdir().unwrap();
        let dir = base.path().join("sesion");
        std::fs::create_dir(&dir).unwrap();
        poner_modo(&dir, 0o755);

        let sesion = dir.join("yoejemplocom.session");
        let cuerpo = br#"{"session_token":"s","trust_token":"t"}"#;
        std::fs::write(&sesion, cuerpo).unwrap();
        poner_modo(&sesion, 0o644);

        let cookies = dir.join("yoejemplocom");
        std::fs::write(&cookies, b"#LWP-Cookies-2.0\n").unwrap();
        poner_modo(&cookies, 0o644);

        let fuera = base.path().join("fuera");
        std::fs::write(&fuera, b"no").unwrap();
        poner_modo(&fuera, 0o644);
        std::os::unix::fs::symlink(&fuera, dir.join("atajo")).unwrap();

        asegurar_permisos_sesion(&dir).unwrap();

        assert_eq!(modo(&dir), 0o700);
        assert_eq!(modo(&sesion), 0o600);
        assert_eq!(modo(&cookies), 0o600);
        assert_eq!(std::fs::read(&sesion).unwrap(), cuerpo);
        assert_eq!(modo(&fuera), 0o644);
    }

    #[test]
    fn iniciar_con_cierra_directorio_y_json_ya_abiertos() {
        let base = tempfile::tempdir().unwrap();
        let dir = base.path().join("sesion");
        std::fs::create_dir(&dir).unwrap();
        poner_modo(&dir, 0o755);
        let sesion = dir.join("yo.session");
        std::fs::write(&sesion, br#"{"session_token":"s","trust_token":"t"}"#).unwrap();
        poner_modo(&sesion, 0o644);

        let puente = super::Puente::iniciar_con(
            std::process::Command::new("/bin/true"),
            "yo@ejemplo.com",
            &dir,
            &base.path().join("puente.log"),
        )
        .unwrap();
        assert_eq!(modo(&dir), 0o700);
        assert_eq!(modo(&sesion), 0o600);
        assert_eq!(std::fs::read(&sesion).unwrap(), br#"{"session_token":"s","trust_token":"t"}"#);
        drop(puente);
    }

    #[test]
    fn un_archivo_mas_cerrado_no_se_abre() {
        let base = tempfile::tempdir().unwrap();
        let dir = base.path().join("sesion");
        std::fs::create_dir(&dir).unwrap();
        let archivo = dir.join("ya.session");
        std::fs::write(&archivo, b"{}").unwrap();
        poner_modo(&archivo, 0o400);

        asegurar_permisos_sesion(&dir).unwrap();

        assert_eq!(modo(&archivo), 0o400);
        assert_eq!(modo(&dir), 0o700);
    }
}
