//! Tipos que llegan del puente de iCloud.
use serde::{Deserialize, Serialize};

pub const BIBLIOTECA_PERSONAL: &str = "PrimarySync";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TipoParte {
    Original,
    Live,
    /// La otra mitad de un par RAW+JPEG (resOriginalAlt).
    Alternativa,
    Editada,
}

impl TipoParte {
    pub fn como_str(self) -> &'static str {
        match self {
            TipoParte::Original => "original",
            TipoParte::Live => "live",
            TipoParte::Alternativa => "alternativa",
            TipoParte::Editada => "editada",
        }
    }

    pub fn desde_str(s: &str) -> Option<TipoParte> {
        match s {
            "original" => Some(TipoParte::Original),
            "live" => Some(TipoParte::Live),
            "alternativa" => Some(TipoParte::Alternativa),
            "editada" => Some(TipoParte::Editada),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParteIcloud {
    pub tipo: TipoParte,
    pub tamano: u64,
    pub url: String,
    pub extension: String,
    pub checksum: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetIcloud {
    pub id: String,
    pub biblioteca: String,
    pub nombre_base: String,
    /// RFC 3339 en hora local con desfase, p. ej. "2026-08-10T19:37:18-04:00".
    pub fecha: String,
    /// Cuándo se añadió a la fototeca (RFC 3339). La ventana de "mantener N días" usa la más
    /// reciente entre esta y `fecha`.
    #[serde(default)]
    pub fecha_agregada: Option<String>,
    /// false si iCloud guarda datos de este asset que icdown no copia: nunca se borra de iCloud.
    /// El puente real siempre lo envía; el valor por omisión es solo por compatibilidad.
    #[serde(default = "verdadero")]
    pub liberable: bool,
    pub partes: Vec<ParteIcloud>,
}

fn verdadero() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializa_asset_del_puente() {
        let j = r#"{"id":"A1","biblioteca":"PrimarySync","nombre_base":"IMG_0001",
            "fecha":"2026-08-10T19:37:18-04:00","partes":[
            {"tipo":"original","tamano":10,"url":"http://x/1","extension":"HEIC","checksum":"c"},
            {"tipo":"live","tamano":5,"url":"http://x/2","extension":"MOV","checksum":"d"}]}"#;
        let a: AssetIcloud = serde_json::from_str(j).unwrap();
        assert_eq!(a.partes[0].tamano, 10);
        assert_eq!(a.partes[1].tipo, TipoParte::Live);
        assert!(a.liberable, "sin el campo se asume liberable (compatibilidad)");
        assert_eq!(a.fecha_agregada, None);
    }

    #[test]
    fn deserializa_alternativa_y_no_liberable() {
        let j = r#"{"id":"A1","biblioteca":"PrimarySync","nombre_base":"IMG_0001","liberable":false,
            "fecha_agregada":"2026-09-01T10:00:00-04:00",
            "fecha":"2026-08-10T19:37:18-04:00","partes":[
            {"tipo":"alternativa","tamano":10,"url":"http://x/1","extension":"DNG","checksum":"c"}]}"#;
        let a: AssetIcloud = serde_json::from_str(j).unwrap();
        assert_eq!(a.partes[0].tipo, TipoParte::Alternativa);
        assert!(!a.liberable);
        assert_eq!(a.fecha_agregada.as_deref(), Some("2026-09-01T10:00:00-04:00"));
    }

    #[test]
    fn tipo_parte_ida_y_vuelta() {
        for t in [TipoParte::Original, TipoParte::Live, TipoParte::Alternativa, TipoParte::Editada] {
            assert_eq!(TipoParte::desde_str(t.como_str()), Some(t));
        }
        assert_eq!(TipoParte::desde_str("x"), None);
    }
}
