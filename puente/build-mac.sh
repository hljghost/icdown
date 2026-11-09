#!/bin/bash
# Empaqueta el puente para macOS (Apple Silicon) con PyInstaller → puente/dist/puente-icloud, firmado
# con hardened runtime (obligatorio para notarizar) y comprobado al final.
#
# - Con ICOPY_MAC_IDENTIDAD (una identidad «Developer ID Application» del llavero): PyInstaller firma
#   también las bibliotecas internas (Python.framework, .so) con nuestra identidad, así comparten
#   Team ID y macOS las carga sin excepciones.
# - Sin ella (desarrollo): firma ad-hoc y la excepción disable-library-validation; sin ella macOS no
#   carga Python.framework («different Team IDs»). Comprobado en el Mac mini el 2026-11-08.
#
# Uso: puente/build-mac.sh   (con puente/.venv creado: python3 -m venv puente/.venv && pip install -r …)
set -euo pipefail
aqui="$(cd "$(dirname "$0")" && pwd)"
raiz="$(dirname "$aqui")"

args=(--noconfirm --onefile --name puente-icloud --target-arch arm64
      --collect-submodules pyicloud_ipd --collect-submodules foundation --collect-submodules icloudpd
      --collect-data certifi --collect-data tzdata
      --distpath "$aqui/dist" --workpath "$aqui/build" --specpath "$aqui/build")
if [ -n "${ICOPY_MAC_IDENTIDAD:-}" ]; then
    args+=(--codesign-identity "$ICOPY_MAC_IDENTIDAD")
    permisos="$raiz/instalador/macos/puente.entitlements"
    identidad="$ICOPY_MAC_IDENTIDAD"
    sello="--timestamp"
else
    permisos="$raiz/instalador/macos/puente-desarrollo.entitlements"
    identidad="-"
    sello="--timestamp=none"
fi

"$aqui/.venv/bin/python" -m PyInstaller "${args[@]}" "$aqui/puente_icloud.py"
bin="$aqui/dist/puente-icloud"
codesign --force --options runtime "$sello" --entitlements "$permisos" --sign "$identidad" "$bin"

# Prueba de humo, sin red: tiene que cargar Python y responder a un comando desconocido.
tmp="$(mktemp -d)"
respuesta="$(printf '{"cmd":"prueba"}\n' | "$bin" --cookies "$tmp/c" --apple-id yo@ejemplo.com 2>"$tmp/err" || true)"
case "$respuesta" in
    *"comando desconocido"*)
        rm -rf "$tmp"
        echo "Puente listo: $bin" ;;
    *)
        echo "El puente firmado no arranca:" >&2
        tail -5 "$tmp/err" >&2
        rm -rf "$tmp"
        exit 1 ;;
esac
