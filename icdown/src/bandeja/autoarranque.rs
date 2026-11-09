//! "Iniciar con la sesión": servicio de usuario systemd `icdown-tray.service`.
use std::path::PathBuf;
use std::process::{Command, Stdio};

const SERVICIO: &str = "icdown-tray.service";

fn ruta_unidad() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("systemd").join("user").join(SERVICIO))
}

fn systemctl(args: &[&str]) -> std::io::Result<std::process::ExitStatus> {
    Command::new("systemctl").arg("--user").args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status()
}

pub fn activo() -> bool {
    systemctl(&["is-enabled", "--quiet", SERVICIO]).map(|s| s.success()).unwrap_or(false)
}

fn texto_unidad(exe: &std::path::Path) -> String {
    format!(
        "[Unit]\nDescription=icdown tray (iCloud Photos backup)\nAfter=graphical-session.target\nPartOf=graphical-session.target\n\n\
         [Service]\nType=simple\nExecStart=\"{}\"\nRestart=on-failure\nRestartSec=5\n\n\
         [Install]\nWantedBy=graphical-session.target\n",
        exe.display()
    )
}

pub fn fijar(activar: bool) -> std::io::Result<()> {
    if activar {
        let exe = std::env::current_exe()?;
        let ruta = ruta_unidad().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "sin ~/.config"))?;
        if let Some(p) = ruta.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(&ruta, texto_unidad(&exe))?;
        systemctl(&["daemon-reload"])?;
        let st = systemctl(&["enable", SERVICIO])?;
        if st.success() { Ok(()) } else { Err(std::io::Error::other("systemctl --user enable falló")) }
    } else {
        let _ = systemctl(&["disable", SERVICIO]);
        if let Some(r) = ruta_unidad() {
            match std::fs::remove_file(r) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
        let _ = systemctl(&["daemon-reload"]);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn unidad_contiene_exec() {
        let t = super::texto_unidad(std::path::Path::new("/usr/bin/icdown-tray"));
        assert!(t.contains("ExecStart=\"/usr/bin/icdown-tray\""));
        assert!(t.contains("WantedBy=graphical-session.target"));
    }
}
