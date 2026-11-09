# iCopy 1.0 — macOS, nube, instaladores, versiones y actualizaciones

Fecha: 2026-11-08. Parte del diseño original (`2026-11-07-icopia-design.md`); aquí solo lo nuevo.

## Peticiones

1. Compatibilidad con macOS.
2. Integración real con Google Drive / OneDrive: iniciar sesión y elegir la carpeta destino.
3. "A veces pide volver a iniciar sesión cuando ya lo había hecho".
4. Instalador `.exe` para Windows y app instalable para Mac.
5. Versionado serio y actualizaciones automáticas al publicarse una versión nueva.
6. Otras mejoras.

## Decisiones tomadas

| Tema | Decisión |
| --- | --- |
| Nube | **Copia local + espejo en la nube.** Lo local sigue siendo la copia principal; la nube es una segunda copia verificada. |
| Proveedor | **Google Drive primero** (ver «Registro de la app»). OneDrive después, sobre la misma interfaz. |
| Interfaz | **Webview en los dos sistemas** (wry: WebView2 en Windows, WKWebView en Mac). Sustituye el dibujo GDI. |
| Firma | Mac: **firma Developer ID + notarización** (la persona tiene Apple Developer). Windows: sin firma por ahora. |

## 3. Sesión que se pierde (HECHO)

Causas confirmadas en `puente.log` y en el código de `pyicloud_ipd`:

- **421 pasajero.** Justo después del login Apple a veces responde `Authentication required (421)` a la
  primera consulta de Fotos. El puente lo trataba como sesión caducada y la bandeja abría el login.
  Ahora `sesion`, `listar` y `refrescar` renuevan la sesión con el token guardado (`authenticate()`,
  sin contraseña) y repiten **una vez**, solo si aún no emitieron nada. Una validación rechazada al conectar se
  reintenta a los 3 s antes de darla por caducada.
- **Escrituras a medias.** `pyicloud_ipd` reescribe el `.session` y las cookies en cada petición
  vaciándolos primero; matar el puente (pausa, cancelar, abrir el login, apagado, corte de luz) en ese
  instante dejaba la sesión vacía. Ahora se escribe a un temporal y se sustituye con `os.replace`.

Pendiente menor: un cerrojo sobre `sesion\` para que la CLI y la bandeja no usen la sesión a la vez.

## 5. Versiones y actualizaciones

- **SemVer** con una sola fuente: `version` de `icopia/Cargo.toml`. El puente recibe la misma versión al
  empaquetarse y la devuelve en un comando `version`; la bandeja avisa si no coinciden.
- Visible en: menú de la bandeja (`iCopy 0.2.0`), `icopia --version`, propiedades del `.exe`
  (VERSIONINFO generado en `build.rs`) e `Info.plist` en Mac.
- `CHANGELOG.md` (formato *Keep a Changelog*, en español). Etiquetas `vX.Y.Z`.
- Plan de números: **v0.2.0** = arreglo de sesión + instalador Windows + actualizaciones;
  **v0.3.0** = interfaz webview; **v0.4.0** = macOS; **v0.5.0** = Google Drive; **v1.0.0** cuando todo
  lleve un tiempo estable.
- **CI (GitHub Actions)**: en cada push, `cargo test`, `clippy -D warnings` y `pytest` en Windows y
  macOS. Al empujar una etiqueta `v*`, el flujo de publicación compila, empaqueta, firma y crea el
  *GitHub Release* con las notas del CHANGELOG.

### Actualizaciones automáticas

- La bandeja consulta `api.github.com/repos/Z1Code/iCopy/releases/latest` al arrancar y cada 24 h
  (sin preversiones). Si hay una versión mayor, la descarga en segundo plano.
- Cada artefacto lleva una **firma ed25519 propia** (`.sig`); la clave pública va compilada en la app
  y la privada solo en un secreto de GitHub. Sin firma válida no se instala nada.
- **Nunca se actualiza mientras se libera espacio** ni a mitad de un lote; con una copia en marcha se
  espera a terminar el archivo y se pausa (se reanuda sola tras actualizar).
- Por defecto: aviso «iCopy 0.3.0 está lista — Reiniciar para actualizar» y opción en el menú
  «Actualizar automáticamente» (instala sola en el siguiente momento inactivo).
- Windows: ejecuta el instalador nuevo en modo silencioso, que cierra la app, reemplaza y la vuelve a
  abrir. Mac: descomprime la `.app` nueva, comprueba `codesign --verify`, sustituye la del bundle y relanza.

## 4. Instaladores

- **Windows**: Inno Setup → `iCopy-Setup-X.Y.Z.exe`. Instalación por usuario en
  `%LOCALAPPDATA%\Programs\iCopy` (sin pedir administrador, por eso las actualizaciones son silenciosas),
  acceso en el menú Inicio, arranque con Windows, desinstalador que cierra la app y conserva fotos,
  configuración y sesión (con casilla opcional para borrar configuración y sesión). Migra la instalación
  actual de `Programs\icopia`. Sin firma: SmartScreen avisará; el README lo explica.
- **macOS**: `iCopy.app` (agente de barra de menús, `LSUIElement`) dentro de `iCopy-X.Y.Z-arm64.dmg`
  y `-x64.dmg`, firmada con *Developer ID Application*, *hardened runtime* y notarizada. El puente
  PyInstaller va dentro de `Contents/MacOS/` y se firma también. Para CI: exportar el certificado a `.p12`
  y una clave de App Store Connect como secretos de GitHub (se puede firmar en el Mac M4 mientras tanto).

## 1. macOS

Núcleo (motor, descargas, verificación, registro, liberar, puente) ya es portable. Hay que aislar con
`#[cfg]` o un módulo `plataforma/`:

| Hoy (Windows) | Mac |
| --- | --- |
| `disco.rs` GetDiskFreeSpaceExW | `fs2`/`statvfs` |
| `energia.rs` SetThreadExecutionState | aserción IOPM (`PreventUserIdleSystemSleep`) |
| `autoarranque.rs` HKCU\Run | LaunchAgent en `~/Library/LaunchAgents` |
| `avisos.rs` toast WinRT | `UNUserNotificationCenter` (la app firmada) |
| mutex `CreateMutexW` | cerrojo de archivo en el directorio de la app |
| `puente-icloud.exe` | `puente-icloud` en el bundle |
| `%APPDATA%\icopia` | `~/Library/Application Support/iCopy` (vía `dirs`) |

La bandeja usa `tray-icon` + `tao`, que ya funcionan en Mac. En la barra de menús no existe el *hover*:
el panel se abre con clic.

### Interfaz webview

- `panel.rs` y `asistente.rs` (modelos de vista) se conservan; desaparecen `*_gdi.rs`, `*_ventana.rs`,
  `hover.rs` y `native-windows-gui`. Login, panel y asistente pasan a HTML/CSS/JS embebidos
  (`include_str!`), sin frameworks ni red, con claro/oscuro del sistema.
- Rust → página: el estado serializado en JSON (`window.icopy.actualizar(estado)`).
  Página → Rust: órdenes por IPC (`pausar`, `liberar`, `confirmar`, …), validadas en Rust.
  La confirmación de borrado (casilla + mantener 2 s) se cronometra **en Rust**, no en la página.
- Las imágenes de los ejemplos (`panel_demo`, `asistente_demo`) se siguen generando, ahora con capturas
  del webview, para revisar la UI sin datos reales.
- Requiere WebView2 (de serie en Windows 11; el instalador lo instala si falta en Windows 10).

## 2. Google Drive (espejo)

### Registro de la app (por qué no se puede evitar del todo)

Cualquier integración real con la API de Google o de Microsoft exige que **quien publica la app**
registre un cliente OAuth una vez. **Los usuarios nunca se registran**: solo pulsan «Iniciar sesión
con Google». Google es la opción con menos trámite:

- El registro se automatiza desde aquí con `gcloud`.
- Con el permiso `drive.file` (la app solo ve lo que crea o lo que la persona elige) no hace falta la
  auditoría de seguridad de pago; basta la verificación de marca, que es gratuita. Hasta hacerla, Google
  muestra «app no verificada» y limita a 100 usuarios.
- Microsoft exige un inquilino de Azure/Entra para registrar la app; queda para después.

### Funcionamiento

- Login: OAuth para apps de escritorio con *loopback* + PKCE en el navegador. El *refresh token* va al
  llavero del sistema (Credential Manager / Keychain, crate `keyring`), nunca a `config.json`.
- Elegir carpeta: página local con **Google Picker** (con `drive.file`, la carpeta elegida queda accesible),
  o «Crear carpeta iCopy». Dentro se replica `AAAA/AAAA-MM/`.
- Subida: sesiones reanudables de Drive por fragmentos de 8 MB, con reanudación tras cortes. Solo se suben
  partes ya `verificado` en local.
- Verificación: se compara el `sha256Checksum` que devuelve Drive con el SHA-256 local; si no coincide,
  se borra en Drive y se vuelve a subir.
- Registro: tabla nueva `nube(parte, proveedor, id_remoto, sha256, estado, intentos)` con migración.
- Liberar: con espejo activo, el criterio exige además «verificado en la nube» (opción activada por
  defecto). Cambio en `criterio.rs` **con pruebas**, y las garantías actuales no se debilitan.
- Cuota llena, token revocado o carpeta borrada: se pausa el espejo y se avisa; la copia local sigue.

## 6. Otras mejoras propuestas

1. **Exportar diagnóstico** (menú): zip con logs y estado, sin el Apple ID ni tokens, para que quien
   reporte un fallo lo adjunte.
2. **Rotar `puente.log`** (hoy crece sin límite).
3. **Ventana de ajustes** (destino, concurrencia, intervalo, idioma, nube, actualizaciones) en vez de
   editar `config.json`.
4. **Re-verificación periódica** (mensual) contra corrupción silenciosa del disco.
5. **Álbumes y favoritos**: guardar los álbumes y favoritos de iCloud como metadatos o carpetas de
   enlaces, para no perder la organización.
6. Cerrojo sobre la carpeta de sesión entre la CLI y la bandeja.

## Orden de trabajo

1. v0.2.0: arreglo de sesión (hecho) + versionado + CI + instalador Windows + actualizaciones.
2. v0.3.0: interfaz webview en Windows (paridad con la actual, revisada con capturas).
3. v0.4.0: macOS (plataforma, bundle, firma, notarización, dmg).
4. v0.5.0: Google Drive.
5. Mejoras de la sección 6 entre medias, según prioridad.
