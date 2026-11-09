$ErrorActionPreference = "Stop"
$aqui = $PSScriptRoot
& "$aqui\.venv\Scripts\python.exe" -m PyInstaller --noconfirm --onefile --name puente-icloud `
    --collect-submodules pyicloud_ipd --collect-submodules foundation --collect-submodules icloudpd `
    --collect-data certifi --collect-data tzdata `
    --distpath "$aqui\dist" --workpath "$aqui\build" --specpath "$aqui\build" `
    "$aqui\puente_icloud.py"
if ($LASTEXITCODE -ne 0) { throw "PyInstaller falló" }
