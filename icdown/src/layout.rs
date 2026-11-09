//! Dónde va cada archivo: <destino>\AAAA\AAAA-MM\<nombre>.
use crate::modelo::TipoParte;
use chrono::{DateTime, Datelike, FixedOffset};
use std::path::{Path, PathBuf};

pub fn sanear(nombre: &str) -> String {
    let s: String = nombre
        .chars()
        .map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c })
        .collect();
    let s = s.trim().trim_end_matches('.').trim().to_string();
    if s.is_empty() { "sin_nombre".to_string() } else { s }
}

pub fn fecha(texto: &str) -> anyhow::Result<DateTime<FixedOffset>> {
    Ok(DateTime::parse_from_rfc3339(texto)?)
}

/// Ruta en disco de una ruta relativa del registro. En el registro se guardan siempre con `\`
/// (formato estable, el mismo disco sirve en Windows y en macOS); aquí se parten por componentes para
/// que en macOS sean carpetas y no un archivo con barras invertidas en el nombre. Los componentes nunca
/// contienen `\` ni `/` (`sanear` los sustituye).
pub fn en_disco(destino: &Path, rel: &str) -> PathBuf {
    rel.split('\\').filter(|c| !c.is_empty()).fold(destino.to_path_buf(), |p, c| p.join(c))
}

/// ¿Existe en disco `rel` (en minúsculas, como la clave de `elegir_ruta`) ignorando mayúsculas?
/// En Windows el sistema de archivos ya lo hace; en Linux hay que comparar los nombres a mano.
pub fn existe_sin_mayusculas(destino: &Path, rel_min: &str) -> bool {
    let mut actual = destino.to_path_buf();
    for comp in rel_min.split('\\').filter(|c| !c.is_empty()) {
        let Ok(rd) = std::fs::read_dir(&actual) else { return false };
        let Some(e) = rd.flatten().find(|e| e.file_name().to_string_lossy().to_lowercase() == comp) else { return false };
        actual = e.path();
    }
    true
}

pub fn carpeta_relativa(f: &DateTime<FixedOffset>) -> String {
    format!("{:04}\\{:04}-{:02}", f.year(), f.year(), f.month())
}

pub fn mes_clave(f: &DateTime<FixedOffset>) -> String {
    format!("{:04}-{:02}", f.year(), f.month())
}

pub fn sufijo_de_id(id: &str) -> String {
    id.chars().filter(|c| c.is_ascii_alphanumeric()).take(6).collect::<String>().to_lowercase()
}

pub fn nombre_parte(nombre_base: &str, tipo: TipoParte, extension: &str, sufijo: Option<&str>) -> String {
    let base = sanear(nombre_base);
    let base = match sufijo {
        Some(s) => format!("{base} ({s})"),
        None => base,
    };
    let ext = sanear(extension).to_uppercase();
    match tipo {
        TipoParte::Editada => format!("{base} (editada).{ext}"),
        TipoParte::Alternativa => format!("{base} (alternativa).{ext}"),
        _ => format!("{base}.{ext}"),
    }
}

/// Ruta relativa libre para una parte. `ocupada(clave_en_minusculas)` dice si ya existe.
pub fn elegir_ruta(
    nombre_base: &str,
    id: &str,
    fecha_txt: &str,
    tipo: TipoParte,
    extension: &str,
    ocupada: &mut dyn FnMut(&str) -> bool,
) -> anyhow::Result<String> {
    let carpeta = carpeta_relativa(&fecha(fecha_txt)?);
    let simple = format!("{carpeta}\\{}", nombre_parte(nombre_base, tipo, extension, None));
    if !ocupada(&simple.to_lowercase()) {
        return Ok(simple);
    }
    let sufijo = sufijo_de_id(id);
    let con_sufijo = format!("{carpeta}\\{}", nombre_parte(nombre_base, tipo, extension, Some(&sufijo)));
    if !ocupada(&con_sufijo.to_lowercase()) {
        return Ok(con_sufijo);
    }
    for n in 2..1000 {
        let s = format!("{sufijo}-{n}");
        let c = format!("{carpeta}\\{}", nombre_parte(nombre_base, tipo, extension, Some(&s)));
        if !ocupada(&c.to_lowercase()) {
            return Ok(c);
        }
    }
    anyhow::bail!("no encuentro un nombre libre para {nombre_base}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ruta_en_disco_separa_por_componentes() {
        let base = Path::new("base");
        let p = en_disco(base, r"2026\2026-08\IMG_0001.HEIC");
        let comps: Vec<_> = p.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
        assert_eq!(comps, ["base", "2026", "2026-08", "IMG_0001.HEIC"]);
        assert_eq!(p, base.join("2026").join("2026-08").join("IMG_0001.HEIC"));
    }

    #[test]
    fn sanea_caracteres_prohibidos() {
        assert_eq!(sanear(r#"a<b>c:d"e/f\g|h?i*j"#), "a_b_c_d_e_f_g_h_i_j");
        assert_eq!(sanear("  foto. "), "foto");
        assert_eq!(sanear(""), "sin_nombre");
    }

    #[test]
    fn carpeta_usa_la_hora_local_del_desfase() {
        // 2026-09-01 02:00 UTC es 31 de agosto en -04:00
        let f = fecha("2026-08-31T22:00:00-04:00").unwrap();
        assert_eq!(carpeta_relativa(&f), r"2026\2026-08");
        assert_eq!(mes_clave(&f), "2026-08");
    }

    #[test]
    fn nombres_de_partes() {
        assert_eq!(nombre_parte("IMG_1", TipoParte::Original, "heic", None), "IMG_1.HEIC");
        assert_eq!(nombre_parte("IMG_1", TipoParte::Live, "MOV", None), "IMG_1.MOV");
        assert_eq!(nombre_parte("IMG_1", TipoParte::Editada, "JPG", None), "IMG_1 (editada).JPG");
        assert_eq!(nombre_parte("IMG_1", TipoParte::Alternativa, "DNG", None), "IMG_1 (alternativa).DNG");
        assert_eq!(nombre_parte("IMG_1", TipoParte::Original, "JPG", Some("ab12cd")), "IMG_1 (ab12cd).JPG");
    }

    #[test]
    fn sufijo_solo_alfanumerico() {
        assert_eq!(sufijo_de_id("Ab+/C-d9xyz"), "abcd9x");
    }

    #[test]
    fn elige_ruta_y_evita_colisiones() {
        let f = "2026-08-10T19:37:18-04:00";
        let libre = elegir_ruta("IMG_1", "Zq9xyw12", f, TipoParte::Original, "JPG", &mut |_| false).unwrap();
        assert_eq!(libre, r"2026\2026-08\IMG_1.JPG");
        let ocupadas = [r"2026\2026-08\img_1.jpg".to_string()];
        let otra = elegir_ruta("IMG_1", "Zq9xyw12", f, TipoParte::Original, "JPG", &mut |c| ocupadas.contains(&c.to_string())).unwrap();
        assert_eq!(otra, r"2026\2026-08\IMG_1 (zq9xyw).JPG");
    }
}
