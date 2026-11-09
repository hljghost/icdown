//! Notificaciones de escritorio con notify-rust (nunca fallan).
use notify_rust::Notification;

pub fn mostrar(titulo: &str, texto: &str) {
    let _ = Notification::new().appname("icdown").summary(titulo).body(texto).icon("folder-download").show();
}
