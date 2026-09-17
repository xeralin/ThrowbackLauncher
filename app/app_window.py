from pathlib import Path

from PySide6.QtCore import QChildEvent, QEvent, QObject, QStandardPaths, Qt, QUrl
from PySide6.QtGui import QCloseEvent, QColor, QDesktopServices, QKeyEvent, QWheelEvent
from PySide6.QtWebChannel import QWebChannel
from PySide6.QtWebEngineCore import QWebEnginePage, QWebEngineScript, QWebEngineSettings
from PySide6.QtWebEngineWidgets import QWebEngineView
from PySide6.QtWidgets import QFileDialog, QWidget

from core import log
from core.constants import IS_WINDOWS


def _platform_script() -> QWebEngineScript:
    script = QWebEngineScript()
    script.setName("throwback-platform")
    script.setInjectionPoint(QWebEngineScript.InjectionPoint.DocumentCreation)
    script.setWorldId(QWebEngineScript.ScriptWorldId.MainWorld)
    script.setRunsOnSubFrames(False)
    script.setSourceCode(f'window.__throwbackOS = "{"windows" if IS_WINDOWS else "linux"}";')
    return script


def _open_external(url: QUrl, origin: QUrl) -> bool:
    internal = (url.scheme(), url.host(), url.port()) == (
        origin.scheme(),
        origin.host(),
        origin.port(),
    )
    if url.scheme() in ("http", "https") and not internal:
        if not QDesktopServices.openUrl(url):
            log.fail("Could not open external URL", url.toString())
        return True
    return False


class _AppPage(QWebEnginePage):
    def __init__(self, origin: QUrl, parent: QObject | None = None) -> None:
        super().__init__(parent)
        self._origin = origin

    def acceptNavigationRequest(
        self, url: QUrl, nav_type: QWebEnginePage.NavigationType, is_main_frame: bool
    ) -> bool:
        if nav_type == QWebEnginePage.NavigationType.NavigationTypeLinkClicked and _open_external(
            url, self._origin
        ):
            return False
        return super().acceptNavigationRequest(url, nav_type, is_main_frame)


_ZOOM_KEYS = {Qt.Key.Key_Plus, Qt.Key.Key_Equal, Qt.Key.Key_Minus, Qt.Key.Key_0}


class BrowserView(QWebEngineView):
    def __init__(self, url: str, objects: dict[str, QObject]) -> None:
        super().__init__()

        self.setContextMenuPolicy(Qt.ContextMenuPolicy.NoContextMenu)
        self._origin = QUrl(url)
        self.setPage(_AppPage(self._origin, self))

        self.page().settings().setAttribute(
            QWebEngineSettings.WebAttribute.ScrollAnimatorEnabled, True
        )
        self.page().settings().setAttribute(
            QWebEngineSettings.WebAttribute.JavascriptCanAccessClipboard, True
        )
        self.page().scripts().insert(_platform_script())
        self.page().newWindowRequested.connect(self._open_external_window)
        self.page().profile().downloadRequested.connect(self._accept_download)

        self._channel = QWebChannel(self.page())
        for name, obj in objects.items():
            obj.setParent(self)
            self._channel.registerObject(name, obj)
        self.page().setWebChannel(self._channel)
        self.page().setBackgroundColor(QColor("#0d0d0f"))

        self.load(self._origin)

    def _open_external_window(self, request) -> None:
        _open_external(request.requestedUrl(), self._origin)

    def _accept_download(self, request) -> None:
        downloads = QStandardPaths.writableLocation(
            QStandardPaths.StandardLocation.DownloadLocation
        )
        picked, _ = QFileDialog.getSaveFileName(
            None, "Save file", str(Path(downloads) / request.downloadFileName())
        )
        if not picked:
            request.cancel()
            return
        target = Path(picked)
        request.setDownloadDirectory(str(target.parent))
        request.setDownloadFileName(target.name)
        request.accept()

    def closeEvent(self, event: QCloseEvent) -> None:
        QWidget.closeEvent(self, event)

    def childEvent(self, event: QChildEvent) -> None:
        if event.type() == QEvent.Type.ChildAdded and event.child().isWidgetType():
            event.child().installEventFilter(self)
        super().childEvent(event)

    def eventFilter(self, obj: QObject, event: QEvent) -> bool:
        if isinstance(event, QWheelEvent) and (
            event.modifiers() & Qt.KeyboardModifier.ControlModifier
        ):
            return True
        if (
            isinstance(event, QKeyEvent)
            and event.type() == QEvent.Type.KeyPress
            and event.modifiers() & Qt.KeyboardModifier.ControlModifier
            and event.key() in _ZOOM_KEYS
        ):
            return True
        return super().eventFilter(obj, event)
