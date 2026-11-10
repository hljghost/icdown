# icdown

Omarchy / Linux 上的 **iCloud 照片完整备份**工具，移植自 [Z1Code/iCopy](https://github.com/Z1Code/iCopy)（MIT）。

直连 iCloud 下载全部原图与视频，校验大小 / 容器结构 / SHA-256，支持断点续传；可选在确认后释放已备份内容占用的 iCloud 空间。

## 功能

- 下载照片、视频、Live Photo 视频轨与编辑版本，按 `年/年-月/` 归档
- 每个文件校验通过后才算完成；异常结构会二次下载比对
- 断点续传（`.part` + SQLite 登记），下载链接过期会自动分批刷新
- CLI：`login` / `copiar` / `estado` / `verificar` / `liberar` / `restaurar`
- 托盘 `icdown-tray`：立即备份、暂停、验证、改目标目录、登录提示、会话自启
- Omarchy：systemd 用户服务 + 可选 Hyprland 快捷键

## 要求

- Arch / Omarchy，Rust（`rustup`），Python 3
- Apple ID 开启双重认证
- 若启用「高级数据保护」，需在 iPhone 打开「在网页上访问 iCloud 数据」

## 安装

```bash
cd ~/Projects/icdown
./scripts/install.sh
```

脚本会：

1. `cargo build --release` 并安装到 `~/.local/bin/`
2. 在 `~/.config/icdown/venv` 安装 Python 桥接（icloudpd）
3. 写入默认配置与 `icdown-tray.service`

默认备份目录：`~/Pictures/iCloud`  
配置目录：`~/.config/icdown/`

## 使用

```bash
# 登录（密码不落盘；会话 cookie 保存在 ~/.config/icdown/sesion/）
icdown login

# 立即备份一轮
icdown copiar

# 看进度摘要
icdown estado

# 启动托盘（或开机自启）
systemctl --user enable --now icdown-tray.service
```

释放 iCloud 空间（**只会删已备份且再校验过的内容**，至少保留最近 30 天，需在终端输入 `BORRAR` 确认）：

```bash
icdown liberar --dias 90
icdown restaurar --ultima   # 从「最近删除」恢复上一批
```

### Omarchy 快捷键（可选）

在 `~/.config/hypr/bindings.lua` 增加：

```lua
o.bind("SUPER + SHIFT + I", "icdown 备份", "icdown copiar")
o.bind("SUPER + ALT + I", "icdown 托盘", "icdown-tray")
```

或登录后自动起托盘：在 `~/.config/hypr/autostart.lua`：

```lua
o.launch_on_start("icdown-tray")
```

（若已 `systemctl --user enable icdown-tray.service`，通常不必再写 autostart。）

## Omarchy 插件

状态栏小组件 `ghost.icdown`：显示备份进度，面板里可「立即备份 / 打开目录 / 登录 / 启停托盘」。

```bash
./scripts/install-plugin.sh
# 或手动：
# cp -a omarchy-plugin ~/.config/omarchy/plugins/ghost.icdown
# omarchy plugin enable ghost.icdown --section right
```

源码在仓库的 `omarchy-plugin/`（因完整仓库含 Rust 工程，不能直接用 `omarchy plugin add <repo-url>`；请用上面的安装脚本）。改插件文件后会热重载；也可 `omarchy-shell shell rescanPlugins`。

## 开发

```bash
# 桥接依赖
python3 -m venv ~/.config/icdown/venv
~/.config/icdown/venv/bin/pip install -r puente/requirements.txt

export PATH="$HOME/.cargo/bin:$PATH"
unset CARGO_TARGET_DIR
cargo test --manifest-path icdown/Cargo.toml
cargo run --manifest-path icdown/Cargo.toml -- estado
```

环境变量：

| 变量 | 作用 |
| --- | --- |
| `ICDOWN_DIR` | 配置目录（默认 `~/.config/icdown`） |
| `ICDOWN_PUENTE` | 桥接可执行文件或 `.py` 路径 |
| `ICDOWN_PYTHON` | 运行 `.py` 桥接时的解释器 |
| `ICDOWN_IDIOMA` | `es` / `en` |

## 当前限制

- 托盘尚未内置「释放空间」图形向导（请用 `icdown liberar`）
- 登录仅 CLI（`icdown login`）
- 暂无应用内自动更新
- 依赖非官方 iCloud Web API（与 icloudpd 相同）；Apple 改动可能导致暂时失效
- **与 Apple 无关**

## 致谢与许可

基于 [Z1Code/iCopy](https://github.com/Z1Code/iCopy) 与 [icloudpd](https://github.com/icloud-photos-downloader/icloud_photos_downloader)。  
[MIT](LICENSE)
