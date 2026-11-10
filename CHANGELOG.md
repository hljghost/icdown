# Changelog

## [Sin publicar]

### Added
- Marketplace plugin https://github.com/hljghost/omarchy-icdown-plugin (`io.github.hljghost.icdown`)
- Omarchy bar plugin (legacy id `ghost.icdown`, migrated) (`omarchy-plugin/` + `scripts/install-plugin.sh`): progress, copy now, open folder, login, tray toggle

## [0.2.0] - 2026-11-09

### Added
- Linux / Omarchy port of [Z1Code/iCopy](https://github.com/Z1Code/iCopy) as **icdown**
- CLI: `destino`, `login`, `diagnostico`, `copiar`, `estado`, `verificar`, `liberar`, `restaurar`
- Tray app `icdown-tray` (StatusNotifierItem): copy / pause / verify / open folder / autostart
- `scripts/install.sh` for user-local install + Python bridge + systemd unit
- Path handling via `layout::en_disco` (registry keeps `\`, disk uses native separators)

### Changed
- Config dir: `~/.config/icdown` (`ICDOWN_DIR`)
- Data dir under destination: `.icdown`
- Sleep inhibit via `systemd-inhibit`; notifications via `notify-rust`
- Autostart via `systemctl --user` (`icdown-tray.service`)

### Known gaps vs Windows iCopy
- No GDI progress panel / liberar wizard in the tray (use CLI `icdown liberar`)
- Login is CLI-only
- In-app auto-update stubbed
