from pathlib import Path

from PySide6.QtCore import QFile, QIODevice, QObject
from PySide6.QtWebEngineCore import (
    QWebEngineUrlRequestJob,
    QWebEngineUrlScheme,
    QWebEngineUrlSchemeHandler,
)

SCHEME = b"app"
APP_ORIGIN = "app://launcher"

_MIME_TYPES = {
    ".html": "text/html",
    ".js": "text/javascript",
    ".css": "text/css",
    ".txt": "text/plain",
    ".webp": "image/webp",
    ".woff2": "font/woff2",
}


def register_app_scheme() -> None:
    scheme = QWebEngineUrlScheme(SCHEME)
    scheme.setSyntax(QWebEngineUrlScheme.Syntax.Host)
    scheme.setFlags(
        QWebEngineUrlScheme.Flag.SecureScheme
        | QWebEngineUrlScheme.Flag.CorsEnabled
        | QWebEngineUrlScheme.Flag.FetchApiAllowed
    )
    QWebEngineUrlScheme.registerScheme(scheme)


class AppSchemeHandler(QWebEngineUrlSchemeHandler):
    def __init__(self, root: Path, parent: QObject | None = None) -> None:
        super().__init__(parent)
        self._root = root.resolve()

    def requestStarted(self, job: QWebEngineUrlRequestJob) -> None:
        path = job.requestUrl().path()
        target = (self._root / path.lstrip("/")).resolve()
        if not target.is_relative_to(self._root):
            job.fail(QWebEngineUrlRequestJob.Error.RequestDenied)
            return
        if target.is_dir():
            target = target / "index.html"
        if not target.is_file():
            job.fail(QWebEngineUrlRequestJob.Error.UrlNotFound)
            return
        mime = _MIME_TYPES.get(target.suffix, "application/octet-stream")
        file = QFile(str(target), job)
        if not file.open(QIODevice.OpenModeFlag.ReadOnly):
            job.fail(QWebEngineUrlRequestJob.Error.UrlNotFound)
            return
        job.reply(mime.encode(), file)
