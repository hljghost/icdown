# Third-party notices

This Omarchy plugin is a UI front-end for **icdown**:
https://github.com/hljghost/icdown

icdown is based on iCopy (MIT) by Z1Code and uses icloudpd / pyicloud_ipd (MIT).

The plugin runs `python3 status.py` locally and may launch:
- `icdown` / `icdown-tray`
- `omarchy-launch-floating-terminal-with-presentation`
- `systemctl --user` for the user unit `icdown-tray.service`
- `xdg-open` for the backup folder

No network calls are made by the plugin itself; iCloud traffic is handled by icdown.
