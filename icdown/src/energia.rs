//! Impide que el sistema se suspenda mientras viva el guardián, con `systemd-inhibit`.
//! Si no se puede lanzar (sin systemd), no hace nada.
use std::marker::PhantomData;
use std::process::{Child, Command, Stdio};

pub struct SinSuspension {
    hijo: Option<Child>,
    _no_send: PhantomData<*const ()>,
}

impl SinSuspension {
    pub fn activar() -> SinSuspension {
        let hijo = Command::new("systemd-inhibit")
            .args(["--what=idle:sleep", "--who=icdown", "--why=downloading", "--mode=block", "sleep", "infinity"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok();
        SinSuspension { hijo, _no_send: PhantomData }
    }
}

impl Drop for SinSuspension {
    fn drop(&mut self) {
        if let Some(mut h) = self.hijo.take() {
            let _ = h.kill();
            let _ = h.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn activar_y_soltar() {
        let g = super::SinSuspension::activar();
        drop(g);
    }
}
