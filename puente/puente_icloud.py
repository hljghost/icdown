"""puente-icloud: puente JSON por líneas entre icopia (Rust) e iCloud Fotos.

Usa la librería pyicloud_ipd de icloudpd v1.33.0. Una petición JSON por línea en stdin;
respuestas JSON por línea en stdout. Todo lo demás (logs) va a stderr.

Peticiones:
  {"cmd":"sesion"}                       -> {"ok":true}
  {"cmd":"login","password":"..."}       -> {"ok":true,"requiere_2fa":bool}
  {"cmd":"codigo_2fa","codigo":"123456"} -> {"ok":true} | {"ok":false,"error":"codigo_incorrecto"}
  {"cmd":"listar"}                       -> {"asset":{...}}* {"aviso":"..."}* {"ok":true,"total":N,"omitidos":M}
  {"cmd":"borrar","ids":[...]}           -> {"ok":true,"borrados":[...],"fallidos":[{"id":..,"error":..}]}
  {"cmd":"restaurar","ids":[...]}        -> {"ok":true,"restaurados":[...],"fallidos":[{"id":..,"error":..}]}
     (saca de "Eliminado recientemente" de la biblioteca personal; los ids son los de listar)
  {"cmd":"refrescar","ids":[...]}        -> {"asset":{...}}* {"ok":true,"total":N,"faltan":[...]}
     (enlaces nuevos para ids del último listado; "faltan": no listados, borrados o sin original)
Errores: {"ok":false,"error":"sesion_caducada"|"credenciales"|"otro","detalle":"..."}
En borrar/restaurar, "limite_apple" = Apple pide esperar (HTTP 503): el resto del lote no se intenta
y vuelve con ese mismo error, para reintentarlo más tarde; "reintentar" = TRY_AGAIN_LATER de un elemento.
"""
from __future__ import annotations

import argparse
import contextlib
import http.cookiejar
import io
import json
import logging
import os
import sys
import time
import traceback
import urllib.parse
from typing import Any, Callable, Dict, List, Optional, Tuple

from pyicloud_ipd.asset_version import ITEM_TYPE_EXTENSIONS, AssetVersion
from pyicloud_ipd.base import PyiCloudService
from pyicloud_ipd.exceptions import (PyiCloudAPIResponseException, PyiCloudFailedLoginException,
                                     PyiCloudServiceUnavailableException)
from pyicloud_ipd.item_type import AssetItemType
from pyicloud_ipd.services.photos import PhotoAlbum, PhotoAsset, filename_with_fallback
from pyicloud_ipd.version_size import AssetVersionSize, LivePhotoVersionSize
from tzlocal import get_localzone

BIBLIOTECA_PERSONAL = "PrimarySync"

_salida: Any = None  # se fija en main(); las pruebas lo sustituyen
_emitidas = 0  # líneas escritas al protocolo: un comando solo se repite si aún no emitió nada

# Comandos que se repiten una vez si Apple pide autenticarse de nuevo (p. ej. 421 justo tras el login).
REINTENTABLES = ("sesion", "listar", "refrescar")


class SesionCaducada(Exception):
    pass


class Credenciales(Exception):
    pass


def responder(obj: Dict[str, Any]) -> None:
    global _emitidas
    _salida.write(json.dumps(obj) + "\n")
    _salida.flush()
    _emitidas += 1


# --- Escrituras atómicas de la sesión ---
# pyicloud_ipd reescribe el archivo de sesión y las cookies en CADA petición, vaciándolos primero.
# Si el proceso muere a mitad (pausa, cancelación, apagado, corte de luz), la sesión queda vacía o
# rota y hay que volver a iniciar sesión. Se escribe a un temporal y se sustituye de golpe.

def _reemplazar(tmp: str, ruta: str) -> None:
    # En Windows, os.replace falla si otro proceso tiene el archivo abierto en ese instante.
    for intento in range(10):
        try:
            os.replace(tmp, ruta)
            return
        except PermissionError:
            if intento == 9:
                logging.warning("No pude actualizar %s (ocupado); se conserva la versión anterior", ruta)
                return
            time.sleep(0.05)


@contextlib.contextmanager
def _escribir_atomico(ruta: str, encoding: str) -> Any:
    tmp = f"{ruta}.{os.getpid()}.tmp"
    try:
        with open(tmp, "w", encoding=encoding) as f:
            yield f
            f.flush()
            os.fsync(f.fileno())
        _reemplazar(tmp, ruta)
    finally:
        if os.path.exists(tmp):
            os.remove(tmp)


def abrir_sesion(ruta: str, modo: str = "r", *args: Any, **kwargs: Any) -> Any:
    """Sustituye a `open` dentro de pyicloud_ipd.session: escrituras atómicas, lecturas normales."""
    if "w" in modo:
        return _escribir_atomico(ruta, kwargs.get("encoding") or "utf-8")
    return open(ruta, modo, *args, **kwargs)


class TarroAtomico(http.cookiejar.LWPCookieJar):
    def save(self, filename: Optional[str] = None, ignore_discard: bool = False,
             ignore_expires: bool = False) -> None:
        ruta = filename or self.filename
        tmp = f"{ruta}.{os.getpid()}.tmp"
        try:
            super().save(tmp, ignore_discard, ignore_expires)
            _reemplazar(tmp, ruta)
        finally:
            if os.path.exists(tmp):
                os.remove(tmp)


def activar_escrituras_atomicas() -> None:
    import pyicloud_ipd.session as sesion_mod
    sesion_mod.open = abrir_sesion  # type: ignore[attr-defined]
    http.cookiejar.LWPCookieJar = TarroAtomico  # type: ignore[misc]


def extension_de(uti: str, nombre: str) -> str:
    if uti in ITEM_TYPE_EXTENSIONS:
        return ITEM_TYPE_EXTENSIONS[uti]
    ext = os.path.splitext(nombre)[1][1:]
    return ext.upper() if ext else "BIN"


def fecha_de(asset: Any) -> str:
    if asset.has_asset_date:
        dt = asset.asset_date
    else:
        try:
            dt = asset.added_date
        except (KeyError, TypeError, ValueError):
            dt = asset.asset_date
    return dt.astimezone(get_localzone()).isoformat()


def fecha_agregada_de(asset: Any, respaldo: str) -> str:
    """Cuándo se añadió a la fototeca (RFC 3339 local); si no se sabe, la fecha de captura."""
    try:
        return asset.added_date.astimezone(get_localzone()).isoformat()
    except (KeyError, TypeError, ValueError, AttributeError):
        return respaldo


def _parte(tipo: str, version: Any, extension: str) -> Dict[str, Any]:
    return {"tipo": tipo, "tamano": int(version.size), "url": version.url,
            "extension": extension, "checksum": version.checksum or ""}


# Campos res*Res que son solo copias reducidas (miniaturas, vistas previas) y se regeneran.
DERIVADOS = {"resJPEGLarge", "resJPEGMed", "resJPEGThumb", "resVidMed", "resVidSmall"}


def _campos(registro: Any) -> Dict[str, Any]:
    if not isinstance(registro, dict):
        return {}
    return registro.get("fields") or {}


def _prefijos_res(asset: Any) -> set:
    """Prefijos de todos los recursos (campos 'res...Res') del asset y su master."""
    prefijos = set()
    for registro in (getattr(asset, "_asset_record", None), getattr(asset, "_master_record", None)):
        for clave in _campos(registro):
            if clave.startswith("res") and clave.endswith("Res") and len(clave) > 6:
                prefijos.add(clave[:-3])
    return prefijos


def _version_de_campos(campos: Dict[str, Any], prefijo: str) -> Optional[AssetVersion]:
    res = (campos.get(f"{prefijo}Res") or {}).get("value")
    tipo = (campos.get(f"{prefijo}FileType") or {}).get("value")
    if not res or not tipo:
        return None
    return AssetVersion(res["size"], res["downloadURL"], tipo, res.get("fileChecksum"))


def asset_a_json(asset: Any, biblioteca: str) -> Optional[Dict[str, Any]]:
    versiones = asset.versions
    original = versiones.get(AssetVersionSize.ORIGINAL)
    if original is None:
        return None
    nombre = filename_with_fallback(asset.id, asset.item_type_extension)(asset.calculate_filename())
    partes = [_parte("original", original, extension_de(original.type, nombre))]
    copiados = {"resOriginal"}
    campos_asset = _campos(asset._asset_record)
    if asset.item_type == AssetItemType.MOVIE:
        if "resVidFullRes" in campos_asset:
            editada = _version_de_campos(campos_asset, "resVidFull")
            if editada is not None:
                partes.append(_parte("editada", editada, extension_de(editada.type, nombre)))
                copiados.add("resVidFull")
    else:
        live = versiones.get(LivePhotoVersionSize.ORIGINAL)
        if live is not None:
            partes.append(_parte("live", live, "MOV"))
            copiados.add("resOriginalVidCompl")
        alternativa = versiones.get(AssetVersionSize.ALTERNATIVE)
        if alternativa is not None:
            partes.append(_parte("alternativa", alternativa, extension_de(alternativa.type, nombre)))
            copiados.add("resOriginalAlt")
        if "resJPEGFullRes" in campos_asset:
            editada = versiones.get(AssetVersionSize.ADJUSTED)
            if editada is not None:
                partes.append(_parte("editada", editada, extension_de(editada.type, nombre)))
                copiados.add("resJPEGFull")
    # Si iCloud guarda algo que no copiamos (y no es una miniatura), borrar el asset lo perdería.
    liberable = not (_prefijos_res(asset) - copiados - DERIVADOS)
    fecha = fecha_de(asset)
    return {"id": asset.id, "biblioteca": biblioteca,
            "nombre_base": os.path.splitext(nombre)[0], "fecha": fecha,
            "fecha_agregada": fecha_agregada_de(asset, fecha),
            "liberable": liberable, "partes": partes}


def borrar_en_icloud(lib: Any, asset: Any) -> Optional[str]:
    """Manda el asset a 'Eliminado recientemente'. Devuelve None si fue bien o un código de error."""
    return marcar_borrado(lib, asset, 1)


def restaurar_en_icloud(lib: Any, asset: Any) -> Optional[str]:
    """Saca el asset de 'Eliminado recientemente' (isDeleted = 0). None si fue bien o un código de error."""
    return marcar_borrado(lib, asset, 0)


LOTE_MODIFY = 100  # operaciones por petición records/modify (CloudKit admite hasta 200 registros)


def marcar_borrado_lote(lib: Any, assets: List[Any], valor: int) -> Dict[str, Optional[str]]:
    """Como marcar_borrado pero con varios assets en UNA petición (atomic=False: cada registro
    se confirma o falla por separado). Devuelve {recordName del CPLAsset: None | código de error}."""
    url = f"{lib.service_endpoint}/records/modify?{urllib.parse.urlencode(lib.params)}"
    nombres = [a._asset_record["recordName"] for a in assets]
    datos = json.dumps({
        "atomic": False,
        "desiredKeys": ["isDeleted"],
        "operations": [{
            "operationType": "update",
            "record": {
                "fields": {"isDeleted": {"value": valor}},
                "recordChangeTag": a._asset_record["recordChangeTag"],
                "recordName": a._asset_record["recordName"],
                "recordType": "CPLAsset",
            },
        } for a in assets],
        "zoneID": lib.zone_id,
    })
    try:
        respuesta = lib.session.post(url, data=datos, headers={"Content-type": "application/json"})
    except PyiCloudAPIResponseException as e:
        if e.code == "TRY_AGAIN_LATER":
            return {n: "reintentar" for n in nombres}
        raise
    estado = getattr(respuesta, "status_code", None)
    if estado in (429, 503):
        return {n: "limite_apple" for n in nombres}
    if getattr(respuesta, "ok", True) is False or (isinstance(estado, int) and estado >= 400):
        return {n: "respuesta_desconocida" for n in nombres}
    try:
        cuerpo = respuesta.json()
    except ValueError:
        return {n: "respuesta_desconocida" for n in nombres}
    registros = cuerpo.get("records") if isinstance(cuerpo, dict) else None
    # Solo cuenta como hecho lo que iCloud confirma explícitamente, registro por registro.
    resultado: Dict[str, Optional[str]] = {n: "respuesta_desconocida" for n in nombres}
    for registro in registros if isinstance(registros, list) else []:
        nombre = registro.get("recordName") if isinstance(registro, dict) else None
        if nombre not in resultado:
            continue
        if "serverErrorCode" in registro:
            codigo = str(registro["serverErrorCode"])
            resultado[nombre] = "reintentar" if codigo in ("TRY_AGAIN_LATER", "CONFLICT", "OPLOCK_FAILURE") else codigo
        else:
            resultado[nombre] = None
    return resultado


def marcar_borrado(lib: Any, asset: Any, valor: int) -> Optional[str]:
    """records/modify del CPLAsset con isDeleted = valor (necesita su recordChangeTag actual)."""
    url = f"{lib.service_endpoint}/records/modify?{urllib.parse.urlencode(lib.params)}"
    datos = json.dumps({
        "atomic": True,
        "desiredKeys": ["isDeleted"],
        "operations": [{
            "operationType": "update",
            "record": {
                "fields": {"isDeleted": {"value": valor}},
                "recordChangeTag": asset._asset_record["recordChangeTag"],
                "recordName": asset._asset_record["recordName"],
                "recordType": "CPLAsset",
            },
        }],
        "zoneID": lib.zone_id,
    })
    try:
        respuesta = lib.session.post(url, data=datos, headers={"Content-type": "application/json"})
    except PyiCloudAPIResponseException as e:
        if e.code == "TRY_AGAIN_LATER":
            return "reintentar"
        raise
    if getattr(respuesta, "status_code", None) in (429, 503):
        return "limite_apple"
    # Solo cuenta como borrado si iCloud confirma explícitamente ESE registro sin error.
    estado = getattr(respuesta, "status_code", None)
    if getattr(respuesta, "ok", True) is False or (isinstance(estado, int) and estado >= 400):
        return "respuesta_desconocida"
    try:
        cuerpo = respuesta.json()
    except ValueError:
        return "respuesta_desconocida"
    registros = cuerpo.get("records") if isinstance(cuerpo, dict) else None
    if not isinstance(registros, list):
        return "respuesta_desconocida"
    nombre = asset._asset_record["recordName"]
    for registro in registros:
        if isinstance(registro, dict) and registro.get("recordName") == nombre:
            if "serverErrorCode" in registro:
                codigo = str(registro["serverErrorCode"])
                # Op-Lock de CloudKit u otro "inténtalo luego": se reintenta más tarde en la misma liberación.
                return "reintentar" if codigo in ("TRY_AGAIN_LATER", "CONFLICT", "OPLOCK_FAILURE") else codigo
            return None
    return "respuesta_desconocida"


# Registros por petición records/lookup (cada asset son dos: CPLAsset y CPLMaster).
REGISTROS_POR_CONSULTA = 200


def claves_deseadas() -> List[str]:
    """Los mismos campos que pide el listado, para que el JSON salga idéntico."""
    album = PhotoAlbum({}, None, "", "", "", "")  # type: ignore[arg-type]
    return list(album._list_query_gen(0, "")["desiredKeys"])


def consultar_registros(lib: Any, nombres: List[str]) -> Dict[str, Dict[str, Any]]:
    """records/lookup de CloudKit: {recordName: registro}. Los que fallan traen 'serverErrorCode'."""
    url = f"{lib.service_endpoint}/records/lookup?{urllib.parse.urlencode(lib.params)}"
    datos = json.dumps({
        "records": [{"recordName": n} for n in nombres],
        "zoneID": lib.zone_id,
        "desiredKeys": claves_deseadas(),
    })
    respuesta = lib.session.post(url, data=datos, headers={"Content-type": "text/plain"})
    cuerpo = respuesta.json()
    registros = cuerpo.get("records") if isinstance(cuerpo, dict) else None
    if not isinstance(registros, list):
        raise ValueError(f"respuesta inesperada de records/lookup: {str(cuerpo)[:200]}")
    return {r["recordName"]: r for r in registros if isinstance(r, dict) and "recordName" in r}


def _registro_valido(registro: Optional[Dict[str, Any]]) -> bool:
    if not isinstance(registro, dict) or "serverErrorCode" in registro or registro.get("deleted"):
        return False
    campos = _campos(registro)
    return not any((campos.get(c) or {}).get("value") for c in ("isDeleted", "isExpunged"))


def es_limite_de_apple(e: Exception) -> bool:
    """Apple frena por exceso de peticiones (p. ej. tras miles de borrados): hay que esperar."""
    if isinstance(e, PyiCloudServiceUnavailableException):
        return True
    codigo = str(getattr(e, "code", "") or "")
    return codigo in ("503", "429") or "temporary refusing" in str(e)


def es_error_de_sesion(e: Exception) -> bool:
    codigo = str(getattr(e, "code", "") or "")
    return codigo in ("401", "421", "AUTHENTICATION_FAILED") or "Authentication required" in str(e)


class Puente:
    def __init__(self, apple_id: str, cookies: str,
                 fabrica: Optional[Callable[[Optional[str]], Any]] = None,
                 borrador: Optional[Callable[[Any, Any], Optional[str]]] = None,
                 restaurador: Optional[Callable[[Any, Any], Optional[str]]] = None,
                 dominio: str = "com",
                 borrador_lote: Optional[Callable[[Any, List[Any]], Dict[str, Optional[str]]]] = None) -> None:
        self.apple_id = apple_id
        self.cookies = cookies
        self.dominio = dominio
        self.api: Any = None
        self._esperar: Callable[[float], None] = time.sleep
        self.cache: Dict[str, Tuple[str, Any, Any]] = {}
        self._fabrica = fabrica or self._fabrica_real
        self._borrador = borrador or borrar_en_icloud
        self._restaurador = restaurador or restaurar_en_icloud
        # Borrado en lote por defecto; si se inyecta un borrador individual (pruebas), se usa ese.
        if borrador_lote is not None:
            self._borrador_lote = borrador_lote
        elif borrador is None:
            self._borrador_lote = lambda lib, assets: marcar_borrado_lote(lib, assets, 1)
        else:
            self._borrador_lote = None

    def _fabrica_real(self, password: Optional[str]) -> Any:
        # "com" (todo el mundo) o "cn" (iCloud de China continental).
        return PyiCloudService(self.dominio, self.apple_id, lambda: password, cookie_directory=self.cookies)

    def conectar(self, password: Optional[str] = None) -> Any:
        try:
            return self._fabrica(password)
        except PyiCloudFailedLoginException as e:
            if password is None:
                raise SesionCaducada() from e
            raise Credenciales(str(e)) from e

    def api_lista(self) -> Any:
        if self.api is None:
            try:
                api = self.conectar()
            except SesionCaducada:
                # Apple a veces rechaza un instante la validación de una sesión buena: se confirma
                # con un segundo intento antes de pedir a la persona que vuelva a entrar.
                logging.warning("La sesión no se validó; reintento antes de darla por caducada")
                self._esperar(3)
                api = self.conectar()
            if api.requires_2fa or api.requires_2sa:
                raise SesionCaducada()
            self.api = api
        return self.api

    def bibliotecas(self) -> List[Tuple[str, Any]]:
        fotos = self.api_lista().photos
        libs = [(n, l) for n, l in fotos.private_libraries.items()
                if n == BIBLIOTECA_PERSONAL or n.startswith("SharedSync-")]
        if not any(n == BIBLIOTECA_PERSONAL for n, _ in libs):
            libs.insert(0, (BIBLIOTECA_PERSONAL, fotos))
        return libs

    def _reautenticar(self) -> None:
        """Renueva la sesión con el token guardado (sin contraseña), sobre el mismo objeto."""
        if self.api is None:
            raise SesionCaducada()
        try:
            self.api.authenticate()
        except PyiCloudFailedLoginException as e:
            raise SesionCaducada() from e
        if self.api.requires_2fa or self.api.requires_2sa:
            raise SesionCaducada()

    def manejar(self, p: Dict[str, Any]) -> None:
        if p.get("cmd") not in REINTENTABLES:
            return self._manejar(p)
        antes = _emitidas
        try:
            self._manejar(p)
        except PyiCloudAPIResponseException as e:
            if not es_error_de_sesion(e) or _emitidas != antes or self.api is None:
                raise
            logging.warning("Apple pide autenticarse de nuevo (%s): renuevo la sesión y reintento", e)
            self._reautenticar()
            self._manejar(p)

    def _manejar(self, p: Dict[str, Any]) -> None:
        cmd = p.get("cmd")
        if cmd == "sesion":
            self.api_lista()
            responder({"ok": True})
        elif cmd == "login":
            self.api = None
            self.api = self.conectar(str(p["password"]))
            requiere_2fa = bool(self.api.requires_2fa or self.api.requires_2sa)
            if requiere_2fa and hasattr(self.api, "trigger_push_notification"):
                # Desde 2026 Apple solo envía el código a los dispositivos de confianza si se
                # pide explícitamente (PUT /verify/trusteddevice/securitycode), como hace icloudpd.
                if not self.api.trigger_push_notification():
                    logging.warning("No se pudo pedir a Apple que envíe el código 2FA")
            responder({"ok": True, "requiere_2fa": requiere_2fa})
        elif cmd == "codigo_2fa":
            if self.api is None:
                raise SesionCaducada()
            if self.api.validate_2fa_code(str(p["codigo"])):
                responder({"ok": True})
            else:
                responder({"ok": False, "error": "codigo_incorrecto"})
        elif cmd == "listar":
            self.listar()
        elif cmd == "borrar":
            self.borrar(list(p.get("ids", [])))
        elif cmd == "restaurar":
            self.restaurar([str(i) for i in p.get("ids", [])])
        elif cmd == "refrescar":
            self.refrescar([str(i) for i in p.get("ids", [])])
        else:
            responder({"ok": False, "error": "otro", "detalle": f"comando desconocido: {cmd}"})

    def _recorrer(self) -> Any:
        """Todos los assets de cada biblioteca, incluido el álbum de ocultas, sin repetir ids."""
        vistos = set()
        for nombre, lib in self.bibliotecas():
            albumes = [lib.all]
            ocultas = getattr(lib, "hidden", None)
            if ocultas is not None:
                albumes.append(ocultas)
            for album in albumes:
                for asset in album:
                    try:
                        i = asset.id
                    except Exception:  # noqa: BLE001 - lo trata asset_a_json
                        i = None
                    if i is not None:
                        if i in vistos:
                            continue
                        vistos.add(i)
                    yield nombre, lib, asset

    def listar(self) -> None:
        self.cache = {}
        total = 0
        omitidos = 0
        for nombre, lib, asset in self._recorrer():
            try:
                j = asset_a_json(asset, nombre)
            except Exception as e:  # un asset raro no debe tumbar el listado
                omitidos += 1
                logging.exception("asset omitido")
                responder({"aviso": f"asset {getattr(asset, 'id', '?')} omitido: {e}"})
                continue
            if j is None:
                omitidos += 1
                continue
            self.cache[asset.id] = (nombre, lib, asset)
            total += 1
            responder({"asset": j})
        responder({"ok": True, "total": total, "omitidos": omitidos})

    def refrescar(self, ids: List[str]) -> None:
        """Enlaces nuevos solo para estos ids (los del último listado), sin volver a listarlo todo."""
        faltan: List[str] = []
        total = 0
        grupos: Dict[int, Tuple[Any, List[Tuple[str, str, Any]]]] = {}
        for i in dict.fromkeys(ids):
            entrada = self.cache.get(i)
            if entrada is None:
                faltan.append(i)
                continue
            biblioteca, lib, asset = entrada
            grupos.setdefault(id(lib), (lib, []))[1].append((i, biblioteca, asset))
        por_consulta = REGISTROS_POR_CONSULTA // 2
        for lib, elementos in grupos.values():
            for inicio in range(0, len(elementos), por_consulta):
                lote = elementos[inicio:inicio + por_consulta]
                nombres: List[str] = []
                for _, _, viejo in lote:
                    nombres += [viejo._asset_record["recordName"], viejo._master_record["recordName"]]
                registros = consultar_registros(lib, list(dict.fromkeys(nombres)))
                for i, biblioteca, viejo in lote:
                    reg_asset = registros.get(viejo._asset_record["recordName"])
                    reg_master = registros.get(viejo._master_record["recordName"])
                    j = None
                    if _registro_valido(reg_asset) and _registro_valido(reg_master):
                        nuevo = PhotoAsset(reg_master, reg_asset)
                        try:
                            j = asset_a_json(nuevo, biblioteca)
                        except Exception:  # noqa: BLE001 - como en listar: no tumba el lote
                            logging.exception("asset %s no se pudo refrescar", i)
                    if j is None:
                        faltan.append(i)
                        self.cache.pop(i, None)
                        continue
                    self.cache[i] = (biblioteca, lib, nuevo)
                    total += 1
                    responder({"asset": j})
        responder({"ok": True, "total": total, "faltan": faltan})

    def borrar(self, ids: List[str]) -> None:
        """Responde siempre con lo hecho hasta el momento, aunque algo falle a mitad del lote."""
        if self._borrador_lote is not None:
            self._borrar_en_lotes(ids)
            return
        borrados: List[str] = []
        fallidos: List[Dict[str, str]] = []
        sesion_caducada = False
        parado: Optional[str] = None
        for i in ids:
            if sesion_caducada:
                fallidos.append({"id": i, "error": "sesion_caducada"})
                continue
            if parado:
                fallidos.append({"id": i, "error": parado})
                continue
            entrada = self.cache.get(i)
            if entrada is None:
                fallidos.append({"id": i, "error": "no_listado"})
                continue
            biblioteca, lib, asset = entrada
            if biblioteca != BIBLIOTECA_PERSONAL:
                fallidos.append({"id": i, "error": "biblioteca_compartida"})
                continue
            error = _aplicar(self._borrador, lib, asset)
            sesion_caducada = error == "sesion_caducada"
            if error == "limite_apple":
                parado = error
            if error:
                fallidos.append({"id": i, "error": error})
            else:
                borrados.append(i)
        responder({"ok": True, "borrados": borrados, "fallidos": fallidos})

    def _borrar_en_lotes(self, ids: List[str]) -> None:
        """Agrupa los ids por biblioteca y los manda en peticiones de hasta LOTE_MODIFY.
        Mismas reglas que el borrado individual; responde con lo hecho aunque algo falle."""
        borrados: List[str] = []
        fallidos: List[Dict[str, str]] = []
        validos: List[Tuple[str, Any, Any]] = []
        for i in ids:
            entrada = self.cache.get(i)
            if entrada is None:
                fallidos.append({"id": i, "error": "no_listado"})
            elif entrada[0] != BIBLIOTECA_PERSONAL:
                fallidos.append({"id": i, "error": "biblioteca_compartida"})
            else:
                validos.append((i, entrada[1], entrada[2]))
        parado: Optional[str] = None
        for desde in range(0, len(validos), LOTE_MODIFY):
            lote = validos[desde:desde + LOTE_MODIFY]
            if parado:
                fallidos += [{"id": i, "error": parado} for i, _, _ in lote]
                continue
            lib = lote[0][1]
            try:
                resultado = self._borrador_lote(lib, [a for _, _, a in lote])
            except Exception as e:  # noqa: BLE001
                error = _codigo_de_excepcion(e, f"lote de {len(lote)}")
                resultado = {a._asset_record["recordName"]: error for _, _, a in lote}
            for i, _, asset in lote:
                error = resultado.get(asset._asset_record["recordName"], "respuesta_desconocida")
                if error in ("limite_apple", "sesion_caducada"):
                    parado = error
                if error:
                    fallidos.append({"id": i, "error": error})
                else:
                    borrados.append(i)
        responder({"ok": True, "borrados": borrados, "fallidos": fallidos})

    def papelera(self, lib: Any) -> Any:
        """Lo que hay en 'Eliminado recientemente' de esa biblioteca."""
        return lib.recently_deleted

    def restaurar(self, ids: List[str]) -> None:
        """Busca los ids en 'Eliminado recientemente' de la biblioteca personal (ahí traen su
        recordChangeTag actual) y los restaura. Responde siempre con lo hecho hasta el momento."""
        pendientes = dict.fromkeys(ids)
        restaurados: List[str] = []
        fallidos: List[Dict[str, str]] = []
        lib = next(l for n, l in self.bibliotecas() if n == BIBLIOTECA_PERSONAL)
        sesion_caducada = False
        limite = False
        for asset in self.papelera(lib):
            if not pendientes or sesion_caducada or limite:
                break
            i = asset.id
            if i not in pendientes:
                continue
            del pendientes[i]
            error = _aplicar(self._restaurador, lib, asset)
            sesion_caducada = error == "sesion_caducada"
            limite = error == "limite_apple"
            if error:
                fallidos.append({"id": i, "error": error})
            else:
                restaurados.append(i)
        motivo = "sesion_caducada" if sesion_caducada else "limite_apple" if limite else "no_en_eliminado_recientemente"
        fallidos += [{"id": i, "error": motivo} for i in pendientes]
        responder({"ok": True, "restaurados": restaurados, "fallidos": fallidos})


def _aplicar(funcion: Callable[[Any, Any], Optional[str]], lib: Any, asset: Any) -> Optional[str]:
    """Borra o restaura un asset; una excepción se convierte en código de error (nada debe perder lo ya hecho)."""
    try:
        return funcion(lib, asset)
    except Exception as e:  # noqa: BLE001
        return _codigo_de_excepcion(e, getattr(asset, "id", "?"))


def _codigo_de_excepcion(e: Exception, que: str) -> str:
    """Traduce una excepción de iCloud a un código de error del protocolo."""
    if es_limite_de_apple(e):
        logging.warning("Apple pide esperar: %s", e)
        return "limite_apple"
    if isinstance(e, SesionCaducada) or (
            isinstance(e, PyiCloudAPIResponseException) and es_error_de_sesion(e)):
        return "sesion_caducada"
    logging.exception("fallo con %s", que)
    return f"desconocido: {type(e).__name__}: {e}"


def main(argv: Optional[List[str]] = None) -> None:
    global _salida
    ap = argparse.ArgumentParser()
    ap.add_argument("--cookies", required=True)
    ap.add_argument("--apple-id", required=True)
    ap.add_argument("--dominio", choices=["com", "cn"], default="com")
    args = ap.parse_args(argv)
    _salida = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", newline="\n")
    sys.stdout = sys.stderr  # nada de la librería puede ensuciar el protocolo
    logging.basicConfig(stream=sys.stderr, level=logging.INFO,
                        format="%(asctime)s %(levelname)s %(message)s")
    activar_escrituras_atomicas()
    puente = Puente(args.apple_id, args.cookies, dominio=args.dominio)
    for linea in io.TextIOWrapper(sys.stdin.buffer, encoding="utf-8"):
        linea = linea.strip()
        if not linea:
            continue
        try:
            puente.manejar(json.loads(linea))
        except SesionCaducada:
            responder({"ok": False, "error": "sesion_caducada"})
        except Credenciales as e:
            responder({"ok": False, "error": "credenciales", "detalle": str(e)})
        except PyiCloudAPIResponseException as e:
            if es_error_de_sesion(e):
                responder({"ok": False, "error": "sesion_caducada"})
            else:
                traceback.print_exc()
                responder({"ok": False, "error": "otro", "detalle": str(e)})
        except Exception as e:
            traceback.print_exc()
            responder({"ok": False, "error": "otro", "detalle": f"{type(e).__name__}: {e}"})


if __name__ == "__main__":
    main()
