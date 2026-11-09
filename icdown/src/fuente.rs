//! De dónde vienen las fotos: el puente real o una fuente falsa para pruebas.
use crate::modelo::AssetIcloud;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum ErrorFuente {
    SesionCaducada,
    Credenciales(String),
    Cancelado,
    Otro(String),
}

impl fmt::Display for ErrorFuente {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorFuente::SesionCaducada => write!(f, "la sesión de iCloud caducó (ejecuta `icdown login`)"),
            ErrorFuente::Credenciales(d) => write!(f, "credenciales rechazadas: {d}"),
            ErrorFuente::Cancelado => write!(f, "cancelado"),
            ErrorFuente::Otro(d) => write!(f, "{d}"),
        }
    }
}

impl std::error::Error for ErrorFuente {}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct ResultadoBorrado {
    pub borrados: Vec<String>,
    /// (id, motivo)
    pub fallidos: Vec<(String, String)>,
}

pub trait FuenteIcloud: Send {
    /// Llama a `en_asset` por cada asset. Si devuelve `false`, se detiene con `Err(Cancelado)`.
    fn listar(&mut self, en_asset: &mut dyn FnMut(AssetIcloud) -> bool) -> Result<usize, ErrorFuente>;
    /// Vuelve a pedir solo estos ids del último listado (enlaces de descarga nuevos) y llama a
    /// `en_asset` con cada uno. Devuelve los que ya no están (borrados o no listados).
    fn refrescar(&mut self, ids: &[String], en_asset: &mut dyn FnMut(AssetIcloud) -> bool) -> Result<Vec<String>, ErrorFuente>;
    /// Borra (manda a "Eliminado recientemente") los ids del último listado.
    fn borrar(&mut self, ids: &[String]) -> Result<ResultadoBorrado, ErrorFuente>;
    /// Saca estos ids de "Eliminado recientemente" (en `borrados` van los restaurados).
    fn restaurar(&mut self, ids: &[String]) -> Result<ResultadoBorrado, ErrorFuente>;
}

/// Fuente en memoria para pruebas.
#[derive(Debug, Default, Clone)]
pub struct FuenteFalsa {
    pub assets: Vec<AssetIcloud>,
    pub borrados: Vec<String>,
    pub sesion_caducada: bool,
    pub rechazar: Vec<String>,
    /// Si es falso, `refrescar` falla (como un puente antiguo sin ese comando).
    pub renovar: bool,
    /// Versión que devuelve `refrescar` para estos ids; si no está, el asset de `assets` con `e` renovado.
    pub renovados: Vec<AssetIcloud>,
    /// Ids que `refrescar` da por desaparecidos.
    pub faltan: Vec<String>,
    /// Ids pedidos en cada llamada a `refrescar`.
    pub refrescos: Vec<Vec<String>>,
    /// "Eliminado recientemente": lo borrado, que se puede restaurar.
    pub papelera: Vec<AssetIcloud>,
    /// Llamadas a `borrar` (desde 0) en las que Apple "pide esperar": todo el lote vuelve con `limite_apple`.
    pub frenar_llamadas: Vec<u32>,
    /// Ids que la primera vez responden "reintentar" (TRY_AGAIN_LATER) y luego se borran.
    pub ocupados_una_vez: Vec<String>,
    /// Cuántas veces se llamó a `borrar`.
    pub llamadas_borrar: u32,
}

/// Cambia el parámetro `e=` (caducidad) de un enlace.
pub fn con_caducidad_nueva(url: &str, e: i64) -> String {
    let (base, query) = url.split_once('?').unwrap_or((url, ""));
    let mut kv: Vec<String> = query.split('&').filter(|x| !x.is_empty() && !x.starts_with("e=")).map(String::from).collect();
    kv.push(format!("e={e}"));
    format!("{base}?{}", kv.join("&"))
}

impl FuenteIcloud for FuenteFalsa {
    fn listar(&mut self, en_asset: &mut dyn FnMut(AssetIcloud) -> bool) -> Result<usize, ErrorFuente> {
        if self.sesion_caducada {
            return Err(ErrorFuente::SesionCaducada);
        }
        for a in self.assets.clone() {
            if !en_asset(a) {
                return Err(ErrorFuente::Cancelado);
            }
        }
        Ok(self.assets.len())
    }

    fn refrescar(&mut self, ids: &[String], en_asset: &mut dyn FnMut(AssetIcloud) -> bool) -> Result<Vec<String>, ErrorFuente> {
        self.refrescos.push(ids.to_vec());
        if self.sesion_caducada {
            return Err(ErrorFuente::SesionCaducada);
        }
        if !self.renovar {
            return Err(ErrorFuente::Otro("comando desconocido: refrescar".into()));
        }
        let e = chrono::Utc::now().timestamp() + 3600;
        let mut faltan = vec![];
        for id in ids {
            let nuevo = if self.faltan.contains(id) {
                None
            } else if let Some(a) = self.renovados.iter().find(|a| &a.id == id) {
                Some(a.clone())
            } else {
                self.assets.iter().find(|a| &a.id == id).map(|a| {
                    let mut a = a.clone();
                    for p in &mut a.partes {
                        p.url = con_caducidad_nueva(&p.url, e);
                    }
                    a
                })
            };
            match nuevo {
                Some(a) => {
                    if !en_asset(a) {
                        return Err(ErrorFuente::Cancelado);
                    }
                }
                None => faltan.push(id.clone()),
            }
        }
        Ok(faltan)
    }

    fn borrar(&mut self, ids: &[String]) -> Result<ResultadoBorrado, ErrorFuente> {
        if self.sesion_caducada {
            return Err(ErrorFuente::SesionCaducada);
        }
        let llamada = self.llamadas_borrar;
        self.llamadas_borrar += 1;
        let mut r = ResultadoBorrado::default();
        if self.frenar_llamadas.contains(&llamada) {
            r.fallidos = ids.iter().map(|i| (i.clone(), "limite_apple".into())).collect();
            return Ok(r);
        }
        for id in ids {
            if let Some(k) = self.ocupados_una_vez.iter().position(|x| x == id) {
                self.ocupados_una_vez.remove(k);
                r.fallidos.push((id.clone(), "reintentar".into()));
            } else if self.rechazar.contains(id) {
                r.fallidos.push((id.clone(), "reintentar".into()));
            } else {
                self.borrados.push(id.clone());
                if let Some(i) = self.assets.iter().position(|a| &a.id == id) {
                    self.papelera.push(self.assets.remove(i));
                }
                r.borrados.push(id.clone());
            }
        }
        Ok(r)
    }

    fn restaurar(&mut self, ids: &[String]) -> Result<ResultadoBorrado, ErrorFuente> {
        if self.sesion_caducada {
            return Err(ErrorFuente::SesionCaducada);
        }
        let mut r = ResultadoBorrado::default();
        for id in ids {
            match self.papelera.iter().position(|a| &a.id == id) {
                Some(i) => {
                    self.assets.push(self.papelera.remove(i));
                    r.borrados.push(id.clone());
                }
                None => r.fallidos.push((id.clone(), "no_en_eliminado_recientemente".into())),
            }
        }
        Ok(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(id: &str) -> AssetIcloud {
        AssetIcloud {
            id: id.into(),
            biblioteca: "PrimarySync".into(),
            nombre_base: id.into(),
            fecha: "2026-08-10T19:37:18-04:00".into(),
            fecha_agregada: None,
            liberable: true,
            partes: vec![],
        }
    }

    #[test]
    fn falsa_lista_y_cancela() {
        let mut f = FuenteFalsa { assets: vec![asset("A"), asset("B")], ..Default::default() };
        let mut vistos = vec![];
        assert_eq!(f.listar(&mut |a| { vistos.push(a.id); true }), Ok(2));
        assert_eq!(vistos, vec!["A", "B"]);
        assert_eq!(f.listar(&mut |_| false), Err(ErrorFuente::Cancelado));
    }

    #[test]
    fn falsa_borra_y_rechaza() {
        let mut f = FuenteFalsa { assets: vec![asset("A"), asset("B")], rechazar: vec!["B".into()], ..Default::default() };
        let r = f.borrar(&["A".into(), "B".into()]).unwrap();
        assert_eq!(r.borrados, vec!["A"]);
        assert_eq!(r.fallidos, vec![("B".to_string(), "reintentar".to_string())]);
        assert_eq!(f.assets.len(), 1);
        let r = f.restaurar(&["A".into(), "B".into()]).unwrap();
        assert_eq!(r.borrados, vec!["A"]);
        assert_eq!(r.fallidos, vec![("B".to_string(), "no_en_eliminado_recientemente".to_string())]);
        assert_eq!(f.assets.len(), 2);
        assert!(f.papelera.is_empty());
    }

    #[test]
    fn falsa_refresca_con_enlace_nuevo_y_faltan() {
        let mut a = asset("A");
        a.partes.push(crate::modelo::ParteIcloud {
            tipo: crate::modelo::TipoParte::Original,
            tamano: 1,
            url: "http://x/a?o=1&e=5&v=1".into(),
            extension: "JPG".into(),
            checksum: "c".into(),
        });
        let mut f = FuenteFalsa { assets: vec![a, asset("B")], faltan: vec!["B".into()], ..Default::default() };
        assert!(f.refrescar(&["A".into()], &mut |_| true).is_err(), "sin `renovar` falla");
        f.renovar = true;
        let mut vistos = vec![];
        let faltan = f.refrescar(&["A".into(), "B".into(), "Z".into()], &mut |a| { vistos.push(a); true }).unwrap();
        assert_eq!(faltan, vec!["B", "Z"]);
        assert_eq!(vistos.len(), 1);
        let url = &vistos[0].partes[0].url;
        assert!(url.starts_with("http://x/a?o=1&v=1&e="), "{url}");
        assert!(crate::motor::caducidad_url(url).unwrap() > 5);
        assert_eq!(f.refrescos.len(), 2);
    }

    #[test]
    fn falsa_caducada() {
        let mut f = FuenteFalsa { sesion_caducada: true, ..Default::default() };
        assert_eq!(f.listar(&mut |_| true), Err(ErrorFuente::SesionCaducada));
    }
}
