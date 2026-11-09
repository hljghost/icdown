# icopia — Diseño

Fecha: 2026-11-07 · Estado: aprobado por el usuario (partes 1–3)

## Problema

iPhone con **Fotos en iCloud activado + "Optimizar almacenamiento"**. Los originales viven
solo en iCloud; el teléfono guarda versiones reducidas. Las copias por USB en Windows (WPD/Explorador/
Fotos) se cuelgan porque el iPhone intenta bajar originales de iCloud y convertir HEIC→JPG en vivo,
y además piden permiso en el iPhone en cada conexión. El usuario paga iCloud solo por acumulación.

## Objetivo

Herramienta para Windows que:
1. Baja **todos los originales** de iCloud Fotos a un destino elegido, organizado `AAAA\AAAA-MM\`.
2. Prioriza la **copia ininterrumpida**: reanuda tras cortes, nunca deja archivos a medias, nunca duplica.
3. **Verifica** cada archivo antes de darlo por bueno.
4. En un paso aparte y con confirmación, **libera espacio** borrando de iCloud (y por sincronización del
   iPhone) solo lo verificado.

No usa el cable USB → no aparece el aviso de permisos del iPhone.

## Fuera de alcance

- Copia por USB (con "Optimizar" activo no tiene los originales).
- Deduplicar o reorganizar backups viejos (una carpeta de backups manuales anteriores queda intacta).
- Vaciar "Eliminado recientemente".
- Fototeca Compartida de iCloud (se lista pero nunca se borra).
- Reimplementar en Rust la autenticación de Apple.

## Arquitectura

```
icopia.exe (Rust, app de bandeja)
 ├─ registro    SQLite (WAL): assets, partes, estados, SHA-256, intentos, log de borrados
 ├─ descargador reqwest, N=3–4 concurrentes, .part + reanudación por Range si el servidor la admite
 ├─ verificador tamaño exacto + estructura de contenedor + SHA-256 releído del disco
 ├─ organizador ruta destino por fecha de captura (hora local), sufijo ante colisión
 ├─ liberador   reglas de seguridad, tandas de 50, log CSV
 └─ bandeja     icono + menú, notificaciones, ventanas de login / liberar
        │ JSON por líneas (stdin/stdout)
puente-icloud.exe (Python, librería pyicloud_ipd de icloudpd con versión fija, empaquetado)
   login + 2FA · listar assets (id, nombre, fecha, partes con tamaño y URL) · refrescar URL · borrar
```

Rust hace las descargas (lo pesado y lo crítico). Python solo habla con Apple.

## Organización en destino

```
<destino>\AAAA\AAAA-MM\IMG_4521.HEIC
                      IMG_4521.MOV              (vídeo de Live Photo)
                      IMG_4521 (editada).HEIC   (si hay edición)
<destino>\.icopia\registro.db, logs\, borrados.csv
```
Colisión de nombre de un asset distinto → `IMG_0001 (ab12cd).HEIC` (6 primeros caracteres del id).

## Estados de cada parte

`pendiente → descargando → descargado → verificado`; fallo → reintento con espera creciente
(5 s, 30 s, 2 min… máx. 10) → `error` (se reintenta en la siguiente ronda). Un asset está
**verificado** solo si todas sus partes lo están.

## Robustez

- Escritura a `.part`; renombrado atómico solo tras verificar.
- Reanudación al reiniciar a partir del registro; Range si el servidor lo admite, si no, rehace el archivo.
- Bloqueo de suspensión mientras copia (`SetThreadExecutionState`).
- Comprobación de espacio libre antes de cada tanda; pausa y aviso si no cabe.
- Destino ausente (disco externo) → espera y avisa.
- Sesión caducada → pausa, icono de "iniciar sesión", notificación.
- URL caducada → pide una nueva al puente.

## Verificación

1. Tamaño en disco == tamaño que reporta iCloud.
2. Contenedor completo: JPEG (SOI…EOI), PNG (IEND), HEIC/MOV/MP4 (cajas ISO-BMFF que cubren todo el archivo).
3. SHA-256 calculado releyendo desde disco tras `fsync`; se guarda para "Verificar todo".
Investigación pendiente: si el checksum propio de Apple es comprobable, se añade como cuarta capa.

## Liberar espacio

Reglas: (1) solo assets con todas sus partes verificadas; (2) re-verificación en disco justo antes
(existe, tamaño, SHA-256); (3) conservar los últimos 30 días (configurable); (4) nunca la Fototeca
Compartida; (5) tandas de 50, registradas, reanudables; (6) va a "Eliminado recientemente";
(7) nunca borra del PC; log CSV. Confirmación escribiendo `BORRAR` tras ver el resumen
(n.º de archivos, GB, rango de fechas, excluidos).

## Interfaz

Icono en la bandeja, arranca con Windows, revisa iCloud cada pocas horas. Menú: estado, Copiar ahora /
Pausar, Verificar todo, Liberar espacio…, Abrir carpeta, Cambiar destino…, Iniciar sesión…, Iniciar con
Windows, Salir. Color del icono = estado. Notificaciones al terminar, en errores y al caducar la sesión.

## Pruebas

Unitarias (estados, verificador con archivos truncados, colisiones de nombre, reglas de borrado),
integración con un iCloud falso (puente simulado + servidor HTTP local con cortes), y prueba real
por fases: 1 mes → todo → primer "Liberar" con un mes viejo.
