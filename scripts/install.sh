#!/usr/bin/env bash
# Install icdown for Omarchy / Arch (user-local).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
CFG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/icdown"
DATA_DEST="${ICDOWN_DEST:-$HOME/Pictures/iCloud}"
# icloudpd pins older Python; prefer 3.11/3.12 if present.
if [[ -z "${ICDOWN_PYTHON:-}" ]]; then
  for c in python3.11 python3.12 python3.13 python3; do
    if command -v "$c" >/dev/null 2>&1; then
      ICDOWN_PYTHON="$c"
      break
    fi
  done
fi
PY_VER="${ICDOWN_PYTHON:?需要 Python 3}"

export PATH="${HOME}/.cargo/bin:${PATH}"
# Cursor/agent sandboxes may redirect cargo targets; install always uses the project tree.
unset CARGO_TARGET_DIR

need() { command -v "$1" >/dev/null 2>&1 || { echo "缺少命令: $1" >&2; exit 1; }; }
need cargo
need "$PY_VER"
echo "==> 使用解释器: $PY_VER ($("$PY_VER" --version 2>&1))"

mkdir -p "$BIN_DIR" "$CFG_DIR" "$DATA_DEST"

echo "==> 编译 icdown (release)"
cargo build --release --manifest-path "$ROOT/icdown/Cargo.toml"

echo "==> 安装二进制到 $BIN_DIR"
install -m 755 "$ROOT/icdown/target/release/icdown" "$BIN_DIR/icdown"
install -m 755 "$ROOT/icdown/target/release/icdown-tray" "$BIN_DIR/icdown-tray"

echo "==> 准备 Python 桥接 (pyicloud / icloudpd)"
VENV="$CFG_DIR/venv"
if [[ ! -x "$VENV/bin/python" ]]; then
  "$PY_VER" -m venv "$VENV"
fi
# shellcheck disable=SC1091
source "$VENV/bin/activate"
pip install -U pip wheel
pip install -r "$ROOT/puente/requirements.txt"
# Keep a stable entry point that uses the venv interpreter.
cat >"$BIN_DIR/puente-icloud" <<EOF
#!/usr/bin/env bash
exec "$VENV/bin/python" -I "$ROOT/puente/puente_icloud.py" "\$@"
EOF
chmod 755 "$BIN_DIR/puente-icloud"

echo "==> 默认配置"
CONFIG="$CFG_DIR/config.json"
if [[ ! -f "$CONFIG" ]]; then
  cat >"$CONFIG" <<EOF
{
  "destino": "$DATA_DEST",
  "mantener_dias": 30,
  "concurrencia": 3,
  "intervalo_horas": 4,
  "dominio": "com",
  "actualizar_auto": false
}
EOF
  echo "已写入 $CONFIG（目标目录: $DATA_DEST）"
else
  echo "保留已有配置 $CONFIG"
fi

UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
mkdir -p "$UNIT_DIR"
cat >"$UNIT_DIR/icdown-tray.service" <<EOF
[Unit]
Description=icdown tray (iCloud Photos backup)
After=graphical-session.target
PartOf=graphical-session.target

[Service]
Type=simple
ExecStart=$BIN_DIR/icdown-tray
Restart=on-failure
RestartSec=5
Environment=PATH=$BIN_DIR:/usr/bin
Environment=ICDOWN_PUENTE=$BIN_DIR/puente-icloud
Environment=ICDOWN_PYTHON=$VENV/bin/python

[Install]
WantedBy=graphical-session.target
EOF

systemctl --user daemon-reload

cat <<EOF

安装完成。

下一步：
  1) 登录 iCloud（会提示密码与 2FA，密码不会保存）：
       icdown login
  2) 可选：立刻跑一轮备份：
       icdown copiar
  3) 启动托盘（或开机自启）：
       systemctl --user enable --now icdown-tray.service
     或在托盘菜单里打开「Iniciar con la sesión」

照片默认保存到: $DATA_DEST
配置目录:       $CFG_DIR

若启用了 iCloud「高级数据保护」，请在 iPhone 打开：
  设置 → [你的名字] → iCloud →「在网页上访问 iCloud 数据」
EOF
