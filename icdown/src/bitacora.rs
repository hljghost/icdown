//! Logs diarios en <destino>\.icdown\logs\icdown-AAAA-MM-DD.log. Nunca falla.
use std::io::Write;
use std::path::Path;

pub fn escribir(dir_datos: &Path, linea: &str) {
    let dir = dir_datos.join("logs");
    let _ = std::fs::create_dir_all(&dir);
    let ahora = chrono::Local::now();
    let nombre = format!("icdown-{}.log", ahora.format("%Y-%m-%d"));
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join(nombre)) {
        let _ = writeln!(f, "{} {}", ahora.format("%H:%M:%S"), linea);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn escribe_linea() {
        let d = tempfile::tempdir().unwrap();
        super::escribir(d.path(), "hola");
        let logs: Vec<_> = std::fs::read_dir(d.path().join("logs")).unwrap().collect();
        assert_eq!(logs.len(), 1);
        let txt = std::fs::read_to_string(logs[0].as_ref().unwrap().path()).unwrap();
        assert!(txt.contains("hola"));
    }
}
