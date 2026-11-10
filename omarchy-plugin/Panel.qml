import QtQuick
import Quickshell
import qs.Commons
import qs.Commons as Commons
import qs.Ui

Panel {
  id: root
  moduleName: "io.github.hljghost.icdown"
  manageIpc: false

  Commons.Translations {
    id: strings
    directory: Qt.resolvedUrl("i18n")
  }

  property var anchorItem: null
  property var hostWidget: null

  readonly property var hw: hostWidget

  function open() {
    root.controller.show()
  }

  function close() {
    root.controller.hide()
  }

  function toggle() {
    if (root.opened)
      root.close()
    else
      root.open()
  }

  function switchPanel(direction) {
    if (root.bar && typeof root.bar.switchPanelFrom === "function")
      return root.bar.switchPanelFrom(root.hostWidget || root, direction)
    return false
  }

  onOpenedChanged: {
    if (opened && hw && typeof hw.refresh === "function")
      hw.refresh()
  }

  KeyboardPanel {
    id: panel
    anchorItem: root.anchorItem
    owner: root.hostWidget || root
    bar: root.bar
    open: root.opened
    centerOnBar: false
    focusTarget: keyCatcher
    contentWidth: panel.fittedContentWidth(Style.space(380))
    contentHeight: panel.fittedContentHeight(contentColumn.implicitHeight)

    PanelKeyCatcher {
      id: keyCatcher
      anchors.fill: parent
      onCloseRequested: root.close()
      onTabRequested: function (direction) {
        root.switchPanel(direction)
      }
      onTextKey: function (t) {
        if (!hw)
          return
        if (t === "r" || t === "R")
          hw.refresh()
        else if (t === "c" || t === "C")
          hw.copyNow()
        else if (t === "o" || t === "O")
          hw.openFolder()
      }

      Column {
        id: contentColumn
        width: parent.width
        spacing: Style.space(12)

        PanelHero {
          title: hw ? strings.t("icdown.title") : "icdown"
          meta: hw ? hw.phaseLabel() : ""
          detail: hw && hw.appleIdMasked.length > 0
            ? strings.t("icdown.account").arg(hw.appleIdMasked)
            : (hw ? strings.t("icdown.subtitle") : "")
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
          visible: hw !== null

          Row {
            width: parent.width
            Text {
              textFormat: Text.PlainText
              text: hw ? strings.t("icdown.progress") : ""
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
              text: hw ? (Commons.I18n.formatNumber(hw.percent) + "%") : ""
              color: hw && hw.phase === "running" ? Commons.Color.accent : Commons.Color.foreground
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
              color: hw && hw.errores > 0 ? Commons.Color.urgent : Commons.Color.accent
              width: parent.width * Math.min(1.0, Math.max(0.0, (hw ? hw.percent : 0) / 100.0))

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
            text: hw ? strings.t("icdown.verified").arg(Commons.I18n.formatNumber(hw.verificadas)).arg(Commons.I18n.formatNumber(hw.partes)) : ""
            color: Commons.Color.foreground
            font.family: Style.font.family
            font.pixelSize: Style.font.caption
            wrapMode: Text.Wrap
          }

          Text {
            width: parent.width
            visible: hw && hw.bytesTotalGb > 0
            textFormat: Text.PlainText
            text: hw ? strings.t("icdown.bytes").arg(Commons.I18n.formatNumber(hw.bytesVerificadosGb, 1)).arg(Commons.I18n.formatNumber(hw.bytesTotalGb, 1)) : ""
            color: Qt.darker(Commons.Color.foreground, 1.4)
            font.family: Style.font.family
            font.pixelSize: Style.font.caption
          }

          Text {
            width: parent.width
            visible: hw && hw.pendientes > 0
            textFormat: Text.PlainText
            text: hw ? strings.t("icdown.pending").arg(Commons.I18n.formatNumber(hw.pendientes)) : ""
            color: Commons.Color.accent
            font.family: Style.font.family
            font.pixelSize: Style.font.caption
          }

          Text {
            width: parent.width
            visible: hw && hw.errores > 0
            textFormat: Text.PlainText
            text: hw ? strings.t("icdown.errors").arg(Commons.I18n.formatNumber(hw.errores)) : ""
            color: Commons.Color.urgent
            font.family: Style.font.family
            font.pixelSize: Style.font.caption
          }

          Text {
            width: parent.width
            visible: hw && hw.ultimoListado.length > 0
            textFormat: Text.PlainText
            text: hw ? strings.t("icdown.lastList").arg(hw.ultimoListado) : ""
            color: Qt.darker(Commons.Color.foreground, 1.6)
            font.family: Style.font.family
            font.pixelSize: Style.font.caption
          }

          Text {
            width: parent.width
            visible: hw && hw.destino.length > 0
            textFormat: Text.PlainText
            text: hw ? strings.t("icdown.destino").arg(hw.destino) : ""
            color: Qt.darker(Commons.Color.foreground, 1.6)
            font.family: Style.font.family
            font.pixelSize: Style.font.caption
            elide: Text.ElideMiddle
          }

          Text {
            width: parent.width
            visible: hw !== null
            textFormat: Text.PlainText
            text: hw ? strings.t("icdown.domain").arg(hw.dominio) : ""
            color: Qt.darker(Commons.Color.foreground, 1.6)
            font.family: Style.font.family
            font.pixelSize: Style.font.caption
          }
        }

        PanelSeparator {}

        Column {
          width: parent.width
          spacing: Style.space(8)
          visible: hw !== null

          Button {
            width: parent.width
            text: hw ? strings.t("icdown.copyNow") : ""
            iconText: "󰑓"
            bordered: true
            enabled: hw && hw.installed && hw.loggedIn && hw.destino.length > 0 && !hw.running
            onClicked: hw.copyNow()
          }

          Button {
            width: parent.width
            text: hw ? strings.t("icdown.openFolder") : ""
            iconText: "󰉋"
            bordered: true
            enabled: hw && hw.destino.length > 0
            onClicked: hw.openFolder()
          }

          Button {
            width: parent.width
            text: hw ? strings.t("icdown.login") : ""
            iconText: "󰌾"
            bordered: true
            enabled: hw && hw.installed && !hw.loggedIn
            onClicked: hw.loginTerminal()
          }

          Button {
            width: parent.width
            text: hw ? strings.t("icdown.logout") : ""
            iconText: "󰍃"
            bordered: true
            enabled: hw && hw.installed && hw.loggedIn && !hw.running
            onClicked: hw.logoutAccount()
          }

          Button {
            width: parent.width
            text: hw ? (hw.trayActive ? strings.t("icdown.stopTray") : strings.t("icdown.startTray")) : ""
            iconText: "󰍜"
            bordered: true
            enabled: hw && hw.installed
            onClicked: hw.toggleTray()
          }

          Text {
            width: parent.width
            textFormat: Text.PlainText
            text: hw ? strings.t("icdown.refreshHint") : ""
            color: Qt.darker(Commons.Color.foreground, 1.7)
            font.family: Style.font.family
            font.pixelSize: Style.font.caption
          }
        }
      }
    }
  }
}
