//! Cuándo toca la siguiente ronda. `None` = esperar una orden del usuario.
use crate::motor::{FinRonda, ResultadoRonda};
use std::time::Duration;

pub fn espera_tras_ronda(r: &ResultadoRonda, proximo_reintento: Option<i64>, ahora: i64, intervalo_horas: u64) -> Option<Duration> {
    let intervalo = Duration::from_secs(intervalo_horas * 3600);
    match r.fin {
        FinRonda::Relistar if r.verificadas > 0 => Some(Duration::ZERO),
        FinRonda::Relistar => Some(Duration::from_secs(60)),
        FinRonda::SesionCaducada | FinRonda::Cancelada => None,
        FinRonda::SinEspacio | FinRonda::DestinoAusente => Some(Duration::from_secs(600)),
        FinRonda::Completa => match proximo_reintento {
            Some(t) => Some(Duration::from_secs((t - ahora).max(5) as u64).min(intervalo)),
            None => Some(intervalo),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(fin: FinRonda, verificadas: usize) -> ResultadoRonda {
        ResultadoRonda { fin, listados: 0, verificadas, fallidas: 0, aplazadas: 0 }
    }

    #[test]
    fn casos() {
        let s = Duration::from_secs;
        assert_eq!(espera_tras_ronda(&r(FinRonda::Relistar, 3), None, 0, 4), Some(Duration::ZERO));
        assert_eq!(espera_tras_ronda(&r(FinRonda::Relistar, 0), None, 0, 4), Some(s(60)));
        assert_eq!(espera_tras_ronda(&r(FinRonda::SesionCaducada, 0), None, 0, 4), None);
        assert_eq!(espera_tras_ronda(&r(FinRonda::Cancelada, 0), None, 0, 4), None);
        assert_eq!(espera_tras_ronda(&r(FinRonda::SinEspacio, 0), None, 0, 4), Some(s(600)));
        assert_eq!(espera_tras_ronda(&r(FinRonda::Completa, 0), None, 0, 4), Some(s(4 * 3600)));
        assert_eq!(espera_tras_ronda(&r(FinRonda::Completa, 0), Some(1030), 1000, 4), Some(s(30)));
        assert_eq!(espera_tras_ronda(&r(FinRonda::Completa, 0), Some(500), 1000, 4), Some(s(5)));
        assert_eq!(espera_tras_ronda(&r(FinRonda::Completa, 0), Some(100_000), 0, 4), Some(s(4 * 3600)));
    }
}
