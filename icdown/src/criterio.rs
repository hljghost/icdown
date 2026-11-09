//! Qué se libera de iCloud: conservar los últimos N días (mínimo 30) o elegir años de captura.
//! La selección se calcula al instante sobre el inventario local (el registro); la definitiva la
//! hace `liberar::preparar` tras volver a listar iCloud y re-verificar en disco.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Nunca se libera nada capturado ni añadido a iCloud en los últimos 30 días.
pub const MIN_DIAS: u32 = 30;
/// Opciones de "conservar los últimos…": 30 días, 3, 6 y 12 meses, 2 y 3 años.
pub const PRESETS_DIAS: [u32; 6] = [30, 90, 180, 365, 730, 1095];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Criterio {
    /// Conservar en iCloud lo de los últimos N días (se aplica al menos `MIN_DIAS`).
    ConservarDias(u32),
    /// Liberar lo capturado en estos años (sin tocar nunca lo de los últimos `MIN_DIAS` días).
    Anios(BTreeSet<i32>),
}

impl Criterio {
    /// Días que se conservan como mínimo (nunca menos de `MIN_DIAS`).
    pub fn dias(&self) -> u32 {
        match self {
            Criterio::ConservarDias(d) => (*d).max(MIN_DIAS),
            Criterio::Anios(_) => MIN_DIAS,
        }
    }

    /// Límite (unix): solo se libera lo capturado y añadido antes.
    pub fn limite(&self, ahora: i64) -> i64 {
        ahora - i64::from(self.dias()) * 86_400
    }

    /// Años elegidos (`None` = todos).
    pub fn anios(&self) -> Option<&BTreeSet<i32>> {
        match self {
            Criterio::Anios(a) => Some(a),
            Criterio::ConservarDias(_) => None,
        }
    }

    pub fn vacio(&self) -> bool {
        matches!(self, Criterio::Anios(a) if a.is_empty())
    }
}

/// "2011-2019,2021,2023-2025" → esos años (rangos con guion, ambos incluidos).
pub fn parsear_anios(texto: &str) -> Result<BTreeSet<i32>, String> {
    let mut v = BTreeSet::new();
    for trozo in texto.split(',').map(str::trim).filter(|x| !x.is_empty()) {
        let num = |x: &str| x.trim().parse::<i32>().ok().filter(|a| (1900..=2200).contains(a)).ok_or_else(|| format!("año no válido: «{x}»"));
        match trozo.split_once('-') {
            Some((a, b)) => {
                let (a, b) = (num(a)?, num(b)?);
                if a > b {
                    return Err(format!("rango al revés: «{trozo}»"));
                }
                v.extend(a..=b);
            }
            None => {
                v.insert(num(trozo)?);
            }
        }
    }
    if v.is_empty() {
        return Err("no hay ningún año".into());
    }
    Ok(v)
}

/// Todos los años de captura hasta `anio` incluido (para `--hasta-anio`).
pub fn hasta_anio(anio: i32) -> BTreeSet<i32> {
    (1900..=anio).collect()
}

/// Un elemento de iCloud según el registro (último listado, aún no borrado).
#[derive(Debug, Clone, PartialEq)]
pub struct FilaInventario {
    /// Año de captura (hora local de la captura, como las carpetas).
    pub anio: i32,
    pub fecha_unix: i64,
    pub agregado_unix: Option<i64>,
    /// De la biblioteca personal (no de la Fototeca Compartida).
    pub personal: bool,
    /// icdown sabe copiar todo lo que tiene en iCloud.
    pub liberable: bool,
    /// Todas sus partes copiadas y verificadas.
    pub completo: bool,
    pub bytes: u64,
    /// Alguna parte con estructura atípica confirmada por doble descarga.
    pub atipico: bool,
}

impl FilaInventario {
    fn reciente(&self, limite: i64) -> bool {
        self.fecha_unix.max(self.agregado_unix.unwrap_or(self.fecha_unix)) >= limite
    }
}

/// Resultado de aplicar un criterio al inventario.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Seleccion {
    pub elementos: u64,
    pub bytes: u64,
    /// Fecha de captura del más antiguo y del más nuevo seleccionados (unix).
    pub desde: Option<i64>,
    pub hasta: Option<i64>,
    /// En el rango elegido pero excluidos, por motivo (cada elemento cuenta en el primero que aplique).
    pub compartida: u64,
    pub no_liberables: u64,
    pub no_copiados: u64,
    pub recientes: u64,
    /// De los seleccionados, con estructura atípica confirmada (sí se liberan).
    pub atipicos: u64,
}

/// Por qué no se libera un elemento del rango elegido (`None` = se libera).
fn exclusion(f: &FilaInventario, limite: i64) -> Option<fn(&mut Seleccion) -> &mut u64> {
    if !f.personal {
        Some(|s| &mut s.compartida)
    } else if !f.liberable {
        Some(|s| &mut s.no_liberables)
    } else if !f.completo {
        Some(|s| &mut s.no_copiados)
    } else if f.reciente(limite) {
        Some(|s| &mut s.recientes)
    } else {
        None
    }
}

/// Aplica el criterio. El rango es por fecha de captura (antes del límite, o en los años elegidos);
/// dentro de él se excluye lo compartido, lo no liberable, lo no copiado y lo añadido hace poco.
pub fn seleccionar(inventario: &[FilaInventario], c: &Criterio, ahora: i64) -> Seleccion {
    let limite = c.limite(ahora);
    let mut s = Seleccion::default();
    for f in inventario {
        let en_rango = match c.anios() {
            Some(a) => a.contains(&f.anio),
            None => f.fecha_unix < limite,
        };
        if !en_rango {
            continue;
        }
        match exclusion(f, limite) {
            Some(campo) => *campo(&mut s) += 1,
            None => {
                s.elementos += 1;
                s.bytes += f.bytes;
                s.atipicos += u64::from(f.atipico);
                s.desde = Some(s.desde.map_or(f.fecha_unix, |d| d.min(f.fecha_unix)));
                s.hasta = Some(s.hasta.map_or(f.fecha_unix, |h| h.max(f.fecha_unix)));
            }
        }
    }
    s
}

/// Lo liberable de cada año de captura (para las casillas de "por años"), del más reciente al más antiguo.
#[derive(Debug, Clone, PartialEq)]
pub struct Anio {
    pub anio: i32,
    pub elementos: u64,
    pub bytes: u64,
    /// Total de elementos de ese año en iCloud (también los que no se liberarían).
    pub en_icloud: u64,
}

pub fn por_anios(inventario: &[FilaInventario], ahora: i64) -> Vec<Anio> {
    let limite = ahora - i64::from(MIN_DIAS) * 86_400;
    let mut mapa: std::collections::BTreeMap<i32, Anio> = Default::default();
    for f in inventario {
        let a = mapa.entry(f.anio).or_insert(Anio { anio: f.anio, elementos: 0, bytes: 0, en_icloud: 0 });
        a.en_icloud += 1;
        if exclusion(f, limite).is_none() {
            a.elementos += 1;
            a.bytes += f.bytes;
        }
    }
    mapa.into_values().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIA: i64 = 86_400;
    const AHORA: i64 = 2_000_000_000;

    fn fila(anio: i32, hace_dias: i64) -> FilaInventario {
        FilaInventario {
            anio,
            fecha_unix: AHORA - hace_dias * DIA,
            agregado_unix: None,
            personal: true,
            liberable: true,
            completo: true,
            bytes: 10,
            atipico: false,
        }
    }

    #[test]
    fn anios_en_texto_con_rangos() {
        assert_eq!(parsear_anios("2011-2013, 2021,2023-2024").unwrap().into_iter().collect::<Vec<_>>(), vec![2011, 2012, 2013, 2021, 2023, 2024]);
        assert!(parsear_anios("2019-2011").is_err());
        assert!(parsear_anios("abc").is_err());
        assert!(parsear_anios(" , ").is_err());
        let h = hasta_anio(2025);
        assert!(h.contains(&1990) && h.contains(&2025) && !h.contains(&2026));
        // Corte exacto por año local de captura: lo del 31 de diciembre de 2025 entra, lo del 1 de enero de 2026 no.
        let mut ultima = fila(2025, 400);
        ultima.fecha_unix = AHORA - 300 * DIA;
        let primera = fila(2026, 299);
        let s = seleccionar(&[ultima, primera], &Criterio::Anios(h), AHORA);
        assert_eq!(s.elementos, 1);
    }

    #[test]
    fn conservar_dias_respeta_el_minimo_de_30() {
        assert_eq!(Criterio::ConservarDias(7).dias(), 30);
        assert_eq!(Criterio::ConservarDias(365).dias(), 365);
        assert_eq!(Criterio::Anios(BTreeSet::new()).dias(), 30);
        let inv = [fila(2026, 10), fila(2026, 40), fila(2025, 400)];
        let s = seleccionar(&inv, &Criterio::ConservarDias(0), AHORA);
        assert_eq!((s.elementos, s.bytes), (2, 20), "lo de hace 10 días nunca");
        let s = seleccionar(&inv, &Criterio::ConservarDias(365), AHORA);
        assert_eq!(s.elementos, 1);
        assert_eq!((s.desde, s.hasta), (Some(AHORA - 400 * DIA), Some(AHORA - 400 * DIA)));
    }

    #[test]
    fn exclusiones_con_motivo() {
        let mut compartida = fila(2020, 2000);
        compartida.personal = false;
        let mut rara = fila(2020, 2000);
        rara.liberable = false;
        let mut sin_copiar = fila(2020, 2000);
        sin_copiar.completo = false;
        let mut agregada = fila(2020, 2000);
        agregada.agregado_unix = Some(AHORA - 5 * DIA);
        let mut atipica = fila(2020, 2000);
        atipica.atipico = true;
        let inv = [compartida, rara, sin_copiar, agregada, atipica, fila(2019, 2500)];
        let s = seleccionar(&inv, &Criterio::ConservarDias(30), AHORA);
        assert_eq!((s.elementos, s.compartida, s.no_liberables, s.no_copiados, s.recientes, s.atipicos), (2, 1, 1, 1, 1, 1));
        assert_eq!(s.desde, Some(AHORA - 2500 * DIA));
        // Agregada hace poco: se conserva aunque la foto sea vieja, incluso con "conservar 30 días".
        let s = seleccionar(&inv, &Criterio::ConservarDias(3 * 365), AHORA);
        assert_eq!((s.elementos, s.recientes), (2, 1));
    }

    #[test]
    fn por_anios_elige_solo_esos_y_nunca_lo_reciente() {
        let inv = [fila(2026, 5), fila(2026, 60), fila(2024, 700), fila(2023, 1100), fila(2023, 1150)];
        let anios = |v: &[i32]| Criterio::Anios(v.iter().copied().collect());
        let s = seleccionar(&inv, &anios(&[2023]), AHORA);
        assert_eq!((s.elementos, s.bytes), (2, 20));
        let s = seleccionar(&inv, &anios(&[2026]), AHORA);
        assert_eq!((s.elementos, s.recientes), (1, 1), "lo de hace 5 días se conserva");
        let vacio = anios(&[]);
        assert!(vacio.vacio());
        assert_eq!(seleccionar(&inv, &vacio, AHORA).elementos, 0);
        let filas = por_anios(&inv, AHORA);
        assert_eq!(filas.iter().map(|a| (a.anio, a.elementos, a.en_icloud)).collect::<Vec<_>>(), vec![(2026, 1, 2), (2024, 1, 1), (2023, 2, 2)]);
    }
}
