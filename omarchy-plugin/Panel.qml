import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Commons as Commons
import qs.Ui

Panel {
  id: root
  moduleName: "ghost.icdown"
  ipcTarget: "ghost.icdown"

  Commons.Translations {
    id: strings
    directory: Qt.resolvedUrl("i18n")
  }

  property int refreshIntervalSec: 15

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

  onOpenedChanged: {
    if (opened)
      root.refresh()
  }

  Component.onCompleted: root.refresh()

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

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

  KeyboardPanel {
    id: panel
    anchorItem: button
    owner: root
    bar: root.bar
    open: root.opened
    centerOnBar: false
    focusTarget: panel.loadedContent
    contentWidth: panel.fittedContentWidth(Style.space(380))
    contentHeight: panel.fittedContentHeight(panel.loadedContentHeight)

    content: PanelKeyCatcher {
      id: keyCatcher
      anchors.fill: parent
      implicitHeight: contentColumn.implicitHeight
      onCloseRequested: root.close()
      onTabRequested: function (direction) {
        root.switchPanel(direction)
      }
      onTextKey: function (t) {
        if (t === "r" || t === "R")
          root.refresh()
        else if (t === "c" || t === "C")
          root.copyNow()
        else if (t === "o" || t === "O")
          root.openFolder()
      }

      Column {
        id: contentColumn
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        spacing: Style.space(12)

        PanelHero {
          title: strings.t("icdown.title")
          meta: root.phaseLabel()
          detail: root.appleIdMasked.length > 0
            ? strings.t("icdown.account").arg(root.appleIdMasked)
            : strings.t("icdown.subtitle")
          iconComponent: Component {
            Text {
              textFormat: Text.PlainText
              text: "󰄧"
              color: Commons.Color.foreground
              font.family: Style.font.family
              font.pixelSize: Style.font.display
            }
          }
        }

        PanelSeparator {}

        Column {
          width: parent.width
          spacing: Style.space(6)

          Row {
            width: parent.width
            Text {
              textFormat: Text.PlainText
              text: strings.t("icdown.progress")
              color: Qt.darker(Commons.Color.foreground, 1.4)
              font.family: Style.font.family
              font.pixelSize: Style.font.caption
              font.bold: true
              font.letterSpacing: 1.0
            }
            Item {
              width: Math.max(0, parent.width - parent.children[0].implicitWidth - parent.children[2].implicitWidth)
              height: 1
            }
            Text {
              textFormat: Text.PlainText
              text: Commons.I18n.formatNumber(root.percent) + "%"
              color: root.phase === "running" ? Commons.Color.accent : Commons.Color.foreground
              font.family: Style.font.family
              font.pixelSize: Style.font.body
              font.bold: true
            }
          }

          Rectangle {
            width: parent.width
            height: Style.space(8)
            radius: Style.space(4)
            color: Qt.rgba(Commons.Color.foreground.r, Commons.Color.foreground.g, Commons.Color.foreground.b, 0.12)

            Rectangle {
              height: parent.height
              radius: parent.radius
              color: root.errores > 0 ? Commons.Color.urgent : Commons.Color.accent
              width: parent.width * Math.min(1.0, Math.max(0.0, root.percent / 100.0))

              Behavior on width {
                NumberAnimation {
                  duration: Style.duration(160)
                  easing.type: Easing.OutCubic
                }
              }
            }
          }

          Text {
            width: parent.width
            textFormat: Text.PlainText
            text: strings.t("icdown.verified").arg(Commons.I18n.formatNumber(root.verificadas)).arg(Commons.I18n.formatNumber(root.partes))
            color: Commons.Color.foreground
            font.family: Style.font.family
            font.pixelSize: Style.font.caption
            wrapMode: Text.Wrap
          }

          Text {
            width: parent.width
            visible: root.bytesTotalGb > 0
            textFormat: Text.PlainText
            text: strings.t("icdown.bytes").arg(Commons.I18n.formatNumber(root.bytesVerificadosGb, 1)).arg(Commons.I18n.formatNumber(root.bytesTotalGb, 1))
            color: Qt.darker(Commons.Color.foreground, 1.4)
            font.family: Style.font.family
            font.pixelSize: Style.font.caption
          }

          Text {
            width: parent.width
            visible: root.pendientes > 0
            textFormat: Text.PlainText
            text: strings.t("icdown.pending").arg(Commons.I18n.formatNumber(root.pendientes))
            color: Commons.Color.accent
            font.family: Style.font.family
            font.pixelSize: Style.font.caption
          }

          Text {
            width: parent.width
            visible: root.errores > 0
            textFormat: Text.PlainText
            text: strings.t("icdown.errors").arg(Commons.I18n.formatNumber(root.errores))
            color: Commons.Color.urgent
            font.family: Style.font.family
            font.pixelSize: Style.font.caption
          }

          Text {
            width: parent.width
            visible: root.ultimoListado.length > 0
            textFormat: Text.PlainText
            text: strings.t("icdown.lastList").arg(root.ultimoListado)
            color: Qt.darker(Commons.Color.foreground, 1.6)
            font.family: Style.font.family
            font.pixelSize: Style.font.caption
          }

          Text {
            width: parent.width
            visible: root.destino.length > 0
            textFormat: Text.PlainText
            text: strings.t("icdown.destino").arg(root.destino)
            color: Qt.darker(Commons.Color.foreground, 1.6)
            font.family: Style.font.family
            font.pixelSize: Style.font.caption
            elide: Text.ElideMiddle
          }

          Text {
            width: parent.width
            textFormat: Text.PlainText
            text: strings.t("icdown.domain").arg(root.dominio)
            color: Qt.darker(Commons.Color.foreground, 1.6)
            font.family: Style.font.family
            font.pixelSize: Style.font.caption
          }
        }

        PanelSeparator {}

        Column {
          width: parent.width
          spacing: Style.space(8)

          Button {
            width: parent.width
            text: strings.t("icdown.copyNow")
            iconText: "󰑓"
            bordered: true
            enabled: root.installed && root.loggedIn && root.destino.length > 0 && !root.running
            onClicked: root.copyNow()
          }

          Button {
            width: parent.width
            text: strings.t("icdown.openFolder")
            iconText: "󰉋"
            bordered: true
            enabled: root.destino.length > 0
            onClicked: root.openFolder()
          }

          Button {
            width: parent.width
            text: strings.t("icdown.login")
            iconText: "󰌾"
            bordered: true
            enabled: root.installed && !root.loggedIn
            onClicked: root.loginTerminal()
          }

          Button {
            width: parent.width
            text: strings.t("icdown.logout")
            iconText: "󰍃"
            bordered: true
            enabled: root.installed && root.loggedIn && !root.running
            onClicked: root.logoutAccount()
          }

          Button {
            width: parent.width
            text: root.trayActive ? strings.t("icdown.stopTray") : strings.t("icdown.startTray")
            iconText: "󰍜"
            bordered: true
            enabled: root.installed
            onClicked: root.toggleTray()
          }

          Text {
            width: parent.width
            textFormat: Text.PlainText
            text: strings.t("icdown.refreshHint")
            color: Qt.darker(Commons.Color.foreground, 1.7)
            font.family: Style.font.family
            font.pixelSize: Style.font.caption
          }
        }
      }
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
