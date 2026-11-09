//! Registro persistente (SQLite en <destino>\.icdown\registro.db).
use crate::criterio::FilaInventario;
use crate::layout;
use crate::modelo::{AssetIcloud, TipoParte, BIBLIOTECA_PERSONAL};
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

pub struct Registro {
    con: Connection,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrabajoParte {
    pub asset_id: String,
    pub tipo: TipoParte,
    pub nombre_base: String,
    pub fecha: String,
    pub extension: String,
    pub tamano: u64,
    /// Checksum que da Apple; junto con el tamaño identifica el contenido de un `.part`.
    pub checksum: String,
    pub url: String,
    pub ruta: Option<String>,
    /// SHA-256 del `.part` verificado justo antes de renombrarlo; solo con él se adopta un archivo final ya existente.
    pub sha_pendiente: Option<String>,
    /// El `sha_pendiente` es de un archivo con estructura atípica confirmado por doble descarga.
    pub estructura_atipica: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParteGuardada {
    pub asset_id: String,
    pub tipo: TipoParte,
    pub ruta: String,
    pub tamano: u64,
    pub extension: String,
    pub sha256: String,
    /// Estructura no estándar en iCloud, confirmada por doble descarga: se re-verifica solo por tamaño y SHA-256.
    pub estructura_atipica: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Candidato {
    pub asset_id: String,
    pub nombre_base: String,
    pub fecha: String,
    pub partes: Vec<ParteGuardada>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CandidatosLiberar {
    pub candidatos: Vec<Candidato>,
    /// Liberables que aún no están copiados y verificados del todo.
    pub no_verificados: u64,
    /// Tienen datos en iCloud que icdown no sabe copiar: nunca se proponen.
    pub no_liberables: u64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Resumen {
    pub assets: u64,
    pub assets_completos: u64,
    pub partes: u64,
    pub verificadas: u64,
    pub pendientes: u64,
    pub con_error: u64,
    pub bytes_total: u64,
    pub bytes_verificados: u64,
    pub borrados_icloud: u64,
    /// Verificadas con estructura atípica (confirmadas por doble descarga).
    pub estructura_atipica: u64,
}

/// Lo que muestra la bandeja: una consulta ligera para poder repetirla cada segundo.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Progreso {
    pub partes: u64,
    pub verificadas: u64,
    pub con_error: u64,
    pub bytes_total: u64,
    pub bytes_verificados: u64,
    /// Fin del último listado completo de iCloud (unix).
    pub ultimo_listado: Option<i64>,
}

const ESQUEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta(clave TEXT PRIMARY KEY, valor TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS assets(
  id TEXT PRIMARY KEY,
  biblioteca TEXT NOT NULL,
  nombre_base TEXT NOT NULL,
  fecha TEXT NOT NULL,
  fecha_unix INTEGER NOT NULL,
  mes TEXT NOT NULL,
  visto_en_lista INTEGER NOT NULL,
  borrado_icloud TEXT,
  liberable INTEGER NOT NULL DEFAULT 1,
  agregado_unix INTEGER
);
CREATE TABLE IF NOT EXISTS partes(
  asset_id TEXT NOT NULL REFERENCES assets(id),
  tipo TEXT NOT NULL,
  extension TEXT NOT NULL,
  tamano INTEGER NOT NULL,
  checksum_apple TEXT NOT NULL,
  url TEXT NOT NULL,
  ruta TEXT,
  ruta_clave TEXT UNIQUE,
  estado TEXT NOT NULL DEFAULT 'pendiente',
  sha256 TEXT,
  intentos INTEGER NOT NULL DEFAULT 0,
  proximo_intento INTEGER NOT NULL DEFAULT 0,
  ultimo_error TEXT,
  sha_pendiente TEXT,
  estructura_atipica INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY(asset_id, tipo)
);
CREATE INDEX IF NOT EXISTS partes_estado ON partes(estado, proximo_intento);
"#;

// Un cambio de tamaño, de extensión o de checksum significa que el archivo en iCloud cambió (p. ej. nueva edición).
// Un fallo por enlace caducado no es culpa del asset: al llegar un enlace nuevo se olvida (intentos, espera
// y estado 'error'). Las versiones anteriores los contaban como fallos y penalizaban miles de partes.
// Igual con "JPEG sin marcador de fin": falso positivo de una verificación antigua (JPEG con datos tras el EOI).
// Y con cualquier "archivo incompleto o dañado" (estructura): ahora se confirma por doble descarga.
const UPSERT_PARTE: &str = r#"
INSERT INTO partes(asset_id, tipo, extension, tamano, checksum_apple, url) VALUES(?1, ?2, ?3, ?4, ?5, ?6)
ON CONFLICT(asset_id, tipo) DO UPDATE SET
  url = excluded.url,
  checksum_apple = excluded.checksum_apple,
  estado = CASE WHEN partes.tamano <> excluded.tamano OR partes.extension <> excluded.extension OR partes.checksum_apple <> excluded.checksum_apple THEN 'pendiente'
    WHEN partes.estado = 'error' AND (partes.ultimo_error LIKE 'enlace de descarga caducado%' OR partes.ultimo_error LIKE '%JPEG sin marcador de fin%' OR partes.ultimo_error LIKE '%archivo incompleto o dañado%') THEN 'pendiente' ELSE partes.estado END,
  sha256 = CASE WHEN partes.tamano <> excluded.tamano OR partes.extension <> excluded.extension OR partes.checksum_apple <> excluded.checksum_apple THEN NULL ELSE partes.sha256 END,
  intentos = CASE WHEN partes.tamano <> excluded.tamano OR partes.extension <> excluded.extension OR partes.checksum_apple <> excluded.checksum_apple THEN 0
    WHEN (partes.ultimo_error LIKE 'enlace de descarga caducado%' OR partes.ultimo_error LIKE '%JPEG sin marcador de fin%' OR partes.ultimo_error LIKE '%archivo incompleto o dañado%') THEN 0 ELSE partes.intentos END,
  proximo_intento = CASE WHEN partes.tamano <> excluded.tamano OR partes.extension <> excluded.extension OR partes.checksum_apple <> excluded.checksum_apple THEN 0
    WHEN (partes.ultimo_error LIKE 'enlace de descarga caducado%' OR partes.ultimo_error LIKE '%JPEG sin marcador de fin%' OR partes.ultimo_error LIKE '%archivo incompleto o dañado%') THEN 0 ELSE partes.proximo_intento END,
  ultimo_error = CASE WHEN (partes.ultimo_error LIKE 'enlace de descarga caducado%' OR partes.ultimo_error LIKE '%JPEG sin marcador de fin%' OR partes.ultimo_error LIKE '%archivo incompleto o dañado%') THEN NULL ELSE partes.ultimo_error END,
  sha_pendiente = CASE WHEN partes.tamano <> excluded.tamano OR partes.extension <> excluded.extension OR partes.checksum_apple <> excluded.checksum_apple THEN NULL ELSE partes.sha_pendiente END,
  estructura_atipica = CASE WHEN partes.tamano <> excluded.tamano OR partes.extension <> excluded.extension OR partes.checksum_apple <> excluded.checksum_apple THEN 0 ELSE partes.estructura_atipica END,
  ruta = CASE WHEN partes.extension <> excluded.extension THEN NULL ELSE partes.ruta END,
  ruta_clave = CASE WHEN partes.extension <> excluded.extension THEN NULL ELSE partes.ruta_clave END,
  tamano = excluded.tamano,
  extension = excluded.extension
"#;

/// Filtro por años de captura: ?4 es una lista JSON de años, o NULL para todos.
const EN_ANIOS: &str = "(?4 IS NULL OR CAST(substr(a.fecha, 1, 4) AS INTEGER) IN (SELECT value FROM json_each(?4)))";

const EN_ULTIMA_LISTA: &str = "a.visto_en_lista = CAST((SELECT valor FROM meta WHERE clave = 'marca_lista') AS INTEGER)";

const COLUMNAS_TRABAJO: &str =
    "p.asset_id, p.tipo, a.nombre_base, a.fecha, p.extension, p.tamano, p.url, p.ruta, p.sha_pendiente, p.checksum_apple, p.estructura_atipica";

fn trabajo_de_fila(r: &rusqlite::Row) -> rusqlite::Result<TrabajoParte> {
    Ok(TrabajoParte {
        asset_id: r.get(0)?,
        tipo: tipo_de(r.get(1)?),
        nombre_base: r.get(2)?,
        fecha: r.get(3)?,
        extension: r.get(4)?,
        tamano: r.get::<_, i64>(5)? as u64,
        url: r.get(6)?,
        ruta: r.get(7)?,
        sha_pendiente: r.get(8)?,
        checksum: r.get(9)?,
        estructura_atipica: r.get::<_, i64>(10)? != 0,
    })
}

/// (estado, segundos de espera) tras `intentos` fallos consecutivos.
pub fn espera_reintento(intentos: u32) -> (&'static str, i64) {
    const ESPERAS: [i64; 9] = [5, 30, 120, 300, 600, 1200, 1800, 3600, 3600];
    if intentos >= 10 {
        ("error", 6 * 3600)
    } else {
        ("pendiente", ESPERAS[(intentos.max(1) - 1) as usize])
    }
}

/// Migración: añade la columna si una base de datos anterior no la tiene.
fn asegurar_columna(con: &Connection, tabla: &str, columna: &str, definicion: &str) -> Result<()> {
    let existe: bool = con
        .prepare(&format!("SELECT 1 FROM pragma_table_info('{tabla}') WHERE name = ?1"))?
        .exists(params![columna])?;
    if !existe {
        con.execute_batch(&format!("ALTER TABLE {tabla} ADD COLUMN {columna} {definicion}"))?;
    }
    Ok(())
}

fn tipo_de(s: String) -> TipoParte {
    TipoParte::desde_str(&s).unwrap_or(TipoParte::Original)
}

impl Registro {
    pub fn abrir(ruta: &Path) -> Result<Registro> {
        if let Some(p) = ruta.parent() {
            std::fs::create_dir_all(p)?;
        }
        let con = Connection::open(ruta).with_context(|| format!("abriendo {}", ruta.display()))?;
        con.busy_timeout(Duration::from_secs(30))?;
        // journal_mode devuelve una fila: se ejecuta con query_row para no provocar "Execute returned results".
        con.query_row("PRAGMA journal_mode=WAL", [], |_| Ok(()))?;
        con.execute_batch("PRAGMA synchronous=FULL;")?;
        Self::con_esquema(con)
    }

    /// Solo lectura y sin tocar el esquema: para consultar mientras el motor escribe (WAL).
    pub fn abrir_lectura(ruta: &Path) -> Result<Registro> {
        let con = Connection::open_with_flags(ruta, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
            .with_context(|| format!("abriendo {}", ruta.display()))?;
        con.busy_timeout(Duration::from_secs(1))?;
        Ok(Registro { con })
    }

    pub fn en_memoria() -> Result<Registro> {
        Self::con_esquema(Connection::open_in_memory()?)
    }

    fn con_esquema(con: Connection) -> Result<Registro> {
        con.execute_batch(ESQUEMA)?;
        asegurar_columna(&con, "partes", "sha_pendiente", "TEXT")?;
        asegurar_columna(&con, "assets", "liberable", "INTEGER NOT NULL DEFAULT 1")?;
        asegurar_columna(&con, "assets", "agregado_unix", "INTEGER")?;
        asegurar_columna(&con, "partes", "estructura_atipica", "INTEGER NOT NULL DEFAULT 0")?;
        // Liberación (borrado en iCloud) en la que se borró: para poder restaurarla entera.
        asegurar_columna(&con, "assets", "sesion_borrado", "TEXT")?;
        Ok(Registro { con })
    }

    pub fn upsert_asset(&mut self, a: &AssetIcloud, marca: i64) -> Result<()> {
        let f = layout::fecha(&a.fecha).with_context(|| format!("fecha inválida en {}", a.id))?;
        let agregado = a.fecha_agregada.as_deref().and_then(|t| layout::fecha(t).ok()).map(|f| f.timestamp()).unwrap_or(f.timestamp());
        let tx = self.con.transaction()?;
        tx.execute(
            "INSERT INTO assets(id, biblioteca, nombre_base, fecha, fecha_unix, mes, visto_en_lista, liberable, agregado_unix)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET biblioteca = excluded.biblioteca, nombre_base = excluded.nombre_base,
               fecha = excluded.fecha, fecha_unix = excluded.fecha_unix, mes = excluded.mes,
               visto_en_lista = excluded.visto_en_lista, borrado_icloud = NULL, liberable = excluded.liberable,
               agregado_unix = excluded.agregado_unix",
            params![a.id, a.biblioteca, a.nombre_base, a.fecha, f.timestamp(), layout::mes_clave(&f), marca, a.liberable, agregado],
        )?;
        for p in &a.partes {
            tx.execute(UPSERT_PARTE, params![a.id, p.tipo.como_str(), p.extension, p.tamano as i64, p.checksum, p.url])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn guardar_marca_lista(&mut self, marca: i64) -> Result<()> {
        self.con.execute(
            "INSERT INTO meta(clave, valor) VALUES('marca_lista', ?1) ON CONFLICT(clave) DO UPDATE SET valor = excluded.valor",
            params![marca.to_string()],
        )?;
        Ok(())
    }

    /// Valor libre en la tabla `meta` (una sentencia: transacción corta, compatible con otros procesos en WAL).
    pub fn guardar_meta(&mut self, clave: &str, valor: &str) -> Result<()> {
        self.con.execute(
            "INSERT INTO meta(clave, valor) VALUES(?1, ?2) ON CONFLICT(clave) DO UPDATE SET valor = excluded.valor",
            params![clave, valor],
        )?;
        Ok(())
    }

    pub fn leer_meta(&self, clave: &str) -> Result<Option<String>> {
        Ok(self.con.query_row("SELECT valor FROM meta WHERE clave = ?1", params![clave], |r| r.get(0)).optional()?)
    }

    pub fn borrar_meta(&mut self, clave: &str) -> Result<()> {
        self.con.execute("DELETE FROM meta WHERE clave = ?1", params![clave])?;
        Ok(())
    }

    pub fn marca_lista(&self) -> Result<Option<i64>> {
        let v: Option<String> = self
            .con
            .query_row("SELECT valor FROM meta WHERE clave = 'marca_lista'", [], |r| r.get(0))
            .optional()?;
        Ok(v.and_then(|s| s.parse().ok()))
    }

    /// Partes por copiar de assets presentes en el último listado, de la más antigua a la más nueva.
    pub fn partes_pendientes(&self, ahora: i64, mes: Option<&str>) -> Result<Vec<TrabajoParte>> {
        let sql = format!(
            "SELECT {COLUMNAS_TRABAJO}
             FROM partes p JOIN assets a ON a.id = p.asset_id
             WHERE p.estado <> 'verificado' AND p.proximo_intento <= ?1 AND a.borrado_icloud IS NULL
               AND {EN_ULTIMA_LISTA} AND (?2 IS NULL OR a.mes = ?2)
             ORDER BY a.fecha_unix, p.asset_id, CASE p.tipo WHEN 'original' THEN 0 WHEN 'live' THEN 1 WHEN 'alternativa' THEN 2 ELSE 3 END"
        );
        let mut st = self.con.prepare(&sql)?;
        let filas = st.query_map(params![ahora, mes], trabajo_de_fila)?;
        Ok(filas.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Partes aún sin verificar de un asset, tal como están ahora en el registro.
    pub fn trabajos_de(&self, asset_id: &str) -> Result<Vec<TrabajoParte>> {
        let sql = format!(
            "SELECT {COLUMNAS_TRABAJO} FROM partes p JOIN assets a ON a.id = p.asset_id
             WHERE p.asset_id = ?1 AND p.estado <> 'verificado'"
        );
        let mut st = self.con.prepare(&sql)?;
        let filas = st.query_map(params![asset_id], trabajo_de_fila)?;
        Ok(filas.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Enlaces renovados sin listar: actualiza las partes que ya existen de un asset ya registrado
    /// (url, tamaño, checksum, extensión; perdona los fallos por enlace caducado). No toca la marca
    /// del listado (`visto_en_lista`) ni crea assets o partes nuevos: eso es cosa del listado completo.
    pub fn actualizar_enlaces(&mut self, a: &AssetIcloud) -> Result<()> {
        let tx = self.con.transaction()?;
        for p in &a.partes {
            let existe = tx
                .query_row("SELECT 1 FROM partes WHERE asset_id = ?1 AND tipo = ?2", params![a.id, p.tipo.como_str()], |_| Ok(()))
                .optional()?
                .is_some();
            if existe {
                tx.execute(UPSERT_PARTE, params![a.id, p.tipo.como_str(), p.extension, p.tamano as i64, p.checksum, p.url])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn proximo_reintento(&self, mes: Option<&str>) -> Result<Option<i64>> {
        let sql = format!(
            "SELECT MIN(p.proximo_intento) FROM partes p JOIN assets a ON a.id = p.asset_id
             WHERE p.estado <> 'verificado' AND a.borrado_icloud IS NULL AND {EN_ULTIMA_LISTA}
               AND (?1 IS NULL OR a.mes = ?1)"
        );
        Ok(self.con.query_row(&sql, params![mes], |r| r.get::<_, Option<i64>>(0))?)
    }

    pub fn reintentar_ya(&mut self) -> Result<()> {
        self.con.execute("UPDATE partes SET proximo_intento = 0 WHERE estado <> 'verificado'", [])?;
        Ok(())
    }

    pub fn ruta_ocupada(&self, clave: &str) -> Result<bool> {
        Ok(self
            .con
            .query_row("SELECT 1 FROM partes WHERE ruta_clave = ?1", params![clave], |_| Ok(()))
            .optional()?
            .is_some())
    }

    pub fn asignar_ruta(&mut self, asset_id: &str, tipo: TipoParte, ruta: &str) -> Result<()> {
        self.con.execute(
            "UPDATE partes SET ruta = ?3, ruta_clave = ?4 WHERE asset_id = ?1 AND tipo = ?2",
            params![asset_id, tipo.como_str(), ruta, ruta.to_lowercase()],
        )?;
        Ok(())
    }

    pub fn marcar_verificada(&mut self, asset_id: &str, tipo: TipoParte, sha256: &str) -> Result<()> {
        self.marcar_verificada_con(asset_id, tipo, sha256, false)
    }

    /// `atipica`: estructura no estándar en iCloud, confirmada por doble descarga.
    pub fn marcar_verificada_con(&mut self, asset_id: &str, tipo: TipoParte, sha256: &str, atipica: bool) -> Result<()> {
        self.con.execute(
            "UPDATE partes SET estado = 'verificado', sha256 = ?3, intentos = 0, proximo_intento = 0, ultimo_error = NULL,
               sha_pendiente = NULL, estructura_atipica = ?4
             WHERE asset_id = ?1 AND tipo = ?2",
            params![asset_id, tipo.como_str(), sha256, atipica],
        )?;
        Ok(())
    }

    /// Se anota antes de renombrar el `.part` verificado al nombre final (`atipica`: confirmado por doble descarga).
    pub fn anotar_sha_pendiente(&mut self, asset_id: &str, tipo: TipoParte, sha256: &str, atipica: bool) -> Result<()> {
        self.con.execute(
            "UPDATE partes SET sha_pendiente = ?3, estructura_atipica = ?4 WHERE asset_id = ?1 AND tipo = ?2",
            params![asset_id, tipo.como_str(), sha256, atipica],
        )?;
        Ok(())
    }

    pub fn registrar_fallo(&mut self, asset_id: &str, tipo: TipoParte, error: &str, ahora: i64) -> Result<()> {
        let intentos: i64 = self.con.query_row(
            "SELECT intentos FROM partes WHERE asset_id = ?1 AND tipo = ?2",
            params![asset_id, tipo.como_str()],
            |r| r.get(0),
        )?;
        let intentos = intentos + 1;
        let (estado, espera) = espera_reintento(intentos as u32);
        self.con.execute(
            "UPDATE partes SET intentos = ?3, proximo_intento = ?4, ultimo_error = ?5, estado = ?6 WHERE asset_id = ?1 AND tipo = ?2",
            params![asset_id, tipo.como_str(), intentos, ahora + espera, error, estado],
        )?;
        Ok(())
    }

    pub fn marcar_para_redescarga(&mut self, asset_id: &str, tipo: TipoParte, motivo: &str) -> Result<()> {
        self.con.execute(
            "UPDATE partes SET estado = 'pendiente', sha256 = NULL, intentos = 0, proximo_intento = 0, ultimo_error = ?3,
               sha_pendiente = NULL, estructura_atipica = 0
             WHERE asset_id = ?1 AND tipo = ?2",
            params![asset_id, tipo.como_str(), motivo],
        )?;
        Ok(())
    }

    pub fn partes_verificadas(&self) -> Result<Vec<ParteGuardada>> {
        let mut st = self.con.prepare(
            "SELECT asset_id, tipo, ruta, tamano, extension, sha256, estructura_atipica FROM partes
             WHERE estado = 'verificado' AND ruta IS NOT NULL AND sha256 IS NOT NULL ORDER BY ruta",
        )?;
        let filas = st.query_map([], |r| {
            Ok(ParteGuardada {
                asset_id: r.get(0)?,
                tipo: tipo_de(r.get(1)?),
                ruta: r.get(2)?,
                tamano: r.get::<_, i64>(3)? as u64,
                extension: r.get(4)?,
                sha256: r.get(5)?,
                estructura_atipica: r.get::<_, i64>(6)? != 0,
            })
        })?;
        Ok(filas.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Assets personales, del listado `marca`, capturados y agregados antes de `limite_unix`, con todas sus partes verificadas.
    /// Solo los marcados como liberables. Cuenta aparte los que no están verificados del todo
    /// y los que tienen datos que icdown no copia.
    pub fn candidatos_liberar(&self, marca: i64, limite_unix: i64) -> Result<CandidatosLiberar> {
        self.candidatos_liberar_en(marca, limite_unix, None)
    }

    /// Como `candidatos_liberar`, pero solo de estos años de captura (si se dan).
    pub fn candidatos_liberar_en(&self, marca: i64, limite_unix: i64, anios: Option<&BTreeSet<i32>>) -> Result<CandidatosLiberar> {
        let anios = anios.map(|a| serde_json::to_string(a).expect("años en JSON"));
        let mut st = self.con.prepare(&format!(
            "SELECT a.id, a.nombre_base, a.fecha FROM assets a
             WHERE a.biblioteca = ?3 AND a.borrado_icloud IS NULL AND a.visto_en_lista = ?1 AND MAX(a.fecha_unix, COALESCE(a.agregado_unix, a.fecha_unix)) < ?2
               AND a.liberable = 1 AND {EN_ANIOS}
               AND EXISTS(SELECT 1 FROM partes p WHERE p.asset_id = a.id)
               AND NOT EXISTS(SELECT 1 FROM partes p WHERE p.asset_id = a.id
                              AND (p.estado <> 'verificado' OR p.sha256 IS NULL OR p.ruta IS NULL))
             ORDER BY a.fecha_unix",
        ))?;
        let base: Vec<(String, String, String)> = st
            .query_map(params![marca, limite_unix, BIBLIOTECA_PERSONAL, anios], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let mut st_p = self.con.prepare(
            "SELECT asset_id, tipo, ruta, tamano, extension, sha256, estructura_atipica FROM partes WHERE asset_id = ?1 ORDER BY tipo",
        )?;
        let mut candidatos = Vec::with_capacity(base.len());
        for (id, nombre_base, fecha) in base {
            let partes = st_p
                .query_map(params![id], |r| {
                    Ok(ParteGuardada {
                        asset_id: r.get(0)?,
                        tipo: tipo_de(r.get(1)?),
                        ruta: r.get(2)?,
                        tamano: r.get::<_, i64>(3)? as u64,
                        extension: r.get(4)?,
                        sha256: r.get(5)?,
                        estructura_atipica: r.get::<_, i64>(6)? != 0,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            candidatos.push(Candidato { asset_id: id, nombre_base, fecha, partes });
        }
        let no_listos: i64 = self.con.query_row(
            &format!(
                "SELECT COUNT(*) FROM assets a
                 WHERE a.biblioteca = ?3 AND a.borrado_icloud IS NULL AND a.visto_en_lista = ?1 AND MAX(a.fecha_unix, COALESCE(a.agregado_unix, a.fecha_unix)) < ?2
                   AND a.liberable = 1 AND {EN_ANIOS}
                   AND (NOT EXISTS(SELECT 1 FROM partes p WHERE p.asset_id = a.id)
                        OR EXISTS(SELECT 1 FROM partes p WHERE p.asset_id = a.id AND p.estado <> 'verificado'))"
            ),
            params![marca, limite_unix, BIBLIOTECA_PERSONAL, anios],
            |r| r.get(0),
        )?;
        let no_liberables: i64 = self.con.query_row(
            &format!(
                "SELECT COUNT(*) FROM assets a
                 WHERE a.biblioteca = ?3 AND a.borrado_icloud IS NULL AND a.visto_en_lista = ?1 AND MAX(a.fecha_unix, COALESCE(a.agregado_unix, a.fecha_unix)) < ?2
                   AND a.liberable = 0 AND {EN_ANIOS}"
            ),
            params![marca, limite_unix, BIBLIOTECA_PERSONAL, anios],
            |r| r.get(0),
        )?;
        Ok(CandidatosLiberar { candidatos, no_verificados: no_listos as u64, no_liberables: no_liberables as u64 })
    }

    pub fn marcar_borrado(&mut self, asset_id: &str, cuando: &str) -> Result<()> {
        self.marcar_borrado_en(asset_id, cuando, None)
    }

    /// Borrado en iCloud durante la liberación `sesion` (para poder restaurarla).
    pub fn marcar_borrado_en(&mut self, asset_id: &str, cuando: &str, sesion: Option<&str>) -> Result<()> {
        self.con.execute("UPDATE assets SET borrado_icloud = ?2, sesion_borrado = ?3 WHERE id = ?1", params![asset_id, cuando, sesion])?;
        Ok(())
    }

    /// Vuelve a estar en iCloud (restaurado de "Eliminado recientemente").
    pub fn marcar_restaurado(&mut self, asset_id: &str) -> Result<()> {
        self.con.execute("UPDATE assets SET borrado_icloud = NULL, sesion_borrado = NULL WHERE id = ?1", params![asset_id])?;
        Ok(())
    }

    /// La última liberación con algo aún borrado (por la hora del último borrado).
    pub fn ultima_sesion_borrado(&self) -> Result<Option<String>> {
        Ok(self
            .con
            .query_row(
                "SELECT sesion_borrado FROM assets WHERE sesion_borrado IS NOT NULL AND borrado_icloud IS NOT NULL
                 ORDER BY borrado_icloud DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Ids borrados en iCloud durante la liberación `sesion` (y no restaurados).
    pub fn ids_de_sesion(&self, sesion: &str) -> Result<Vec<String>> {
        let mut st = self.con.prepare("SELECT id FROM assets WHERE sesion_borrado = ?1 AND borrado_icloud IS NOT NULL ORDER BY fecha_unix")?;
        let filas = st.query_map(params![sesion], |r| r.get(0))?;
        Ok(filas.collect::<rusqlite::Result<Vec<String>>>()?)
    }

    /// Todo lo del último listado que sigue en iCloud, para calcular selecciones al instante.
    pub fn inventario_liberar(&self) -> Result<Vec<FilaInventario>> {
        let Some(marca) = self.marca_lista()? else { return Ok(vec![]) };
        let mut st = self.con.prepare(
            "SELECT a.fecha, a.fecha_unix, a.agregado_unix, a.biblioteca = ?2, a.liberable,
                    EXISTS(SELECT 1 FROM partes p WHERE p.asset_id = a.id)
                      AND NOT EXISTS(SELECT 1 FROM partes p WHERE p.asset_id = a.id
                                     AND (p.estado <> 'verificado' OR p.sha256 IS NULL OR p.ruta IS NULL)),
                    COALESCE((SELECT SUM(tamano) FROM partes p WHERE p.asset_id = a.id), 0),
                    EXISTS(SELECT 1 FROM partes p WHERE p.asset_id = a.id AND p.estructura_atipica = 1)
             FROM assets a WHERE a.borrado_icloud IS NULL AND a.visto_en_lista = ?1",
        )?;
        let filas = st.query_map(params![marca, BIBLIOTECA_PERSONAL], |r| {
            let fecha: String = r.get(0)?;
            Ok(FilaInventario {
                anio: fecha.get(..4).and_then(|a| a.parse().ok()).unwrap_or(0),
                fecha_unix: r.get(1)?,
                agregado_unix: r.get(2)?,
                personal: r.get(3)?,
                liberable: r.get(4)?,
                completo: r.get(5)?,
                bytes: r.get::<_, i64>(6)? as u64,
                atipico: r.get(7)?,
            })
        })?;
        Ok(filas.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn resumen(&self) -> Result<Resumen> {
        let (partes, verificadas, pendientes, con_error, bytes_total, bytes_verificados): (i64, i64, i64, i64, i64, i64) =
            self.con.query_row(
                "SELECT COUNT(*),
                        COALESCE(SUM(estado = 'verificado'), 0),
                        COALESCE(SUM(estado = 'pendiente'), 0),
                        COALESCE(SUM(estado = 'error'), 0),
                        COALESCE(SUM(tamano), 0),
                        COALESCE(SUM(CASE WHEN estado = 'verificado' THEN tamano ELSE 0 END), 0)
                 FROM partes",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )?;
        let assets: i64 = self.con.query_row("SELECT COUNT(*) FROM assets", [], |r| r.get(0))?;
        let completos: i64 = self.con.query_row(
            "SELECT COUNT(*) FROM assets a WHERE EXISTS(SELECT 1 FROM partes p WHERE p.asset_id = a.id)
               AND NOT EXISTS(SELECT 1 FROM partes p WHERE p.asset_id = a.id AND p.estado <> 'verificado')",
            [],
            |r| r.get(0),
        )?;
        let borrados: i64 = self.con.query_row("SELECT COUNT(*) FROM assets WHERE borrado_icloud IS NOT NULL", [], |r| r.get(0))?;
        let atipicas: i64 =
            self.con.query_row("SELECT COUNT(*) FROM partes WHERE estado = 'verificado' AND estructura_atipica = 1", [], |r| r.get(0))?;
        Ok(Resumen {
            assets: assets as u64,
            assets_completos: completos as u64,
            partes: partes as u64,
            verificadas: verificadas as u64,
            pendientes: pendientes as u64,
            con_error: con_error as u64,
            bytes_total: bytes_total as u64,
            bytes_verificados: bytes_verificados as u64,
            borrados_icloud: borrados as u64,
            estructura_atipica: atipicas as u64,
        })
    }

    /// Cuántos elementos tenía el último listado completo de iCloud (None si nunca se listó).
    /// Sirve de total previsto mientras se lista de nuevo.
    pub fn assets_ultimo_listado(&self) -> Result<Option<u64>> {
        let Some(marca) = self.marca_lista()? else { return Ok(None) };
        let n: i64 = self.con.query_row("SELECT COUNT(*) FROM assets WHERE visto_en_lista = ?1", params![marca], |r| r.get(0))?;
        Ok(Some(n as u64))
    }

    pub fn progreso(&self) -> Result<Progreso> {
        let (partes, verificadas, con_error, bytes_total, bytes_verificados): (i64, i64, i64, i64, i64) = self.con.query_row(
            "SELECT COUNT(*),
                    COALESCE(SUM(estado = 'verificado'), 0),
                    COALESCE(SUM(estado = 'error'), 0),
                    COALESCE(SUM(tamano), 0),
                    COALESCE(SUM(CASE WHEN estado = 'verificado' THEN tamano ELSE 0 END), 0)
             FROM partes",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )?;
        Ok(Progreso {
            partes: partes as u64,
            verificadas: verificadas as u64,
            con_error: con_error as u64,
            bytes_total: bytes_total as u64,
            bytes_verificados: bytes_verificados as u64,
            ultimo_listado: self.marca_lista()?,
        })
    }

    /// Últimos errores, uno por línea: "IMG_0001 (live): motivo".
    pub fn errores(&self, limite: usize) -> Result<Vec<String>> {
        let mut st = self.con.prepare(
            "SELECT a.nombre_base, p.tipo, p.ultimo_error FROM partes p JOIN assets a ON a.id = p.asset_id
             WHERE p.estado <> 'verificado' AND p.ultimo_error IS NOT NULL ORDER BY p.intentos DESC LIMIT ?1",
        )?;
        let filas = st.query_map(params![limite as i64], |r| {
            Ok(format!("{} ({}): {}", r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
        })?;
        Ok(filas.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modelo::{AssetIcloud, ParteIcloud, TipoParte};
    use TipoParte::*;

    const AGO: &str = "2026-08-10T19:37:18-04:00";
    const VIEJO: &str = "2025-01-10T10:00:00-05:00";

    fn asset(id: &str, bib: &str, fecha: &str, partes: &[(TipoParte, u64)]) -> AssetIcloud {
        AssetIcloud {
            id: id.into(),
            biblioteca: bib.into(),
            nombre_base: format!("IMG_{id}"),
            fecha: fecha.into(),
            fecha_agregada: None,
            liberable: true,
            partes: partes
                .iter()
                .map(|(t, n)| ParteIcloud { tipo: *t, tamano: *n, url: format!("http://x/{id}/{}", t.como_str()), extension: "HEIC".into(), checksum: "c".into() })
                .collect(),
        }
    }

    fn listar(r: &mut Registro, assets: &[AssetIcloud], marca: i64) {
        for a in assets {
            r.upsert_asset(a, marca).unwrap();
        }
        r.guardar_marca_lista(marca).unwrap();
    }

    fn dar_por_copiado(r: &mut Registro, id: &str, tipo: TipoParte) {
        r.asignar_ruta(id, tipo, &format!(r"x\{id}.{}", tipo.como_str())).unwrap();
        r.marcar_verificada(id, tipo, "sha").unwrap();
    }

    #[test]
    fn upsert_crea_partes_pendientes() {
        let mut r = Registro::en_memoria().unwrap();
        listar(&mut r, &[asset("A1", "PrimarySync", AGO, &[(Original, 10), (Live, 5)])], 100);
        let p = r.partes_pendientes(1000, None).unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].tipo, Original);
        assert_eq!(p[0].ruta, None);
        assert_eq!(p[0].tamano, 10);
    }

    #[test]
    fn relistar_sin_cambios_conserva_lo_verificado() {
        let mut r = Registro::en_memoria().unwrap();
        let a = asset("A1", "PrimarySync", AGO, &[(Original, 10)]);
        listar(&mut r, std::slice::from_ref(&a), 100);
        dar_por_copiado(&mut r, "A1", Original);
        listar(&mut r, &[a], 200);
        assert!(r.partes_pendientes(1000, None).unwrap().is_empty());
        assert_eq!(r.partes_verificadas().unwrap()[0].sha256, "sha");
    }

    #[test]
    fn cambio_de_tamano_vuelve_a_pendiente() {
        let mut r = Registro::en_memoria().unwrap();
        listar(&mut r, &[asset("A1", "PrimarySync", AGO, &[(Editada, 10)])], 100);
        dar_por_copiado(&mut r, "A1", Editada);
        listar(&mut r, &[asset("A1", "PrimarySync", AGO, &[(Editada, 11)])], 200);
        let p = r.partes_pendientes(1000, None).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].tamano, 11);
    }

    #[test]
    fn cambio_de_checksum_con_mismo_tamano_vuelve_a_pendiente() {
        let mut r = Registro::en_memoria().unwrap();
        listar(&mut r, &[asset("A1", "PrimarySync", AGO, &[(Editada, 10)])], 100);
        dar_por_copiado(&mut r, "A1", Editada);
        let mut otra = asset("A1", "PrimarySync", AGO, &[(Editada, 10)]);
        otra.partes[0].checksum = "otro".into();
        listar(&mut r, &[otra], 200);
        let p = r.partes_pendientes(1000, None).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].checksum, "otro");
        assert!(r.partes_verificadas().unwrap().is_empty());
    }

    #[test]
    fn lo_que_ya_no_esta_en_icloud_no_se_intenta() {
        let mut r = Registro::en_memoria().unwrap();
        listar(&mut r, &[asset("A1", "PrimarySync", AGO, &[(Original, 10)])], 100);
        listar(&mut r, &[asset("A2", "PrimarySync", AGO, &[(Original, 10)])], 200);
        let p = r.partes_pendientes(1000, None).unwrap();
        assert_eq!(p.iter().map(|t| t.asset_id.as_str()).collect::<Vec<_>>(), vec!["A2"]);
    }

    #[test]
    fn fallos_con_espera_creciente_y_error_tras_10() {
        let mut r = Registro::en_memoria().unwrap();
        listar(&mut r, &[asset("A1", "PrimarySync", AGO, &[(Original, 10)])], 100);
        r.registrar_fallo("A1", Original, "corte", 1000).unwrap();
        assert!(r.partes_pendientes(1000, None).unwrap().is_empty());
        assert_eq!(r.partes_pendientes(1005, None).unwrap().len(), 1);
        assert_eq!(r.proximo_reintento(None).unwrap(), Some(1005));
        for _ in 0..9 {
            r.registrar_fallo("A1", Original, "corte", 1000).unwrap();
        }
        assert_eq!(r.resumen().unwrap().con_error, 1);
        assert_eq!(espera_reintento(10), ("error", 6 * 3600));
        r.reintentar_ya().unwrap();
        assert_eq!(r.partes_pendientes(1000, None).unwrap().len(), 1);
    }

    #[test]
    fn relistar_perdona_los_fallos_por_enlace_caducado() {
        // Base dañada por versiones anteriores: miles de partes penalizadas por enlaces caducados.
        let mut r = Registro::en_memoria().unwrap();
        listar(&mut r, &[
            asset("A1", "PrimarySync", AGO, &[(Original, 10)]),
            asset("A2", "PrimarySync", AGO, &[(Original, 10)]),
            asset("A3", "PrimarySync", AGO, &[(Original, 10)]),
        ], 100);
        for _ in 0..10 {
            r.registrar_fallo("A1", Original, "enlace de descarga caducado (HTTP 410)", 1000).unwrap();
        }
        r.registrar_fallo("A2", Original, "enlace de descarga caducado (HTTP 403)", 1000).unwrap();
        r.registrar_fallo("A3", Original, "fallo de red: corte", 1000).unwrap();
        assert_eq!(r.resumen().unwrap().con_error, 1);
        listar(&mut r, &[
            asset("A1", "PrimarySync", AGO, &[(Original, 10)]),
            asset("A2", "PrimarySync", AGO, &[(Original, 10)]),
            asset("A3", "PrimarySync", AGO, &[(Original, 10)]),
        ], 200);
        let ids: Vec<_> = r.partes_pendientes(1000, None).unwrap().into_iter().map(|t| t.asset_id).collect();
        assert_eq!(ids, vec!["A1", "A2"], "los caducados vuelven ya; el fallo real sigue esperando");
        assert_eq!(r.resumen().unwrap().con_error, 0);
        assert_eq!(r.errores(10).unwrap(), vec!["IMG_A3 (original): fallo de red: corte"]);
        // Sin penalización acumulada: el siguiente fallo vuelve a esperar solo 5 s.
        r.registrar_fallo("A1", Original, "corte", 1000).unwrap();
        assert_eq!(r.partes_pendientes(1005, None).unwrap().len(), 3);
    }

    #[test]
    fn relistar_perdona_el_falso_fallo_de_jpeg_sin_marcador_de_fin() {
        let mut r = Registro::en_memoria().unwrap();
        let a = || vec![asset("A1", "PrimarySync", AGO, &[(Original, 10)]), asset("A2", "PrimarySync", AGO, &[(Original, 10)])];
        listar(&mut r, &a(), 100);
        for _ in 0..10 {
            r.registrar_fallo("A1", Original, "verificación: archivo incompleto o dañado: JPEG sin marcador de fin", 1000).unwrap();
        }
        r.registrar_fallo("A2", Original, "fallo de red: corte", 1000).unwrap();
        assert!(r.partes_pendientes(1000, None).unwrap().is_empty());
        listar(&mut r, &a(), 200);
        let ids: Vec<_> = r.partes_pendientes(1000, None).unwrap().into_iter().map(|t| t.asset_id).collect();
        assert_eq!(ids, vec!["A1"]);
        assert_eq!(r.resumen().unwrap().con_error, 0);
    }

    #[test]
    fn relistar_perdona_los_fallos_de_estructura_para_la_doble_descarga() {
        let mut r = Registro::en_memoria().unwrap();
        let ids = ["A1", "A2", "A3", "A4"];
        let a = || ids.iter().map(|i| asset(i, "PrimarySync", AGO, &[(Original, 10)])).collect::<Vec<_>>();
        listar(&mut r, &a(), 100);
        for _ in 0..10 {
            r.registrar_fallo("A1", Original, "verificación: archivo incompleto o dañado: JPEG cortado antes del marcador de fin", 1000).unwrap();
        }
        r.registrar_fallo("A2", Original, "verificación: archivo incompleto o dañado: PNG sin bloque IEND final", 1000).unwrap();
        r.registrar_fallo("A3", Original, "verificación: archivo incompleto o dañado: JPEG sin marcador de fin", 1000).unwrap();
        r.registrar_fallo("A4", Original, "las dos descargas no coinciden", 1000).unwrap();
        listar(&mut r, &a(), 200);
        let pend: Vec<_> = r.partes_pendientes(1000, None).unwrap().into_iter().map(|t| t.asset_id).collect();
        assert_eq!(pend, vec!["A1", "A2", "A3"], "el fallo por descargas distintas sigue con su espera");
        assert_eq!(r.resumen().unwrap().con_error, 0);
        assert_eq!(r.errores(10).unwrap(), vec!["IMG_A4 (original): las dos descargas no coinciden"]);
    }

    #[test]
    fn la_marca_atipica_se_guarda_y_se_borra_si_cambia_el_contenido() {
        let mut r = Registro::en_memoria().unwrap();
        listar(&mut r, &[asset("A1", "PrimarySync", AGO, &[(Original, 10)])], 100);
        r.anotar_sha_pendiente("A1", Original, "abc", true).unwrap();
        assert!(r.partes_pendientes(1000, None).unwrap()[0].estructura_atipica);
        r.asignar_ruta("A1", Original, "x").unwrap();
        r.marcar_verificada_con("A1", Original, "abc", true).unwrap();
        assert!(r.partes_verificadas().unwrap()[0].estructura_atipica);
        assert_eq!(r.resumen().unwrap().estructura_atipica, 1);
        listar(&mut r, &[asset("A1", "PrimarySync", AGO, &[(Original, 10)])], 200);
        assert!(r.partes_verificadas().unwrap()[0].estructura_atipica, "relistar sin cambios la conserva");
        listar(&mut r, &[asset("A1", "PrimarySync", AGO, &[(Original, 11)])], 300);
        assert!(!r.partes_pendientes(1000, None).unwrap()[0].estructura_atipica);
        r.marcar_verificada("A1", Original, "def").unwrap();
        assert_eq!(r.resumen().unwrap().estructura_atipica, 0);
    }

    #[test]
    fn actualizar_enlaces_no_cambia_la_marca_ni_crea_nada() {
        let mut r = Registro::en_memoria().unwrap();
        let a = asset("A1", "PrimarySync", "2026-08-10T19:37:18-04:00", &[(Original, 10)]);
        listar(&mut r, std::slice::from_ref(&a), 1000);
        r.registrar_fallo("A1", Original, "enlace de descarga caducado (HTTP 410)", 1000).unwrap();
        let mut nuevo = a.clone();
        nuevo.partes[0].url = "http://x/nuevo?e=99".into();
        // Una parte nueva o un asset desconocido no se registran aquí (solo el listado completo).
        nuevo.partes.push(ParteIcloud { tipo: Live, tamano: 5, url: "u".into(), extension: "MOV".into(), checksum: "c".into() });
        r.actualizar_enlaces(&nuevo).unwrap();
        let mut otro = asset("B1", "PrimarySync", "2026-08-10T19:37:18-04:00", &[(Original, 10)]);
        otro.partes[0].url = "http://x/b".into();
        r.actualizar_enlaces(&otro).unwrap();
        assert_eq!(r.assets_ultimo_listado().unwrap(), Some(1), "la marca sigue siendo la del listado");
        let p = r.partes_pendientes(1000, None).unwrap();
        assert_eq!(p.len(), 1, "el fallo por enlace caducado se perdona y no aparecen partes nuevas");
        assert_eq!(p[0].url, "http://x/nuevo?e=99");
        assert_eq!(r.trabajos_de("A1").unwrap(), p);
        assert!(r.trabajos_de("B1").unwrap().is_empty());
        let visto: i64 = r.con.query_row("SELECT visto_en_lista FROM assets WHERE id = 'A1'", [], |f| f.get(0)).unwrap();
        assert_eq!(visto, 1000);
    }

    #[test]
    fn filtro_por_mes() {
        let mut r = Registro::en_memoria().unwrap();
        listar(&mut r, &[
            asset("A1", "PrimarySync", AGO, &[(Original, 10)]),
            asset("A2", "PrimarySync", "2026-09-02T10:00:00-04:00", &[(Original, 10)]),
        ], 100);
        let p = r.partes_pendientes(1000, Some("2026-08")).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].asset_id, "A1");
    }

    #[test]
    fn ruta_ocupada_ignora_mayusculas() {
        let mut r = Registro::en_memoria().unwrap();
        listar(&mut r, &[asset("A1", "PrimarySync", AGO, &[(Original, 10)])], 100);
        r.asignar_ruta("A1", Original, r"2026\2026-08\IMG_A1.HEIC").unwrap();
        assert!(r.ruta_ocupada(r"2026\2026-08\img_a1.heic").unwrap());
        assert!(!r.ruta_ocupada(r"2026\2026-08\otra.heic").unwrap());
    }

    #[test]
    fn candidatos_liberar_aplica_reglas() {
        let mut r = Registro::en_memoria().unwrap();
        listar(&mut r, &[asset("A5", "PrimarySync", VIEJO, &[(Original, 1)])], 100);
        dar_por_copiado(&mut r, "A5", Original);
        listar(&mut r, &[
            asset("A1", "PrimarySync", VIEJO, &[(Original, 1), (Live, 1)]),
            asset("A2", "SharedSync-1", VIEJO, &[(Original, 1)]),
            asset("A3", "PrimarySync", VIEJO, &[(Original, 1), (Live, 1)]),
            asset("A4", "PrimarySync", AGO, &[(Original, 1)]),
        ], 200);
        for (id, t) in [("A1", Original), ("A1", Live), ("A2", Original), ("A3", Original), ("A4", Original)] {
            dar_por_copiado(&mut r, id, t);
        }
        let limite = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z").unwrap().timestamp();
        let c = r.candidatos_liberar(200, limite).unwrap();
        assert_eq!(c.candidatos.iter().map(|c| c.asset_id.as_str()).collect::<Vec<_>>(), vec!["A1"]);
        assert_eq!(c.candidatos[0].partes.len(), 2);
        assert_eq!(c.no_verificados, 1); // A3: le falta la Live
        assert_eq!(c.no_liberables, 0);
        r.marcar_borrado("A1", "2026-11-07T10:00:00-05:00").unwrap();
        assert!(r.candidatos_liberar(200, limite).unwrap().candidatos.is_empty());
    }

    #[test]
    fn candidatos_por_anios_e_inventario_coinciden_con_la_seleccion() {
        use crate::criterio::{seleccionar, Criterio};
        let mut r = Registro::en_memoria().unwrap();
        listar(&mut r, &[
            asset("A1", "PrimarySync", "2023-05-01T10:00:00-04:00", &[(Original, 5)]),
            asset("A2", "PrimarySync", "2024-05-01T10:00:00-04:00", &[(Original, 7), (Live, 1)]),
            asset("A3", "PrimarySync", "2024-12-31T23:30:00-05:00", &[(Original, 2)]),
            asset("C1", "SharedSync-1", "2024-05-01T10:00:00-04:00", &[(Original, 1)]),
            asset("A4", "PrimarySync", "2024-06-01T10:00:00-04:00", &[(Original, 1)]),
        ], 100);
        for (id, t) in [("A1", Original), ("A2", Original), ("A2", Live), ("A3", Original), ("C1", Original)] {
            dar_por_copiado(&mut r, id, t);
        }
        let ahora = chrono::DateTime::parse_from_rfc3339("2026-11-07T00:00:00Z").unwrap().timestamp();
        let c = Criterio::Anios([2024].into_iter().collect());
        let cand = r.candidatos_liberar_en(100, c.limite(ahora), c.anios()).unwrap();
        assert_eq!(cand.candidatos.iter().map(|c| c.asset_id.as_str()).collect::<Vec<_>>(), vec!["A2", "A3"], "año local de captura");
        assert_eq!(cand.no_verificados, 1); // A4
        let inv = r.inventario_liberar().unwrap();
        assert_eq!(inv.len(), 5);
        let s = seleccionar(&inv, &c, ahora);
        assert_eq!((s.elementos, s.bytes, s.no_copiados, s.compartida), (2, 10, 1, 1));
        let todo = Criterio::ConservarDias(30);
        let cand = r.candidatos_liberar_en(100, todo.limite(ahora), todo.anios()).unwrap();
        assert_eq!(cand.candidatos.len() as u64, seleccionar(&inv, &todo, ahora).elementos);
        // Sesiones de borrado: la última, sus ids y restaurar.
        r.marcar_borrado_en("A1", "2026-11-01T10:00:00-05:00", Some("s1")).unwrap();
        r.marcar_borrado_en("A2", "2026-11-02T10:00:00-05:00", Some("s2")).unwrap();
        r.marcar_borrado_en("A3", "2026-11-02T10:00:01-05:00", Some("s2")).unwrap();
        assert_eq!(r.ultima_sesion_borrado().unwrap().as_deref(), Some("s2"));
        assert_eq!(r.ids_de_sesion("s2").unwrap(), vec!["A2", "A3"]);
        r.marcar_restaurado("A2").unwrap();
        assert_eq!(r.ids_de_sesion("s2").unwrap(), vec!["A3"]);
        assert_eq!(r.inventario_liberar().unwrap().len(), 3, "lo restaurado vuelve al inventario");
    }

    #[test]
    fn no_liberables_nunca_son_candidatos() {
        let mut r = Registro::en_memoria().unwrap();
        let mut a = asset("A1", "PrimarySync", VIEJO, &[(Original, 1)]);
        a.liberable = false;
        let b = asset("A2", "PrimarySync", VIEJO, &[(Original, 1), (Alternativa, 1)]);
        listar(&mut r, &[a, b], 100);
        for (id, t) in [("A1", Original), ("A2", Original), ("A2", Alternativa)] {
            dar_por_copiado(&mut r, id, t);
        }
        let limite = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z").unwrap().timestamp();
        let c = r.candidatos_liberar(100, limite).unwrap();
        assert_eq!(c.candidatos.iter().map(|c| c.asset_id.as_str()).collect::<Vec<_>>(), vec!["A2"]);
        assert_eq!(c.candidatos[0].partes.len(), 2);
        assert_eq!((c.no_verificados, c.no_liberables), (0, 1));
    }

    #[test]
    fn lo_agregado_hace_poco_se_conserva_aunque_la_foto_sea_vieja() {
        let mut r = Registro::en_memoria().unwrap();
        let mut a = asset("A1", "PrimarySync", VIEJO, &[(Original, 1)]);
        a.fecha_agregada = Some("2026-10-01T10:00:00-04:00".into());
        let b = asset("A2", "PrimarySync", VIEJO, &[(Original, 1)]);
        let mut c = asset("A3", "PrimarySync", VIEJO, &[(Original, 1)]);
        c.fecha_agregada = Some("2024-01-01T10:00:00-05:00".into()); // anterior a la captura: manda la captura
        listar(&mut r, &[a, b, c], 100);
        for id in ["A1", "A2", "A3"] {
            dar_por_copiado(&mut r, id, Original);
        }
        let limite = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z").unwrap().timestamp();
        let c = r.candidatos_liberar(100, limite).unwrap();
        assert_eq!(c.candidatos.iter().map(|c| c.asset_id.as_str()).collect::<Vec<_>>(), vec!["A2", "A3"]);
    }

    #[test]
    fn orden_de_partes_pendientes() {
        let mut r = Registro::en_memoria().unwrap();
        listar(&mut r, &[asset("A1", "PrimarySync", AGO, &[(Editada, 1), (Alternativa, 1), (Live, 1), (Original, 1)])], 100);
        let tipos: Vec<_> = r.partes_pendientes(1000, None).unwrap().iter().map(|t| t.tipo).collect();
        assert_eq!(tipos, vec![Original, Live, Alternativa, Editada]);
    }

    #[test]
    fn migra_base_antigua_sin_sha_pendiente() {
        let con = Connection::open_in_memory().unwrap();
        let viejo = ESQUEMA.replace("  sha_pendiente TEXT,
  estructura_atipica INTEGER NOT NULL DEFAULT 0,
", "").replace("borrado_icloud TEXT,
  liberable INTEGER NOT NULL DEFAULT 1,
  agregado_unix INTEGER", "borrado_icloud TEXT");
        con.execute_batch(&viejo).unwrap();
        assert!(con.prepare("SELECT sha_pendiente FROM partes").is_err());
        assert!(con.prepare("SELECT liberable FROM assets").is_err());
        assert!(con.prepare("SELECT agregado_unix FROM assets").is_err());
        assert!(con.prepare("SELECT estructura_atipica FROM partes").is_err());
        con.execute_batch("INSERT INTO assets(id, biblioteca, nombre_base, fecha, fecha_unix, mes, visto_en_lista) VALUES('A1', 'PrimarySync', 'IMG', 'f', 0, 'm', 1);
           INSERT INTO partes(asset_id, tipo, extension, tamano, checksum_apple, url, ruta, estado, sha256) VALUES('A1', 'original', 'JPG', 1, 'c', 'u', 'r', 'verificado', 's');").unwrap();
        let r = Registro::con_esquema(con).unwrap();
        assert!(r.con.prepare("SELECT sha_pendiente FROM partes").is_ok());
        assert!(r.con.prepare("SELECT liberable, agregado_unix FROM assets").is_ok());
        let p = r.partes_verificadas().unwrap();
        assert!(!p[0].estructura_atipica, "lo ya verificado queda como estándar");
        assert_eq!(r.resumen().unwrap().estructura_atipica, 0);
        // Idempotente.
        let r = Registro::con_esquema(r.con).unwrap();
        assert_eq!(r.partes_verificadas().unwrap().len(), 1);
    }

    #[test]
    fn sha_pendiente_se_limpia_al_verificar() {
        let mut r = Registro::en_memoria().unwrap();
        listar(&mut r, &[asset("A1", "PrimarySync", AGO, &[(Original, 10)])], 100);
        r.anotar_sha_pendiente("A1", Original, "abc", false).unwrap();
        assert_eq!(r.partes_pendientes(1000, None).unwrap()[0].sha_pendiente.as_deref(), Some("abc"));
        dar_por_copiado(&mut r, "A1", Original);
        r.marcar_para_redescarga("A1", Original, "x").unwrap();
        assert_eq!(r.partes_pendientes(1000, None).unwrap()[0].sha_pendiente, None);
    }

    #[test]
    fn meta_guardar_leer_y_borrar() {
        let mut r = Registro::en_memoria().unwrap();
        assert_eq!(r.leer_meta("x").unwrap(), None);
        r.guardar_meta("x", "1").unwrap();
        r.guardar_meta("x", "2").unwrap();
        assert_eq!(r.leer_meta("x").unwrap().as_deref(), Some("2"));
        r.borrar_meta("x").unwrap();
        assert_eq!(r.leer_meta("x").unwrap(), None);
    }

    #[test]
    fn resumen_y_errores() {
        let mut r = Registro::en_memoria().unwrap();
        listar(&mut r, &[asset("A1", "PrimarySync", AGO, &[(Original, 10), (Live, 5)])], 100);
        dar_por_copiado(&mut r, "A1", Original);
        r.registrar_fallo("A1", Live, "HTTP 503", 1000).unwrap();
        let s = r.resumen().unwrap();
        assert_eq!((s.assets, s.partes, s.verificadas, s.pendientes), (1, 2, 1, 1));
        assert_eq!((s.bytes_total, s.bytes_verificados), (15, 10));
        let p = r.progreso().unwrap();
        assert_eq!((p.partes, p.verificadas, p.con_error, p.bytes_total, p.bytes_verificados), (2, 1, 0, 15, 10));
        assert_eq!(p.ultimo_listado, r.marca_lista().unwrap());
        assert_eq!(Registro::en_memoria().unwrap().assets_ultimo_listado().unwrap(), None);
        assert_eq!(r.assets_ultimo_listado().unwrap(), Some(1));
        assert_eq!(r.errores(10).unwrap(), vec!["IMG_A1 (live): HTTP 503".to_string()]);
    }
}
