#!/usr/bin/env python3
"""Status / logout helper for the ghost.icdown Omarchy bar plugin."""
from __future__ import annotations

import argparse
import fcntl
import json
import os
import shutil
import sqlite3
import subprocess
from datetime import datetime
from pathlib import Path


def config_dir() -> Path:
    override = os.environ.get("ICDOWN_DIR")
    if override:
        return Path(override)
    return Path.home() / ".config" / "icdown"


def load_config() -> dict:
    path = config_dir() / "config.json"
    if not path.exists():
        return {}
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return {}


def mask_apple_id(value: str | None) -> str:
    if not value:
        return ""
    local, _, domain = value.partition("@")
    if not domain:
        return "***"
    if len(local) <= 2:
        shown = local[:1] + "*"
    else:
        shown = local[:2] + "***"
    return f"{shown}@{domain}"


def lock_held(path: Path) -> bool:
    if not path.exists():
        return False
    try:
        with path.open("r") as handle:
            fcntl.flock(handle.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
            fcntl.flock(handle.fileno(), fcntl.LOCK_UN)
        return False
    except BlockingIOError:
        return True
    except OSError:
        return False


def process_running() -> bool:
    try:
        out = subprocess.check_output(["pgrep", "-af", "icdown"], text=True, timeout=2)
    except (OSError, subprocess.CalledProcessError, subprocess.TimeoutExpired):
        return False
    for line in out.splitlines():
        if "icdown-tray" in line:
            continue
        if "icdown copiar" in line or "icdown verificar" in line or "icdown liberar" in line:
            return True
    return False


def tray_active() -> bool:
    try:
        completed = subprocess.run(
            ["systemctl", "--user", "is-active", "--quiet", "icdown-tray.service"],
            check=False,
            timeout=2,
        )
        return completed.returncode == 0
    except (OSError, subprocess.TimeoutExpired):
        return False


def session_present() -> bool:
    sesion = config_dir() / "sesion"
    if not sesion.is_dir():
        return False
    try:
        return any(sesion.iterdir())
    except OSError:
        return False


def read_registro(destino: Path) -> dict:
    db = destino / ".icdown" / "registro.db"
    empty = {
        "partes": 0,
        "verificadas": 0,
        "pendientes": 0,
        "errores": 0,
        "bytes_total": 0,
        "bytes_verificados": 0,
        "ultimo_listado": "",
        "db_ok": False,
    }
    if not db.exists():
        return empty
    try:
        con = sqlite3.connect(f"file:{db}?mode=ro", uri=True, timeout=2)
        con.execute("PRAGMA query_only=ON")
        partes, verificadas, pendientes, errores = con.execute(
            """
            SELECT
              COUNT(*),
              SUM(CASE WHEN estado = 'verificado' THEN 1 ELSE 0 END),
              SUM(CASE WHEN estado IS NULL OR estado IN ('pendiente','descargando','descargado') THEN 1 ELSE 0 END),
              SUM(CASE WHEN estado = 'error' THEN 1 ELSE 0 END)
            FROM partes
            """
        ).fetchone()
        bytes_total, bytes_verificados = con.execute(
            """
            SELECT
              COALESCE(SUM(tamano), 0),
              COALESCE(SUM(CASE WHEN estado = 'verificado' THEN tamano ELSE 0 END), 0)
            FROM partes
            """
        ).fetchone()
        marca = con.execute("SELECT valor FROM meta WHERE clave = 'marca_lista'").fetchone()
        ultimo = ""
        if marca and marca[0]:
            try:
                ts = int(marca[0])
                ultimo = datetime.fromtimestamp(ts).strftime("%Y-%m-%d %H:%M")
            except (TypeError, ValueError, OSError):
                ultimo = str(marca[0])[:32]
        con.close()
        return {
            "partes": int(partes or 0),
            "verificadas": int(verificadas or 0),
            "pendientes": int(pendientes or 0),
            "errores": int(errores or 0),
            "bytes_total": int(bytes_total or 0),
            "bytes_verificados": int(bytes_verificados or 0),
            "ultimo_listado": ultimo[:40],
            "db_ok": True,
        }
    except sqlite3.Error:
        return empty


def gb(n: int) -> float:
    return round(n / 1e9, 1)


def save_config(cfg: dict) -> None:
    path = config_dir() / "config.json"
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(".json.tmp")
    tmp.write_text(json.dumps(cfg, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    tmp.replace(path)


def clear_session_files() -> int:
    sesion = config_dir() / "sesion"
    removed = 0
    if not sesion.exists():
        return removed
    for path in sesion.iterdir():
        try:
            if path.is_file() or path.is_symlink():
                path.unlink(missing_ok=True)
                removed += 1
            elif path.is_dir():
                shutil.rmtree(path, ignore_errors=True)
                removed += 1
        except OSError:
            continue
    return removed


def stop_tray() -> None:
    try:
        subprocess.run(
            ["systemctl", "--user", "stop", "icdown-tray.service"],
            check=False,
            capture_output=True,
            timeout=5,
        )
    except (OSError, subprocess.TimeoutExpired):
        pass
    try:
        subprocess.run(["pkill", "-x", "icdown-tray"], check=False, capture_output=True, timeout=3)
    except (OSError, subprocess.TimeoutExpired):
        pass


def do_logout() -> dict:
    """Clear iCloud session + Apple ID. Keeps destino / photos / registro."""
    stop_tray()
    removed = clear_session_files()
    cfg = load_config()
    had_id = bool(cfg.get("apple_id"))
    cfg.pop("apple_id", None)
    if cfg or (config_dir() / "config.json").exists():
        save_config(cfg)
    log = config_dir() / "puente.log"
    try:
        if log.exists():
            log.write_text("", encoding="utf-8")
    except OSError:
        pass
    return {
        "ok": True,
        "action": "logout",
        "cleared_apple_id": had_id,
        "session_files_removed": removed,
        "logged_in": False,
        "phase": "need_login",
    }


def emit_status() -> dict:
    cfg = load_config()
    destino_raw = cfg.get("destino") or ""
    destino = Path(str(destino_raw)).expanduser() if destino_raw else None
    installed = shutil.which("icdown") is not None
    logged_in = bool(cfg.get("apple_id")) and session_present()
    running = False
    stats = {
        "partes": 0,
        "verificadas": 0,
        "pendientes": 0,
        "errores": 0,
        "bytes_total": 0,
        "bytes_verificados": 0,
        "ultimo_listado": "",
        "db_ok": False,
    }
    if destino and destino.exists():
        running = lock_held(destino / ".icdown" / "motor.lock") or process_running()
        stats = read_registro(destino)

    partes = stats["partes"]
    verificadas = stats["verificadas"]
    percent = 100 if partes == 0 and verificadas == 0 else (
        int(round(100.0 * verificadas / partes)) if partes else 0
    )
    percent = max(0, min(100, percent))

    if not installed:
        phase = "missing"
    elif not destino:
        phase = "no_dest"
    elif not logged_in:
        phase = "need_login"
    elif running:
        phase = "running"
    elif stats["pendientes"] > 0 or stats["errores"] > 0:
        phase = "behind"
    elif partes > 0 and verificadas >= partes:
        phase = "idle"
    elif not stats["db_ok"]:
        phase = "empty"
    else:
        phase = "behind"

    return {
        "ok": True,
        "installed": installed,
        "logged_in": logged_in,
        "destino": str(destino) if destino else "",
        "dominio": str(cfg.get("dominio") or "com")[:8],
        "apple_id_masked": mask_apple_id(cfg.get("apple_id")),
        "running": running,
        "tray_active": tray_active(),
        "phase": phase,
        "percent": percent,
        "partes": partes,
        "verificadas": verificadas,
        "pendientes": stats["pendientes"],
        "errores": stats["errores"],
        "bytes_total_gb": gb(stats["bytes_total"]),
        "bytes_verificados_gb": gb(stats["bytes_verificados"]),
        "ultimo_listado": stats["ultimo_listado"],
    }


def main() -> None:
    ap = argparse.ArgumentParser(add_help=False)
    ap.add_argument("--logout", action="store_true")
    args, _unknown = ap.parse_known_args()
    if args.logout:
        payload = do_logout()
    else:
        payload = emit_status()
    print(json.dumps(payload, ensure_ascii=False, separators=(",", ":")))


if __name__ == "__main__":
    main()
