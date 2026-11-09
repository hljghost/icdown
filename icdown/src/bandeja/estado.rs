//! Estado que muestra el icono: texto del menú/tooltip y color.
use crate::registro::Progreso;

#[derive(Debug, Clone, PartialEq)]
pub enum Fase {
    Inactiva,
    Listando { vistos: usize },
    Copiando { hechas: usize, total: usize },
    Verificando { hechas: usize, total: usize },
    Pausada,
    SesionCaducada,
    SinEspacio,
    DestinoAusente,
    SinDestino,
    Error(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct EstadoUi {
    pub fase: Fase,
    pub verificadas: u64,
    pub total: u64,
    pub detalle: Detalle,
}

/// Datos extra para el panel de progreso.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Detalle {
    pub bytes_verificados: u64,
    pub bytes_total: u64,
    pub con_error: u64,
    /// Fin del último listado completo de iCloud (unix).
    pub ultimo_listado: Option<i64>,
    /// Archivo que se está copiando ahora.
    pub actual: Option<String>,
    /// Trabajo de la ronda en curso (desde que acabó el listado).
    pub ronda: Option<Ronda>,
    /// Cuándo está prevista la próxima ronda (unix), si se sabe.
    pub proxima: Option<i64>,
    /// El listado en curso es para pedir enlaces nuevos (los anteriores caducaron).
    pub renovando_enlaces: bool,
    /// Mientras se lista: cuántos elementos trajo el listado completo anterior.
    pub listado_previsto: Option<u64>,
}

/// Lo que tiene que copiar la ronda actual, para medir su avance aparte del total.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ronda {
    pub archivos: u64,
    pub bytes: u64,
    /// Bytes verificados de toda la fototeca al empezar a copiar.
    pub bytes_base: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EventoUi {
    Estado(EstadoUi),
    Aviso { titulo: String, texto: String },
    /// Para el asistente de liberar espacio.
    Liberar(crate::bandeja::asistente::EventoLiberar),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Gris,
    Azul,
    Naranja,
    Rojo,
}

pub use crate::i18n::miles;
use crate::i18n::{t, tf, T};

impl EstadoUi {
    pub fn sin_datos(fase: Fase) -> EstadoUi {
        EstadoUi { fase, verificadas: 0, total: 0, detalle: Detalle::default() }
    }

    pub fn con_progreso(fase: Fase, p: &Progreso) -> EstadoUi {
        EstadoUi {
            fase,
            verificadas: p.verificadas,
            total: p.partes,
            detalle: Detalle {
                bytes_verificados: p.bytes_verificados,
                bytes_total: p.bytes_total,
                con_error: p.con_error,
                ultimo_listado: p.ultimo_listado,
                actual: None,
                ronda: None,
                proxima: None,
                renovando_enlaces: false,
                listado_previsto: None,
            },
        }
    }

    /// Recalcula los totales con una lectura fresca del registro (sin tocar fase ni ronda).
    pub fn actualizar_progreso(&mut self, p: &Progreso) {
        self.verificadas = p.verificadas;
        self.total = p.partes;
        let d = &mut self.detalle;
        d.bytes_verificados = p.bytes_verificados;
        d.bytes_total = p.bytes_total;
        d.con_error = p.con_error;
        d.ultimo_listado = p.ultimo_listado;
    }

    /// Hay una ronda en marcha (listando, copiando o verificando).
    pub fn trabajando(&self) -> bool {
        matches!(self.fase, Fase::Listando { .. } | Fase::Copiando { .. } | Fase::Verificando { .. })
    }

    /// Porcentaje copiado y verificado (por bytes si se conocen; si no, por archivos).
    pub fn porcentaje(&self) -> Option<u64> {
        let d = &self.detalle;
        (d.bytes_verificados * 100)
            .checked_div(d.bytes_total)
            .or_else(|| (self.verificadas * 100).checked_div(self.total))
    }

    fn progreso(&self) -> String {
        if self.total == 0 {
            t(T::EstNada).into()
        } else {
            tf(T::EstArchivos, &[("v", &miles(self.verificadas)), ("t", &miles(self.total)), ("p", &(self.verificadas * 100 / self.total))])
        }
    }

    /// Texto corto (cabe en el tooltip de Windows, 127 caracteres).
    pub fn texto(&self) -> String {
        let texto = match &self.fase {
            Fase::Inactiva if self.total > 0 && self.verificadas == self.total => tf(T::EstAlDia, &[("p", &self.progreso())]),
            Fase::Inactiva => tf(T::EstEspera, &[("p", &self.progreso())]),
            Fase::Listando { vistos } if self.detalle.renovando_enlaces => tf(T::TitRenovandoN, &[("n", &miles(*vistos as u64))]),
            Fase::Listando { vistos } => tf(T::TitRevisandoN, &[("n", &miles(*vistos as u64))]),
            Fase::Copiando { hechas, total } => {
                let pct = self.porcentaje().map(|p| tf(T::EstDelTotal, &[("p", &p)])).unwrap_or_default();
                tf(T::EstCopiando, &[("h", &miles(*hechas as u64)), ("t", &miles(*total as u64)), ("pct", &pct)])
            }
            Fase::Verificando { hechas, total } => tf(T::EstVerificando, &[("h", &miles(*hechas as u64)), ("t", &miles(*total as u64))]),
            Fase::Pausada => tf(T::EstPausa, &[("p", &self.progreso())]),
            Fase::SesionCaducada => t(T::EstSesion).into(),
            Fase::SinEspacio => t(T::EstSinEspacio).into(),
            Fase::DestinoAusente => t(T::EstSinDestino).into(),
            Fase::SinDestino => t(T::EstElige).into(),
            Fase::Error(m) => tf(T::EstError, &[("m", m)]),
        };
        texto.chars().take(110).collect()
    }

    pub fn color(&self) -> Color {
        match self.fase {
            Fase::Listando { .. } | Fase::Copiando { .. } | Fase::Verificando { .. } => Color::Azul,
            Fase::Inactiva | Fase::Pausada => Color::Gris,
            Fase::SesionCaducada | Fase::SinEspacio | Fase::DestinoAusente | Fase::SinDestino => Color::Naranja,
            Fase::Error(_) => Color::Rojo,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porcentaje_por_bytes_y_si_no_por_archivos() {
        let mut e = EstadoUi::sin_datos(Fase::Inactiva);
        assert_eq!(e.porcentaje(), None, "sin datos");
        e.verificadas = 1;
        e.total = 4;
        assert_eq!(e.porcentaje(), Some(25), "por archivos");
        e.detalle.bytes_verificados = 3;
        e.detalle.bytes_total = 4;
        assert_eq!(e.porcentaje(), Some(75), "los bytes mandan si se conocen");
        e.detalle.bytes_total = 0;
        assert_eq!(e.porcentaje(), Some(25), "bytes sin total: vuelve a archivos");
    }

    #[test]
    fn formato_miles() {
        assert_eq!(miles(999), "999");
        assert_eq!(miles(12430), "12.430");
        assert_eq!(miles(1_000_000), "1.000.000");
    }

    #[test]
    fn textos_y_colores() {
        let al_dia = EstadoUi { fase: Fase::Inactiva, verificadas: 10, total: 10, detalle: Detalle::default() };
        assert_eq!(al_dia.texto(), "Al día — 10 de 10 archivos (100%)");
        assert_eq!(al_dia.color(), Color::Gris);
        let mut copiando = EstadoUi::sin_datos(Fase::Copiando { hechas: 3, total: 1200 });
        assert_eq!(copiando.texto(), "Copiando 3 de 1.200…");
        copiando.detalle.bytes_total = 200;
        copiando.detalle.bytes_verificados = 103;
        assert_eq!(copiando.texto(), "Copiando 3 de 1.200… · 51 % del total");
        assert_eq!(copiando.color(), Color::Azul);
        assert_eq!(EstadoUi::sin_datos(Fase::SesionCaducada).color(), Color::Naranja);
        assert_eq!(EstadoUi::sin_datos(Fase::Error("x".into())).color(), Color::Rojo);
        let largo = EstadoUi::sin_datos(Fase::Error("x".repeat(500)));
        assert!(largo.texto().chars().count() <= 110);
    }
}
