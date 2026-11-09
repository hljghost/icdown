//! Flujo de inicio de sesión (contraseña + código 2FA), común a `icdown login` y a `icdown-login`.
use crate::i18n::{t, tf, T};
use crate::fuente::ErrorFuente;
use crate::puente::Puente;

/// Códigos incorrectos que se toleran antes de pedir que se empiece de nuevo.
pub const MAX_FALLOS_CODIGO: u32 = 3;

/// Lo que el flujo necesita del puente (permite probarlo con un falso).
pub trait PuenteLogin {
    /// `true` si además hace falta el código 2FA.
    fn login(&mut self, password: &str) -> Result<bool, ErrorFuente>;
    fn codigo_2fa(&mut self, codigo: &str) -> Result<(), ErrorFuente>;
    fn sesion(&mut self) -> Result<(), ErrorFuente>;
}

impl PuenteLogin for Puente {
    fn login(&mut self, password: &str) -> Result<bool, ErrorFuente> {
        Puente::login(self, password)
    }
    fn codigo_2fa(&mut self, codigo: &str) -> Result<(), ErrorFuente> {
        Puente::codigo_2fa(self, codigo)
    }
    fn sesion(&mut self) -> Result<(), ErrorFuente> {
        Puente::sesion(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Paso {
    /// Hace falta el código de 6 dígitos.
    PideCodigo,
    /// Sesión iniciada y comprobada.
    Hecho,
    /// Error en el paso actual; se puede volver a intentar.
    Reintentar(String),
    /// Error que obliga a empezar de nuevo (contraseña).
    Fallo(String),
}

/// "123 456" → Some("123456"); cualquier otra cosa que no sean 6 dígitos → None.
pub fn normalizar_codigo(texto: &str) -> Option<String> {
    let c: String = texto.chars().filter(|c| !c.is_whitespace()).collect();
    (c.len() == 6 && c.chars().all(|c| c.is_ascii_digit())).then_some(c)
}

/// Resultado de comprobar la sesión guardada antes de pedir la contraseña.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EstadoSesion {
    Valida,
    Caducada,
    /// No se pudo comprobar (red, puente…): se ofrece iniciar sesión igualmente.
    Desconocida(String),
}

pub fn comprobar_sesion<P: PuenteLogin>(puente: &mut P) -> EstadoSesion {
    match puente.sesion() {
        Ok(()) => EstadoSesion::Valida,
        Err(ErrorFuente::SesionCaducada) => EstadoSesion::Caducada,
        Err(e) => EstadoSesion::Desconocida(e.to_string()),
    }
}

pub struct FlujoLogin<P> {
    puente: P,
    fallos: u32,
}

impl<P: PuenteLogin> FlujoLogin<P> {
    pub fn new(puente: P) -> Self {
        FlujoLogin { puente, fallos: 0 }
    }

    pub fn entrar(&mut self, password: &str) -> Paso {
        match self.puente.login(password) {
            Ok(true) => Paso::PideCodigo,
            Ok(false) => self.comprobar(),
            Err(ErrorFuente::Credenciales(_)) => Paso::Reintentar(t(T::LogIncorrectos).into()),
            Err(e) => Paso::Reintentar(tf(T::LogSinConexion, &[("e", &e)])),
        }
    }

    pub fn verificar(&mut self, codigo: &str) -> Paso {
        let Some(codigo) = normalizar_codigo(codigo) else {
            return Paso::Reintentar(t(T::LogSeisDigitos).trim_end_matches('.').into());
        };
        match self.puente.codigo_2fa(&codigo) {
            Ok(()) => self.comprobar(),
            Err(ErrorFuente::Credenciales(_)) => {
                self.fallos += 1;
                if self.fallos >= MAX_FALLOS_CODIGO {
                    Paso::Fallo(t(T::LogDemasiados).into())
                } else {
                    Paso::Reintentar(t(T::LogCodigoMal).into())
                }
            }
            Err(e) => Paso::Fallo(tf(T::LogNoVerificado, &[("e", &e)])),
        }
    }

    fn comprobar(&mut self) -> Paso {
        match self.puente.sesion() {
            Ok(()) => Paso::Hecho,
            Err(e) => Paso::Fallo(tf(T::LogNoValida, &[("e", &e)])),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[derive(Default)]
    struct Falso {
        login: VecDeque<Result<bool, ErrorFuente>>,
        codigos: VecDeque<Result<(), ErrorFuente>>,
        sesion: VecDeque<Result<(), ErrorFuente>>,
        enviados: Vec<String>,
    }

    impl PuenteLogin for Falso {
        fn login(&mut self, _password: &str) -> Result<bool, ErrorFuente> {
            self.login.pop_front().expect("login inesperado")
        }
        fn codigo_2fa(&mut self, codigo: &str) -> Result<(), ErrorFuente> {
            self.enviados.push(codigo.to_string());
            self.codigos.pop_front().expect("código inesperado")
        }
        fn sesion(&mut self) -> Result<(), ErrorFuente> {
            self.sesion.pop_front().expect("sesión inesperada")
        }
    }

    fn mal() -> ErrorFuente {
        ErrorFuente::Credenciales("x".into())
    }

    #[test]
    fn normaliza_codigo() {
        assert_eq!(normalizar_codigo("123456").as_deref(), Some("123456"));
        assert_eq!(normalizar_codigo(" 123 456 ").as_deref(), Some("123456"));
        assert_eq!(normalizar_codigo("12345"), None);
        assert_eq!(normalizar_codigo("1234567"), None);
        assert_eq!(normalizar_codigo("12a456"), None);
        assert_eq!(normalizar_codigo("١٢٣٤٥٦"), None);
        assert_eq!(normalizar_codigo(""), None);
    }

    #[test]
    fn sin_2fa_comprueba_sesion() {
        let f = Falso { login: [Ok(false)].into(), sesion: [Ok(())].into(), ..Default::default() };
        assert_eq!(FlujoLogin::new(f).entrar("pw"), Paso::Hecho);
    }

    #[test]
    fn contrasena_incorrecta() {
        let f = Falso { login: [Err(mal())].into(), ..Default::default() };
        assert_eq!(FlujoLogin::new(f).entrar("pw"), Paso::Reintentar("Apple ID o contraseña incorrectos".into()));
    }

    #[test]
    fn con_2fa_y_codigo_bueno() {
        let f = Falso { login: [Ok(true)].into(), codigos: [Ok(())].into(), sesion: [Ok(())].into(), ..Default::default() };
        let mut fl = FlujoLogin::new(f);
        assert_eq!(fl.entrar("pw"), Paso::PideCodigo);
        assert_eq!(fl.verificar("123 456"), Paso::Hecho);
        assert_eq!(fl.puente.enviados, vec!["123456"]);
    }

    #[test]
    fn codigo_mal_formado_no_llega_al_puente_ni_cuenta() {
        let f = Falso { login: [Ok(true)].into(), codigos: [Ok(())].into(), sesion: [Ok(())].into(), ..Default::default() };
        let mut fl = FlujoLogin::new(f);
        fl.entrar("pw");
        for _ in 0..5 {
            assert!(matches!(fl.verificar("12"), Paso::Reintentar(_)));
        }
        assert!(fl.puente.enviados.is_empty());
        assert_eq!(fl.verificar("123456"), Paso::Hecho);
    }

    #[test]
    fn reintenta_codigo_y_al_final_falla() {
        let f = Falso { login: [Ok(true)].into(), codigos: [Err(mal()), Err(mal()), Err(mal())].into(), ..Default::default() };
        let mut fl = FlujoLogin::new(f);
        fl.entrar("pw");
        assert!(matches!(fl.verificar("111111"), Paso::Reintentar(_)));
        assert!(matches!(fl.verificar("222222"), Paso::Reintentar(_)));
        assert!(matches!(fl.verificar("333333"), Paso::Fallo(_)));
    }

    #[test]
    fn codigo_bueno_tras_un_fallo() {
        let f = Falso {
            login: [Ok(true)].into(),
            codigos: [Err(mal()), Ok(())].into(),
            sesion: [Ok(())].into(),
            ..Default::default()
        };
        let mut fl = FlujoLogin::new(f);
        fl.entrar("pw");
        assert!(matches!(fl.verificar("111111"), Paso::Reintentar(_)));
        assert_eq!(fl.verificar("222222"), Paso::Hecho);
    }

    #[test]
    fn comprobar_sesion_guardada() {
        let mut f = Falso { sesion: [Ok(()), Err(ErrorFuente::SesionCaducada), Err(ErrorFuente::Otro("sin red".into()))].into(), ..Default::default() };
        assert_eq!(comprobar_sesion(&mut f), EstadoSesion::Valida);
        assert_eq!(comprobar_sesion(&mut f), EstadoSesion::Caducada);
        assert!(matches!(comprobar_sesion(&mut f), EstadoSesion::Desconocida(m) if m.contains("sin red")));
    }

    #[test]
    fn sesion_invalida_al_final_es_fallo() {
        let f = Falso { login: [Ok(false)].into(), sesion: [Err(ErrorFuente::SesionCaducada)].into(), ..Default::default() };
        assert!(matches!(FlujoLogin::new(f).entrar("pw"), Paso::Fallo(_)));
    }

    #[test]
    fn error_de_red_en_codigo_es_fallo() {
        let f = Falso { login: [Ok(true)].into(), codigos: [Err(ErrorFuente::Otro("red".into()))].into(), ..Default::default() };
        let mut fl = FlujoLogin::new(f);
        fl.entrar("pw");
        assert!(matches!(fl.verificar("123456"), Paso::Fallo(_)));
    }
}
