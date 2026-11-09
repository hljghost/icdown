import json
import sys

caducada = "--caducada" in sys.argv


def asset(i):
    return {"id": i, "biblioteca": "PrimarySync", "nombre_base": "IMG_" + i,
            "fecha": "2026-08-10T19:37:18-04:00",
            "partes": [{"tipo": "original", "tamano": 3, "url": "http://127.0.0.1:1/" + i,
                        "extension": "JPG", "checksum": "c"}]}


def out(o):
    sys.stdout.write(json.dumps(o) + "\n")
    sys.stdout.flush()


for linea in sys.stdin:
    p = json.loads(linea)
    c = p.get("cmd")
    if caducada:
        out({"ok": False, "error": "sesion_caducada"})
    elif c == "sesion":
        out({"ok": True})
    elif c == "login":
        out({"ok": True, "requiere_2fa": True})
    elif c == "codigo_2fa":
        out({"ok": True} if p["codigo"] == "123456" else {"ok": False, "error": "codigo_incorrecto"})
    elif c == "listar":
        out({"asset": asset("A1")})
        out({"aviso": "asset X omitido: roto"})
        out({"asset": asset("A2")})
        out({"ok": True, "total": 2, "omitidos": 1})
    elif c == "borrar":
        ids = p["ids"]
        out({"ok": True, "borrados": [i for i in ids if i != "MALO"],
             "fallidos": [{"id": "MALO", "error": "reintentar"}] if "MALO" in ids else []})
    elif c == "restaurar":
        ids = p["ids"]
        out({"ok": True, "restaurados": [i for i in ids if i != "PERDIDO"],
             "fallidos": [{"id": "PERDIDO", "error": "no_en_eliminado_recientemente"}] if "PERDIDO" in ids else []})
    elif c == "refrescar":
        ids = p["ids"]
        for i in ids:
            if i.startswith("A"):
                out({"asset": asset(i)})
        out({"ok": True, "total": sum(1 for i in ids if i.startswith("A")),
             "faltan": [i for i in ids if not i.startswith("A")]})
    else:
        out({"ok": False, "error": "otro", "detalle": "comando desconocido: " + str(c)})
