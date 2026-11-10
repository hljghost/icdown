import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Commons as Commons
import qs.Ui

BarWidget {
  id: root
  moduleName: "io.github.hljghost.icdown"

  Commons.Translations {
    id: strings
    directory: Qt.resolvedUrl("i18n")
  }

  property int refreshIntervalSec: setting("refreshIntervalSec", 15)

  property bool installed: false
  property bool loggedIn: false
  property bool running: false
  property bool trayActive: false
  property string phase: "empty"
  property int percent: 0
  property int partes: 0
  property int verificadas: 0
  property int pendientes: 0
  property int errores: 0
  property real bytesTotalGb: 0
  property real bytesVerificadosGb: 0
  property string destino: ""
  property string dominio: "com"
  property string appleIdMasked: ""
  property string ultimoListado: ""

  readonly property string statusScript: Qt.resolvedUrl("status.py").toString().replace(/^file:\/\//, "")

  readonly property bool opened: panelLoader.item ? panelLoader.item.opened === true : false
  readonly property bool popoutSwitchClosing: panelLoader.item ? panelLoader.item.popoutSwitchClosing === true : false

  function open() {
    if (panelLoader.item)
      panelLoader.item.open()
  }

  function close() {
    if (panelLoader.item)
      panelLoader.item.close()
  }

  function toggle() {
    if (panelLoader.item)
      panelLoader.item.toggle()
  }

  function closeForPopoutSwitch() {
    if (panelLoader.item)
      panelLoader.item.closeForPopoutSwitch()
  }

  function injectPanel() {
    if (!panelLoader.item)
      return
    panelLoader.item.bar = root.bar
    panelLoader.item.anchorItem = button
    panelLoader.item.hostWidget = root
  }

  function refresh() {
    if (!statusProc.running)
      statusProc.running = true
  }

  function parseStatus(raw) {
    try {
      var data = JSON.parse(raw)
      root.installed = !!data.installed
      root.loggedIn = !!data.logged_in
      root.running = !!data.running
      root.trayActive = !!data.tray_active
      root.phase = String(data.phase || "empty").slice(0, 32)
      root.percent = Math.max(0, Math.min(100, Math.round(Number(data.percent) || 0)))
      root.partes = Math.max(0, Math.round(Number(data.partes) || 0))
      root.verificadas = Math.max(0, Math.round(Number(data.verificadas) || 0))
      root.pendientes = Math.max(0, Math.round(Number(data.pendientes) || 0))
      root.errores = Math.max(0, Math.round(Number(data.errores) || 0))
      root.bytesTotalGb = Number(data.bytes_total_gb) || 0
      root.bytesVerificadosGb = Number(data.bytes_verificados_gb) || 0
      root.destino = String(data.destino || "").slice(0, 512)
      root.dominio = String(data.dominio || "com").slice(0, 8)
      root.appleIdMasked = String(data.apple_id_masked || "").slice(0, 128)
      root.ultimoListado = String(data.ultimo_listado || "").slice(0, 40)
    } catch (e) {
      console.warn("icdown: failed to parse status", e)
    }
  }

  function phaseLabel() {
    if (root.phase === "missing")
      return strings.t("icdown.notInstalled")
    if (root.phase === "no_dest")
      return strings.t("icdown.noDest")
    if (root.phase === "need_login")
      return strings.t("icdown.needLogin")
    if (root.phase === "running")
      return strings.t("icdown.running")
    if (root.phase === "idle")
      return strings.t("icdown.idle")
    if (root.phase === "behind")
      return strings.t("icdown.catchingUp")
    return strings.t("icdown.subtitle")
  }

  function barText() {
    if (root.phase === "running")
      return "󰄧 " + Commons.I18n.formatNumber(root.percent) + "%"
    if (root.phase === "idle")
      return "󰄧 ✓"
    if (root.phase === "behind")
      return "󰄧 " + Commons.I18n.formatNumber(root.percent) + "%"
    if (root.phase === "need_login" || root.phase === "no_dest" || root.phase === "missing")
      return "󰄧 !"
    return "󰄧"
  }

  function runShell(cmd) {
    if (root.bar)
      root.bar.run(cmd)
  }

  function copyNow() {
    runShell("omarchy-launch-floating-terminal-with-presentation icdown copiar")
    root.close()
  }

  function openFolder() {
    if (!root.destino)
      return
    runShell("xdg-open " + JSON.stringify(root.destino))
    root.close()
  }

  function loginTerminal() {
    runShell("omarchy-launch-floating-terminal-with-presentation icdown login")
    root.close()
  }

  function toggleTray() {
    if (root.trayActive)
      runShell("systemctl --user stop icdown-tray.service")
    else
      runShell("systemctl --user start icdown-tray.service")
    Qt.callLater(root.refresh)
  }

  function logoutAccount() {
    if (!logoutProc.running)
      logoutProc.running = true
  }

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  onBarChanged: injectPanel()
  Component.onCompleted: root.refresh()

  Loader {
    id: panelLoader
    active: true
    source: Qt.resolvedUrl("Panel.qml")
    visible: false
    onLoaded: {
      root.injectPanel()
      Qt.callLater(root.injectPanel)
    }
  }

  WidgetButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: root.barText()
    tooltipText: root.phaseLabel() + (root.partes > 0 ? (" · " + root.percent + "%") : "")
    active: root.phase === "running" || root.phase === "behind" || root.phase === "need_login"
    useActiveColor: true
    activeColor: root.phase === "need_login" || root.phase === "missing" ? Commons.Color.urgent : Commons.Color.accent
    horizontalMargin: 8.5
    onPressed: function (b) {
      if (b === Qt.RightButton)
        root.copyNow()
      else
        root.toggle()
    }
  }

  Process {
    id: statusProc
    command: ["python3", root.statusScript]
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.parseStatus(text)
    }
  }

  Process {
    id: logoutProc
    command: ["python3", root.statusScript, "--logout"]
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: {
        root.parseStatus(text)
        root.refresh()
      }
    }
  }

  Timer {
    interval: Math.max(5, root.refreshIntervalSec) * 1000
    running: true
    repeat: true
    triggeredOnStart: true
    onTriggered: root.refresh()
  }

  Timer {
    interval: 2000
    running: root.opened || root.running
    repeat: true
    triggeredOnStart: false
    onTriggered: root.refresh()
  }
}
