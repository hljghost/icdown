use icdown::fuente::{ErrorFuente, FuenteIcloud};
use icdown::puente::Puente;
use std::process::Command;

fn falso(args: &[&str]) -> Puente {
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/datos/puente_falso.py");
    let mut cmd = Command::new("python");
    cmd.arg(script);
    for a in args {
        cmd.arg(a);
    }
    let base = std::env::temp_dir().join(format!("icdown-puente-{}", std::process::id()));
    Puente::iniciar_con(cmd, "yo@ejemplo.com", &base.join("sesion"), &base.join("puente.log")).unwrap()
}

#[test]
fn lista_assets_e_ignora_avisos() {
    let mut p = falso(&[]);
    let mut vistos = vec![];
    let n = p.listar(&mut |a| { vistos.push(a.id); true }).unwrap();
    assert_eq!(n, 2);
    assert_eq!(vistos, vec!["A1", "A2"]);
}

#[test]
fn cancelar_listado() {
    let mut p = falso(&[]);
    assert_eq!(p.listar(&mut |_| false), Err(ErrorFuente::Cancelado));
}

#[test]
fn borrar_reporta_fallidos() {
    let mut p = falso(&[]);
    let r = p.borrar(&["A1".into(), "MALO".into()]).unwrap();
    assert_eq!(r.borrados, vec!["A1"]);
    assert_eq!(r.fallidos, vec![("MALO".to_string(), "reintentar".to_string())]);
}

#[test]
fn restaurar_reporta_restaurados_y_fallidos() {
    let mut p = falso(&[]);
    let r = p.restaurar(&["A1".into(), "PERDIDO".into()]).unwrap();
    assert_eq!(r.borrados, vec!["A1"]);
    assert_eq!(r.fallidos, vec![("PERDIDO".to_string(), "no_en_eliminado_recientemente".to_string())]);
}

#[test]
fn refrescar_emite_assets_y_faltan() {
    let mut p = falso(&[]);
    let mut vistos = vec![];
    let faltan = p.refrescar(&["A1".into(), "BORRADO".into()], &mut |a| { vistos.push(a.id); true }).unwrap();
    assert_eq!(vistos, vec!["A1"]);
    assert_eq!(faltan, vec!["BORRADO"]);
    // El puente sigue atendiendo después.
    assert_eq!(p.listar(&mut |_| true), Ok(2));
}

#[test]
fn refrescar_con_sesion_caducada() {
    let mut p = falso(&["--caducada"]);
    assert_eq!(p.refrescar(&["A1".into()], &mut |_| true), Err(ErrorFuente::SesionCaducada));
}

#[test]
fn sesion_caducada() {
    let mut p = falso(&["--caducada"]);
    assert_eq!(p.sesion(), Err(ErrorFuente::SesionCaducada));
    assert_eq!(p.listar(&mut |_| true), Err(ErrorFuente::SesionCaducada));
}

#[test]
fn login_con_2fa() {
    let mut p = falso(&[]);
    assert_eq!(p.login("x"), Ok(true));
    assert!(matches!(p.codigo_2fa("000000"), Err(ErrorFuente::Credenciales(_))));
    assert_eq!(p.codigo_2fa("123456"), Ok(()));
}
