import io
import json
import os
import sys
from datetime import datetime, timezone

import pytest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import puente_icloud as pi  # noqa: E402
from pyicloud_ipd.exceptions import PyiCloudFailedLoginException  # noqa: E402
from pyicloud_ipd.item_type import AssetItemType  # noqa: E402
from pyicloud_ipd.version_size import AssetVersionSize, LivePhotoVersionSize  # noqa: E402


class V:
    def __init__(self, size, url, type, checksum="ck"):
        self.size = size
        self.url = url
        self.type = type
        self.checksum = checksum


class AssetFalso:
    def __init__(self, id="AbC-123+xyz", nombre="IMG_0001.HEIC", versiones=None,
                 item_type=AssetItemType.IMAGE, campos=None, campos_master=None,
                 fecha=datetime(2026, 8, 10, 23, 37, 18, tzinfo=timezone.utc)):
        self.id = id
        self._nombre = nombre
        self.versions = versiones if versiones is not None else {}
        self.item_type = item_type
        self.item_type_extension = "HEIC"
        self._asset_record = {"fields": campos or {}, "recordChangeTag": "tag1", "recordName": id}
        self._master_record = {"fields": campos_master or {}, "recordName": id}
        self.asset_date = fecha
        self.has_asset_date = True
        self.added_date = fecha

    def calculate_filename(self):
        return self._nombre


class ApiFalsa:
    def __init__(self, requires_2fa=False):
        self.requires_2fa = requires_2fa
        self.requires_2sa = False
        self.pushes = 0

    def trigger_push_notification(self):
        self.pushes += 1
        return True

    def validate_2fa_code(self, codigo):
        return codigo == "123456"


@pytest.fixture(autouse=True)
def salida(monkeypatch):
    buf = io.StringIO()
    monkeypatch.setattr(pi, "_salida", buf)
    return buf


def lineas(buf):
    return [json.loads(l) for l in buf.getvalue().splitlines()]


def test_live_photo_editada_tiene_tres_partes():
    a = AssetFalso(
        versiones={
            AssetVersionSize.ORIGINAL: V(100, "u1", "public.heic"),
            LivePhotoVersionSize.ORIGINAL: V(50, "u2", "com.apple.quicktime-movie"),
            AssetVersionSize.ADJUSTED: V(80, "u3", "public.jpeg"),
        },
        campos={"resJPEGFullRes": {}},
    )
    j = pi.asset_a_json(a, "PrimarySync")
    assert j["nombre_base"] == "IMG_0001"
    assert j["biblioteca"] == "PrimarySync"
    assert [(p["tipo"], p["extension"], p["tamano"]) for p in j["partes"]] == [
        ("original", "HEIC", 100), ("live", "MOV", 50), ("editada", "JPG", 80)]


def test_sin_edicion_no_incluye_editada():
    a = AssetFalso(versiones={
        AssetVersionSize.ORIGINAL: V(100, "u1", "public.heic"),
        AssetVersionSize.ADJUSTED: V(100, "u1", "public.heic"),
    })
    assert [p["tipo"] for p in pi.asset_a_json(a, "PrimarySync")["partes"]] == ["original"]


def test_video_solo_original():
    a = AssetFalso(nombre="IMG_0002.MOV", item_type=AssetItemType.MOVIE, versiones={
        AssetVersionSize.ORIGINAL: V(900, "u", "com.apple.quicktime-movie"),
        LivePhotoVersionSize.ORIGINAL: V(1, "x", "com.apple.quicktime-movie"),
    })
    partes = pi.asset_a_json(a, "PrimarySync")["partes"]
    assert [(p["tipo"], p["extension"]) for p in partes] == [("original", "MOV")]


def test_sin_original_se_omite():
    assert pi.asset_a_json(AssetFalso(versiones={}), "PrimarySync") is None


def test_sin_nombre_usa_respaldo():
    a = AssetFalso(nombre=None, versiones={AssetVersionSize.ORIGINAL: V(1, "u", "public.heic")})
    assert pi.asset_a_json(a, "PrimarySync")["nombre_base"] == "AbC_123_xyz"


def test_extension():
    assert pi.extension_de("public.webp", "foto.webp") == "WEBP"
    assert pi.extension_de("public.heic", "x.jpg") == "HEIC"
    assert pi.extension_de("raro", "sinext") == "BIN"


def test_fecha_local_con_desfase():
    a = AssetFalso(versiones={AssetVersionSize.ORIGINAL: V(1, "u", "public.heic")})
    f = datetime.fromisoformat(pi.asset_a_json(a, "PrimarySync")["fecha"])
    assert f.utcoffset() is not None
    assert f == a.asset_date


def test_sesion_sin_token_es_caducada():
    def fabrica(_pw):
        raise PyiCloudFailedLoginException("sin sesión")
    p = pi.Puente("yo@x.com", "c", fabrica=fabrica)
    with pytest.raises(pi.SesionCaducada):
        p.manejar({"cmd": "sesion"})


def test_sesion_que_pide_2fa_es_caducada():
    p = pi.Puente("yo@x.com", "c", fabrica=lambda _pw: ApiFalsa(requires_2fa=True))
    with pytest.raises(pi.SesionCaducada):
        p.manejar({"cmd": "sesion"})


def test_login_y_codigo(salida):
    api = ApiFalsa(requires_2fa=True)
    p = pi.Puente("yo@x.com", "c", fabrica=lambda _pw: api)
    p.manejar({"cmd": "login", "password": "x"})
    p.manejar({"cmd": "codigo_2fa", "codigo": "000000"})
    p.manejar({"cmd": "codigo_2fa", "codigo": "123456"})
    assert lineas(salida) == [
        {"ok": True, "requiere_2fa": True},
        {"ok": False, "error": "codigo_incorrecto"},
        {"ok": True},
    ]
    assert api.pushes == 1  # Apple solo envía el código si se le pide


def test_login_sin_2fa_no_pide_codigo(salida):
    api = ApiFalsa(requires_2fa=False)
    p = pi.Puente("yo@x.com", "c", fabrica=lambda _pw: api)
    p.manejar({"cmd": "login", "password": "x"})
    assert api.pushes == 0


def test_login_con_contrasena_mala():
    def fabrica(_pw):
        raise PyiCloudFailedLoginException("mala")
    p = pi.Puente("yo@x.com", "c", fabrica=fabrica)
    with pytest.raises(pi.Credenciales):
        p.manejar({"cmd": "login", "password": "x"})


def test_borrar_solo_lo_listado_y_personal(salida):
    borrados = []
    p = pi.Puente("yo@x.com", "c", fabrica=lambda _pw: ApiFalsa(),
                  borrador=lambda lib, a: borrados.append(a.id))
    p.cache = {
        "A1": ("PrimarySync", "lib", AssetFalso(id="A1")),
        "S1": ("SharedSync-1", "lib2", AssetFalso(id="S1")),
    }
    p.manejar({"cmd": "borrar", "ids": ["A1", "S1", "ZZ"]})
    assert borrados == ["A1"]
    assert lineas(salida) == [{
        "ok": True, "borrados": ["A1"],
        "fallidos": [{"id": "S1", "error": "biblioteca_compartida"},
                     {"id": "ZZ", "error": "no_listado"}],
    }]


def test_listar_emite_assets_avisos_y_total(salida):
    class Lib:
        def __init__(self, assets):
            self.all = assets

    class Rota:
        id = "A3"

        @property
        def versions(self):
            raise ValueError("roto")

    buena = AssetFalso(id="A1", versiones={AssetVersionSize.ORIGINAL: V(1, "u", "public.heic")})
    sin_original = AssetFalso(id="A2", versiones={})
    p = pi.Puente("yo@x.com", "c", fabrica=lambda _pw: ApiFalsa())
    p.bibliotecas = lambda: [("PrimarySync", Lib([buena, sin_original, Rota()]))]
    p.manejar({"cmd": "listar"})
    ls = lineas(salida)
    assert ls[0]["asset"]["id"] == "A1"
    assert "aviso" in ls[1]
    assert ls[-1] == {"ok": True, "total": 1, "omitidos": 2}
    assert set(p.cache) == {"A1"}


def _res(size, url, ck="k"):
    return {"value": {"size": size, "downloadURL": url, "fileChecksum": ck}}


def test_raw_jpeg_copia_la_alternativa():
    a = AssetFalso(versiones={
        AssetVersionSize.ORIGINAL: V(100, "u1", "public.jpeg"),
        AssetVersionSize.ALTERNATIVE: V(300, "u2", "com.adobe.raw-image"),
    }, campos_master={"resOriginalRes": {}, "resOriginalAltRes": {}})
    j = pi.asset_a_json(a, "PrimarySync")
    assert [(p["tipo"], p["extension"], p["tamano"], p["url"]) for p in j["partes"]] == [
        ("original", "JPG", 100, "u1"), ("alternativa", "DNG", 300, "u2")]
    assert j["liberable"] is True


def test_video_editado_copia_el_render():
    a = AssetFalso(nombre="IMG_0002.MOV", item_type=AssetItemType.MOVIE, versiones={
        AssetVersionSize.ORIGINAL: V(900, "u", "com.apple.quicktime-movie"),
    }, campos={"resVidFullRes": _res(700, "uv", "ckv"),
               "resVidFullFileType": {"value": "com.apple.quicktime-movie"},
               "resVidMedRes": {}, "resVidSmallRes": {}, "resJPEGThumbRes": {}},
       campos_master={"resOriginalRes": {}})
    j = pi.asset_a_json(a, "PrimarySync")
    assert [(p["tipo"], p["extension"], p["tamano"], p["url"], p["checksum"]) for p in j["partes"]] == [
        ("original", "MOV", 900, "u", "ck"), ("editada", "MOV", 700, "uv", "ckv")]
    assert j["liberable"] is True


def test_derivados_no_impiden_liberar():
    a = AssetFalso(versiones={AssetVersionSize.ORIGINAL: V(100, "u1", "public.heic")},
                   campos={"resJPEGMedRes": {}, "resJPEGThumbRes": {}, "resJPEGLargeRes": {}},
                   campos_master={"resOriginalRes": {}, "resJPEGThumbRes": {}})
    assert pi.asset_a_json(a, "PrimarySync")["liberable"] is True


def test_datos_desconocidos_no_son_liberables():
    for campos, master in [({"resSidecarRes": {}}, {}),
                           ({}, {"resAlgoNuevoRes": {}}),
                           ({"resVidFullRes": {}}, {})]:  # vídeo editado de una Live Photo: no se copia
        a = AssetFalso(versiones={AssetVersionSize.ORIGINAL: V(100, "u1", "public.heic")},
                       campos=campos, campos_master=master)
        assert pi.asset_a_json(a, "PrimarySync")["liberable"] is False, (campos, master)


class Respuesta:
    def __init__(self, cuerpo, status_code=200, ok=None):
        self._cuerpo = cuerpo
        self.status_code = status_code
        self.ok = (status_code < 400) if ok is None else ok

    def json(self):
        if isinstance(self._cuerpo, Exception):
            raise self._cuerpo
        return self._cuerpo


class SesionFalsa:
    def __init__(self, respuesta):
        self.respuesta = respuesta
        self.envios = []

    def post(self, url, data=None, headers=None):
        self.envios.append(json.loads(data))
        return self.respuesta


class LibFalsa:
    def __init__(self, respuesta):
        self.service_endpoint = "https://x"
        self.params = {"a": "1"}
        self.zone_id = {"zoneName": "PrimarySync"}
        self.session = SesionFalsa(respuesta)


def _borrar_con(respuesta, id="A1"):
    lib = LibFalsa(respuesta)
    return pi.borrar_en_icloud(lib, AssetFalso(id=id)), lib


def test_borrar_en_icloud_exito_solo_con_el_registro_pedido():
    r, lib = _borrar_con(Respuesta({"records": [{"recordName": "A1", "fields": {"isDeleted": {"value": 1}}}]}))
    assert r is None
    assert lib.session.envios[0]["operations"][0]["record"]["recordName"] == "A1"


def test_borrar_en_icloud_respuestas_dudosas_no_cuentan():
    casos = [
        Respuesta({}),                                            # sin registros
        Respuesta({"records": []}),
        Respuesta({"records": [{"recordName": "OTRO"}]}),          # otro registro
        Respuesta(ValueError("no json")),                         # cuerpo ilegible
        Respuesta({"records": [{"recordName": "A1"}]}, status_code=500),  # HTTP de error
        Respuesta({"records": [{"recordName": "A1"}]}, ok=False),
        Respuesta(["no", "es", "dict"]),
    ]
    for c in casos:
        assert _borrar_con(c)[0] == "respuesta_desconocida", c._cuerpo


def test_borrar_en_icloud_devuelve_el_error_del_servidor():
    r, _ = _borrar_con(Respuesta({"records": [{"recordName": "A1", "serverErrorCode": "ACCESS_DENIED"}]}))
    assert r == "ACCESS_DENIED"
    # Op-Lock y similares: se reintenta más tarde.
    r, _ = _borrar_con(Respuesta({"records": [{"recordName": "A1", "serverErrorCode": "CONFLICT"}]}))
    assert r == "reintentar"
    r, _ = _borrar_con(Respuesta({"records": [{"recordName": "A1"}]}, status_code=503))
    assert r == "limite_apple"


def test_borrar_ante_el_limite_de_apple_para_el_lote(salida):
    from pyicloud_ipd.exceptions import PyiCloudServiceUnavailableException
    intentados = []

    def borrador(lib, a):
        intentados.append(a.id)
        if a.id == "A2":
            raise PyiCloudServiceUnavailableException("Apple iCloud is temporary refusing to serve icloudpd")

    p = _puente_con_cache(borrador, ["A1", "A2", "A3"])
    p.manejar({"cmd": "borrar", "ids": ["A1", "A2", "A3"]})
    assert intentados == ["A1", "A2"], "no se insiste con el resto del lote"
    assert lineas(salida) == [{"ok": True, "borrados": ["A1"],
                               "fallidos": [{"id": "A2", "error": "limite_apple"},
                                            {"id": "A3", "error": "limite_apple"}]}]


def _puente_con_cache(borrador, ids):
    p = pi.Puente("yo@x.com", "c", fabrica=lambda _pw: ApiFalsa(), borrador=borrador)
    p.cache = {i: ("PrimarySync", "lib", AssetFalso(id=i)) for i in ids}
    return p


def test_borrar_excepcion_en_medio_no_pierde_lo_hecho(salida):
    intentados = []

    def borrador(lib, a):
        intentados.append(a.id)
        if a.id == "A2":
            raise ValueError("explota")
        return None

    p = _puente_con_cache(borrador, ["A1", "A2", "A3"])
    p.manejar({"cmd": "borrar", "ids": ["A1", "A2", "A3"]})
    assert intentados == ["A1", "A2", "A3"]
    assert lineas(salida) == [{"ok": True, "borrados": ["A1", "A3"],
                               "fallidos": [{"id": "A2", "error": "desconocido: ValueError: explota"}]}]


def test_borrar_sesion_caducada_en_medio_responde_parcial(salida):
    from pyicloud_ipd.exceptions import PyiCloudAPIResponseException
    intentados = []

    def borrador(lib, a):
        intentados.append(a.id)
        if a.id == "A2":
            raise PyiCloudAPIResponseException("Authentication required", "401")
        return None

    p = _puente_con_cache(borrador, ["A1", "A2", "A3"])
    p.manejar({"cmd": "borrar", "ids": ["A1", "A2", "A3"]})
    assert intentados == ["A1", "A2"]
    assert lineas(salida) == [{"ok": True, "borrados": ["A1"],
                               "fallidos": [{"id": "A2", "error": "sesion_caducada"},
                                            {"id": "A3", "error": "sesion_caducada"}]}]


def test_listar_incluye_ocultas_sin_duplicar(salida):
    class Lib:
        def __init__(self, todas, ocultas=None):
            self.all = todas
            if ocultas is not None:
                self.hidden = ocultas

    def a(i):
        return AssetFalso(id=i, versiones={AssetVersionSize.ORIGINAL: V(1, "u", "public.heic")})

    p = pi.Puente("yo@x.com", "c", fabrica=lambda _pw: ApiFalsa())
    p.bibliotecas = lambda: [("PrimarySync", Lib([a("A1")], [a("H1"), a("A1")])),
                             ("SharedSync-1", Lib([a("S1")]))]
    p.manejar({"cmd": "listar"})
    ls = lineas(salida)
    assert [l["asset"]["id"] for l in ls if "asset" in l] == ["A1", "H1", "S1"]
    assert ls[-1] == {"ok": True, "total": 3, "omitidos": 0}
    assert p.cache["H1"][0] == "PrimarySync"


def test_fecha_agregada():
    a = AssetFalso(versiones={AssetVersionSize.ORIGINAL: V(1, "u", "public.heic")})
    a.added_date = datetime(2026, 10, 1, 12, 0, 0, tzinfo=timezone.utc)
    j = pi.asset_a_json(a, "PrimarySync")
    f = datetime.fromisoformat(j["fecha_agregada"])
    assert f.utcoffset() is not None
    assert f == a.added_date


def test_fecha_agregada_sin_dato_usa_la_de_captura():
    class SinAgregada(AssetFalso):
        @property
        def added_date(self):
            raise KeyError("addedDate")

        @added_date.setter
        def added_date(self, v):
            pass

    a = SinAgregada(versiones={AssetVersionSize.ORIGINAL: V(1, "u", "public.heic")})
    j = pi.asset_a_json(a, "PrimarySync")
    assert j["fecha_agregada"] == j["fecha"]


# --- refrescar: forma de records/lookup observada en la API real (nov 2026) ---
ZONA = {"zoneName": "PrimarySync", "ownerRecordName": "_abc", "zoneType": "REGULAR_CUSTOM_ZONE"}


def _master(nombre, url, tamano=3249419, tipo="public.heic", fichero="SU1HXzM0MjEuSEVJQw=="):
    return {"recordName": nombre, "recordType": "CPLMaster", "recordChangeTag": "384q", "deleted": False,
            "pluginFields": {}, "fields": {
                "itemType": {"value": tipo, "type": "STRING"},
                "filenameEnc": {"value": fichero, "type": "ENCRYPTED_BYTES"},
                "resOriginalRes": {"value": {"fileChecksum": "ck" + nombre, "size": tamano, "wrappingKey": "k",
                                             "referenceChecksum": "r", "downloadURL": url}},
                "resOriginalFileType": {"value": tipo, "type": "STRING"}}}


def _asset(nombre, master, tag="384r", **extra):
    campos = {"masterRef": {"value": {"recordName": master, "action": "DELETE_SELF", "zoneID": ZONA}, "type": "REFERENCE"},
              "assetDate": {"value": 1794065900000, "type": "TIMESTAMP"},
              "addedDate": {"value": 1794065900785, "type": "TIMESTAMP"}}
    campos.update(extra)
    return {"recordName": nombre, "recordType": "CPLAsset", "recordChangeTag": tag, "deleted": False,
            "pluginFields": {}, "fields": campos}


class SesionLookup:
    """Responde a records/lookup con los registros que tenga para cada recordName pedido."""

    def __init__(self, registros):
        self.registros = registros
        self.envios = []
        self.urls = []

    def post(self, url, data=None, headers=None):
        self.urls.append(url)
        cuerpo = json.loads(data)
        self.envios.append(cuerpo)
        assert len(cuerpo["records"]) <= 200, "la API real rechaza más de 200 registros por petición"
        recs = [self.registros.get(r["recordName"]) or
                {"recordName": r["recordName"], "reason": "Record not found", "serverErrorCode": "NOT_FOUND"}
                for r in cuerpo["records"]]
        return Respuesta({"records": recs, "syncToken": "tok"})


class LibLookup:
    def __init__(self, registros):
        self.service_endpoint = "https://p1-ckdatabasews.icloud.com:443/database/1/com.apple.photos.cloud/production/private"
        self.params = {"dsid": "1", "remapEnums": True}
        self.zone_id = ZONA
        self.session = SesionLookup(registros)


def _puente_con(n, registros_nuevos):
    lib = LibLookup(registros_nuevos)
    p = pi.Puente("a@b.c", "/tmp")
    for k in range(n):
        viejo = pi.PhotoAsset(_master(f"M{k}", f"https://c/viejo{k}?e=100"), _asset(f"A{k}", f"M{k}", tag="viejo"))
        p.cache[f"M{k}"] = ("PrimarySync", lib, viejo)
    return p, lib


def test_refrescar_pide_asset_y_master_y_emite_enlaces_nuevos(salida):
    nuevos = {"A0": _asset("A0", "M0", tag="nuevo"), "M0": _master("M0", "https://c/nuevo0?e=999")}
    p, lib = _puente_con(1, nuevos)
    p.manejar({"cmd": "refrescar", "ids": ["M0"]})
    out = lineas(salida)
    assert out[-1] == {"ok": True, "total": 1, "faltan": []}
    a = out[0]["asset"]
    assert a["id"] == "M0" and a["nombre_base"] == "IMG_3421"
    assert a["partes"][0]["url"] == "https://c/nuevo0?e=999"
    envio = lib.session.envios[0]
    assert sorted(r["recordName"] for r in envio["records"]) == ["A0", "M0"]
    assert envio["zoneID"] == ZONA
    assert "resOriginalRes" in envio["desiredKeys"] and "masterRef" in envio["desiredKeys"]
    assert lib.session.urls[0].startswith(lib.service_endpoint + "/records/lookup?")
    assert "dsid=1" in lib.session.urls[0]
    # La caché queda con el registro fresco (recordChangeTag nuevo, para un borrado posterior).
    assert p.cache["M0"][2]._asset_record["recordChangeTag"] == "nuevo"


def test_refrescar_mismo_json_que_listar_salvo_url(salida):
    viejo = pi.PhotoAsset(_master("M0", "https://c/v?e=1"), _asset("A0", "M0"))
    nuevos = {"A0": _asset("A0", "M0"), "M0": _master("M0", "https://c/n?e=2")}
    p, lib = _puente_con(0, nuevos)
    p.cache["M0"] = ("PrimarySync", lib, viejo)
    p.manejar({"cmd": "refrescar", "ids": ["M0"]})
    antes = pi.asset_a_json(viejo, "PrimarySync")
    despues = lineas(salida)[0]["asset"]
    for x in (antes, despues):
        for parte in x["partes"]:
            parte.pop("url")
    assert antes == despues


def test_refrescar_no_encontrados_borrados_y_no_listados_faltan(salida):
    nuevos = {"A0": _asset("A0", "M0"), "M0": _master("M0", "https://c/n0?e=9"),
              # A1/M1: NOT_FOUND (borrado del todo)
              "A2": _asset("A2", "M2", isDeleted={"value": 1}), "M2": _master("M2", "https://c/n2?e=9")}
    p, lib = _puente_con(3, nuevos)
    p.manejar({"cmd": "refrescar", "ids": ["M0", "M1", "M2", "NO-LISTADO"]})
    out = lineas(salida)
    assert [o["asset"]["id"] for o in out[:-1]] == ["M0"]
    assert out[-1]["total"] == 1
    assert sorted(out[-1]["faltan"]) == ["M1", "M2", "NO-LISTADO"]
    assert "M1" not in p.cache and "M2" not in p.cache


def test_refrescar_en_lotes_de_200_registros(salida):
    nuevos = {}
    for k in range(250):
        nuevos[f"A{k}"] = _asset(f"A{k}", f"M{k}")
        nuevos[f"M{k}"] = _master(f"M{k}", f"https://c/n{k}?e=9")
    p, lib = _puente_con(250, nuevos)
    p.manejar({"cmd": "refrescar", "ids": [f"M{k}" for k in range(250)]})
    assert [len(e["records"]) for e in lib.session.envios] == [200, 200, 100]
    assert lineas(salida)[-1] == {"ok": True, "total": 250, "faltan": []}


def test_refrescar_sesion_caducada_es_error_de_sesion(salida, monkeypatch):
    from pyicloud_ipd.exceptions import PyiCloudAPIResponseException

    p, lib = _puente_con(1, {})

    def falla(*a, **k):
        raise PyiCloudAPIResponseException("Authentication required for Account.", "421")

    monkeypatch.setattr(lib.session, "post", falla)
    with pytest.raises(PyiCloudAPIResponseException) as e:
        p.manejar({"cmd": "refrescar", "ids": ["M0"]})
    assert pi.es_error_de_sesion(e.value)


class ApiConLibs(ApiFalsa):
    def __init__(self, libs):
        super().__init__()
        self.photos = type("Fotos", (), {"private_libraries": libs})()


def _puente_con_papelera(papelera, restaurador):
    p = pi.Puente("yo@x.com", "c", fabrica=lambda _pw: ApiConLibs({"PrimarySync": "lib"}), restaurador=restaurador)
    p.papelera = lambda lib: iter(papelera)
    return p


def test_restaurar_busca_en_eliminado_recientemente(salida):
    hechos = []
    papelera = [AssetFalso(id=i) for i in ["X1", "A1", "A2", "A3"]]
    p = _puente_con_papelera(papelera, lambda lib, a: hechos.append((lib, a.id)) or ("ACCESS_DENIED" if a.id == "A2" else None))
    p.manejar({"cmd": "restaurar", "ids": ["A1", "A2", "ZZ"]})
    assert hechos == [("lib", "A1"), ("lib", "A2")], "solo los pedidos, en la biblioteca personal"
    assert lineas(salida) == [{"ok": True, "restaurados": ["A1"],
                               "fallidos": [{"id": "A2", "error": "ACCESS_DENIED"},
                                            {"id": "ZZ", "error": "no_en_eliminado_recientemente"}]}]


def test_restaurar_sesion_caducada_en_medio(salida):
    from pyicloud_ipd.exceptions import PyiCloudAPIResponseException

    def restaurador(lib, a):
        if a.id == "A2":
            raise PyiCloudAPIResponseException("Authentication required", "401")

    p = _puente_con_papelera([AssetFalso(id=i) for i in ["A1", "A2", "A3"]], restaurador)
    p.manejar({"cmd": "restaurar", "ids": ["A1", "A2", "A3"]})
    assert lineas(salida) == [{"ok": True, "restaurados": ["A1"],
                               "fallidos": [{"id": "A2", "error": "sesion_caducada"},
                                            {"id": "A3", "error": "sesion_caducada"}]}]


def test_restaurar_en_icloud_pone_is_deleted_a_cero():
    lib = LibFalsa(Respuesta({"records": [{"recordName": "A1", "fields": {"isDeleted": {"value": 0}}}]}))
    assert pi.restaurar_en_icloud(lib, AssetFalso(id="A1")) is None
    op = lib.session.envios[0]["operations"][0]
    assert op["record"]["fields"] == {"isDeleted": {"value": 0}}
    assert op["record"]["recordChangeTag"] == "tag1"
    assert pi.restaurar_en_icloud(LibFalsa(Respuesta({"records": []})), AssetFalso(id="A1")) == "respuesta_desconocida"


def test_dominio_china_se_pasa_a_pyicloud(monkeypatch):
    vistos = []
    monkeypatch.setattr(pi, "PyiCloudService", lambda dominio, *a, **k: vistos.append(dominio) or ApiFalsa())
    pi.Puente("yo@x.com", "c", dominio="cn").conectar()
    pi.Puente("yo@x.com", "c").conectar()
    assert vistos == ["cn", "com"]


# --- Borrado en lote (varias operaciones en una sola petición records/modify) ---

def test_borrar_en_lote_una_peticion_y_resultado_por_registro():
    resp = Respuesta({"records": [
        {"recordName": "A1", "fields": {"isDeleted": {"value": 1}}},
        {"recordName": "A2", "serverErrorCode": "CONFLICT"},
        {"recordName": "A3", "serverErrorCode": "ACCESS_DENIED"},
    ]})
    lib = LibFalsa(resp)
    r = pi.marcar_borrado_lote(lib, [AssetFalso(id=i) for i in ("A1", "A2", "A3", "A4")], 1)
    assert len(lib.session.envios) == 1
    envio = lib.session.envios[0]
    assert envio["atomic"] is False
    assert [o["record"]["recordName"] for o in envio["operations"]] == ["A1", "A2", "A3", "A4"]
    # A4 no vino en la respuesta: no se da por borrado
    assert r == {"A1": None, "A2": "reintentar", "A3": "ACCESS_DENIED", "A4": "respuesta_desconocida"}


def test_borrar_en_lote_limite_de_apple_marca_todo_el_lote():
    lib = LibFalsa(Respuesta({}, status_code=503))
    r = pi.marcar_borrado_lote(lib, [AssetFalso(id="A1"), AssetFalso(id="A2")], 1)
    assert r == {"A1": "limite_apple", "A2": "limite_apple"}


def test_puente_borra_en_lotes_de_100_y_se_detiene_si_apple_frena(salida):
    llamadas = []

    def lote(lib, assets):
        llamadas.append(len(assets))
        if len(llamadas) == 2:
            return {a._asset_record["recordName"]: "limite_apple" for a in assets}
        return {a._asset_record["recordName"]: None for a in assets}

    p = pi.Puente("yo@x.com", "c", fabrica=lambda _pw: ApiFalsa(), borrador_lote=lote)
    ids = [f"A{i}" for i in range(250)]
    p.cache = {i: ("PrimarySync", "lib", AssetFalso(id=i)) for i in ids}
    p.cache["S1"] = ("SharedSync-1", "lib2", AssetFalso(id="S1"))
    p.manejar({"cmd": "borrar", "ids": ids + ["S1", "ZZ"]})
    r = lineas(salida)[-1]
    assert llamadas == [100, 100]           # el tercer lote no se envía tras el freno
    assert r["borrados"] == ids[:100]
    errores = {f["id"]: f["error"] for f in r["fallidos"]}
    assert errores["S1"] == "biblioteca_compartida" and errores["ZZ"] == "no_listado"
    assert all(errores[i] == "limite_apple" for i in ids[100:])


def test_puente_sin_borrador_inyectado_usa_lotes():
    p = pi.Puente("yo@x.com", "c", fabrica=lambda _pw: ApiFalsa())
    assert p._borrador_lote is not None
    p2 = pi.Puente("yo@x.com", "c", fabrica=lambda _pw: ApiFalsa(), borrador=lambda l, a: None)
    assert p2._borrador_lote is None


# --- Sesión: errores pasajeros de Apple y escrituras a medias no deben pedir volver a entrar ---

def _error_421():
    from pyicloud_ipd.exceptions import PyiCloudAPIResponseException
    return PyiCloudAPIResponseException("Authentication required for Account.", "421")


class ApiQueFallaUnaVez(ApiFalsa):
    """Las fotos dan 421 hasta que se vuelve a autenticar con el token guardado."""

    def __init__(self, fallos=1, autentica_bien=True):
        super().__init__()
        self.fallos = fallos
        self.autentica_bien = autentica_bien
        self.autenticaciones = 0
        lib = type("Lib", (), {"all": [AssetFalso(id="A1", versiones={
            AssetVersionSize.ORIGINAL: V(10, "https://u/1", "public.heic")})]})()
        self._libs = {"PrimarySync": lib}

    def authenticate(self):
        self.autenticaciones += 1
        if not self.autentica_bien:
            raise PyiCloudFailedLoginException("token inválido")

    @property
    def photos(self):
        if self.fallos > 0:
            self.fallos -= 1
            raise _error_421()
        return type("Fotos", (), {"private_libraries": self._libs})()


def test_421_pasajero_reautentica_con_el_token_y_reintenta(salida):
    api = ApiQueFallaUnaVez()
    creadas = []
    p = pi.Puente("yo@x.com", "c", fabrica=lambda _pw: creadas.append(1) or api)
    p.manejar({"cmd": "listar"})
    assert api.autenticaciones == 1 and len(creadas) == 1
    assert lineas(salida)[-1] == {"ok": True, "total": 1, "omitidos": 0}


def test_421_persistente_sigue_siendo_error_de_sesion(salida):
    from pyicloud_ipd.exceptions import PyiCloudAPIResponseException
    api = ApiQueFallaUnaVez(fallos=5)
    p = pi.Puente("yo@x.com", "c", fabrica=lambda _pw: api)
    with pytest.raises(PyiCloudAPIResponseException) as e:
        p.manejar({"cmd": "listar"})
    assert pi.es_error_de_sesion(e.value) and api.autenticaciones == 1, "un solo reintento"


def test_421_con_token_caducado_es_sesion_caducada(salida):
    p = pi.Puente("yo@x.com", "c", fabrica=lambda _pw: ApiQueFallaUnaVez(autentica_bien=False))
    with pytest.raises(pi.SesionCaducada):
        p.manejar({"cmd": "listar"})


def test_no_reintenta_si_ya_emitio_algo(salida, monkeypatch):
    from pyicloud_ipd.exceptions import PyiCloudAPIResponseException
    api = ApiQueFallaUnaVez(fallos=0)
    p = pi.Puente("yo@x.com", "c", fabrica=lambda _pw: api)

    def a_medias():
        pi.responder({"asset": {"id": "A1"}})
        raise _error_421()

    monkeypatch.setattr(p, "listar", a_medias)
    with pytest.raises(PyiCloudAPIResponseException):
        p.manejar({"cmd": "listar"})
    assert api.autenticaciones == 0, "repetir duplicaría lo ya emitido"


def test_sesion_rechazada_un_instante_se_reintenta_antes_de_pedir_login(salida):
    intentos = []

    def fabrica(_pw):
        intentos.append(1)
        if len(intentos) == 1:
            raise PyiCloudFailedLoginException("validate devolvió 421")
        return ApiFalsa()

    p = pi.Puente("yo@x.com", "c", fabrica=fabrica)
    p._esperar = lambda s: None
    p.manejar({"cmd": "sesion"})
    assert len(intentos) == 2 and lineas(salida) == [{"ok": True}]


def test_escritura_de_sesion_a_medias_no_destruye_la_anterior(tmp_path):
    ruta = tmp_path / "cuenta.session"
    ruta.write_text('{"session_token": "bueno"}', encoding="utf-8")
    with pytest.raises(RuntimeError):
        with pi.abrir_sesion(str(ruta), "w", encoding="utf-8") as f:
            f.write('{"sess')
            raise RuntimeError("el proceso muere a mitad")
    assert json.loads(ruta.read_text(encoding="utf-8")) == {"session_token": "bueno"}
    assert list(tmp_path.iterdir()) == [ruta], "sin temporales sueltos"
    with pi.abrir_sesion(str(ruta), "w", encoding="utf-8") as f:
        json.dump({"session_token": "nuevo"}, f)
    assert json.loads(ruta.read_text(encoding="utf-8")) == {"session_token": "nuevo"}


def test_cookies_se_guardan_de_forma_atomica(tmp_path):
    import http.cookiejar
    ruta = tmp_path / "cuenta"
    tarro = pi.TarroAtomico(filename=str(ruta))
    tarro.set_cookie(http.cookiejar.Cookie(0, "X-APPLE-WEBAUTH-TOKEN", "v", None, False, ".icloud.com", True, True,
                                           "/", True, True, None, False, None, None, {}))
    tarro.save(ignore_discard=True, ignore_expires=True)
    otro = http.cookiejar.LWPCookieJar(filename=str(ruta))
    otro.load(ignore_discard=True, ignore_expires=True)
    assert [c.name for c in otro] == ["X-APPLE-WEBAUTH-TOKEN"]
    assert list(tmp_path.iterdir()) == [ruta]


def test_escrituras_atomicas_se_aplican_a_pyicloud(monkeypatch):
    import http.cookiejar
    import pyicloud_ipd.session as sesion_mod
    monkeypatch.setattr(http.cookiejar, "LWPCookieJar", http.cookiejar.LWPCookieJar)
    monkeypatch.setattr(sesion_mod, "open", open, raising=False)
    pi.activar_escrituras_atomicas()
    assert sesion_mod.open is pi.abrir_sesion
    assert http.cookiejar.LWPCookieJar is pi.TarroAtomico
