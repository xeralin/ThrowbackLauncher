import contextlib
import ctypes
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import urllib.error
import urllib.request
import zipfile
from pathlib import Path

from PySide6.QtCore import QLockFile, QRectF, QSize, Qt, QThread, QUrl, Signal
from PySide6.QtGui import (
    QBrush,
    QColor,
    QDesktopServices,
    QFont,
    QFontDatabase,
    QGradient,
    QIcon,
    QLinearGradient,
    QPainter,
    QPainterPath,
    QPen,
    QPixmap,
)
from PySide6.QtWidgets import (
    QApplication,
    QFileDialog,
    QFrame,
    QGraphicsOpacityEffect,
    QHBoxLayout,
    QLabel,
    QPushButton,
    QSizePolicy,
    QVBoxLayout,
    QWidget,
)

from core.constants import DEFAULT_ACCENT, HTTP_TIMEOUT, UPDATE_API_URL
from core.github import SSL_CONTEXT, RateLimited, fetch_to, rate_limit_error
from layout import (
    APP_NAME,
    APP_SUBDIR,
    ATTEMPTED_FILE,
    DIR_NAME,
    EXE_NAME,
    PENDING_FILE,
    PENDING_SUBDIR,
    PREVIOUS_SUBDIR,
    RUNTIME_ASSET,
    UNINSTALL_KEY,
    start_menu_shortcut,
    user_data_base,
)

DEFAULT_DIR = user_data_base() / DIR_NAME

CARD_WIDTH = 440
CARD_PADDING = 24
BUTTON_GAP = 8
CLOSE_WIDTH = 84
CONTINUE_WIDTH = 104
MESSAGE_PADDING = 3
ERROR_TEXT_MAX = 160
DOWNLOAD_PCT = 90
INACTIVE_OPACITY = 0.4

TEXT = "#e8e0d5"
MUTED = "#7a7890"
ACCENT = DEFAULT_ACCENT
ACCENT_HOVER = "#a01020"
ACCENT_DIM = "#560913"
CARD_BG = "#13131a"
FIELD_BG = "#1a1a24"
BORDER = "#2a2a38"


def app_exe(root: Path) -> Path:
    return root / APP_SUBDIR / EXE_NAME


def _load_fonts() -> None:
    fonts = Path(__file__).resolve().parent / "assets" / "fonts"
    for ttf in fonts.glob("*.ttf"):
        QFontDatabase.addApplicationFont(str(ttf))


STYLE = f"""
#card {{
    background: {CARD_BG};
    border: 1px solid {BORDER};
    border-radius: 8px;
    font-family: "Barlow";
}}
#title {{ color: {TEXT}; font-family: "Rajdhani"; font-weight: 600; font-size: 19px; }}
#body {{ color: {MUTED}; font-family: "Barlow"; font-size: 12px; }}
#message {{
    background: {FIELD_BG};
    border: 1px solid {BORDER};
    border-radius: 6px;
}}
#path {{
    background: {FIELD_BG};
    border: 1px solid {BORDER};
    border-radius: 6px;
}}
#messagetext, #pathtext {{
    color: {TEXT};
    font-family: "Share Tech Mono";
    font-size: 12px;
}}
#browse {{ background: transparent; border: none; padding: 1px; }}
#close {{
    background: {FIELD_BG};
    border: 1px solid {BORDER};
    border-radius: 6px;
    color: {MUTED};
}}
#close:hover {{ background: {BORDER}; color: {TEXT}; }}
"""


def _fade_right(widget: QWidget) -> None:
    gradient = QLinearGradient(0, 0, 1, 0)
    gradient.setCoordinateMode(QGradient.CoordinateMode.ObjectBoundingMode)
    gradient.setColorAt(0.9, Qt.GlobalColor.black)
    gradient.setColorAt(1.0, Qt.GlobalColor.transparent)
    effect = QGraphicsOpacityEffect(widget)
    effect.setOpacity(1.0)
    effect.setOpacityMask(QBrush(gradient))
    widget.setGraphicsEffect(effect)


def _button_font() -> QFont:
    font = QFont()
    font.setFamilies(["Share Tech Mono", "monospace"])
    font.setPixelSize(12)
    font.setLetterSpacing(QFont.SpacingType.AbsoluteSpacing, 1.0)
    return font


def _folder_pixmap(color: str, opacity: float = 1.0) -> QPixmap:
    k = 48 / 24
    pixmap = QPixmap(48, 48)
    pixmap.fill(Qt.GlobalColor.transparent)
    painter = QPainter(pixmap)
    painter.setRenderHint(QPainter.RenderHint.Antialiasing)
    painter.setOpacity(opacity)
    try:
        pen = QPen(QColor(color))
        pen.setWidthF(2 * k)
        pen.setCapStyle(Qt.PenCapStyle.RoundCap)
        pen.setJoinStyle(Qt.PenJoinStyle.RoundJoin)
        painter.setPen(pen)
        painter.setBrush(Qt.BrushStyle.NoBrush)
        path = QPainterPath()
        path.moveTo(4 * k, 20 * k)
        path.lineTo(20 * k, 20 * k)
        path.quadTo(22 * k, 20 * k, 22 * k, 18 * k)
        path.lineTo(22 * k, 8 * k)
        path.quadTo(22 * k, 6 * k, 20 * k, 6 * k)
        path.lineTo(12.1 * k, 6 * k)
        path.quadTo(11 * k, 6 * k, 10.41 * k, 5.1 * k)
        path.lineTo(9.6 * k, 3.9 * k)
        path.quadTo(9 * k, 3 * k, 7.93 * k, 3 * k)
        path.lineTo(4 * k, 3 * k)
        path.quadTo(2 * k, 3 * k, 2 * k, 5 * k)
        path.lineTo(2 * k, 18 * k)
        path.quadTo(2 * k, 20 * k, 4 * k, 20 * k)
        path.closeSubpath()
        painter.drawPath(path)
    finally:
        painter.end()
    return pixmap


def _folder_icon(color: str, opacity: float = 1.0) -> QIcon:
    return QIcon(_folder_pixmap(color, opacity))


def _target_dir(chosen: Path) -> Path:
    return chosen if chosen.name.lower() == DIR_NAME.lower() else chosen / DIR_NAME


def _installed_dir() -> Path | None:
    import winreg

    with contextlib.suppress(OSError), winreg.OpenKey(winreg.HKEY_CURRENT_USER, UNINSTALL_KEY) as k:
        value, _ = winreg.QueryValueEx(k, "InstallLocation")
        if value and Path(value).is_dir():
            return Path(value)
    return None


def _apply_pending(root: Path) -> None:
    app = root / APP_SUBDIR
    pending = root / PENDING_SUBDIR
    if app.exists() or not (pending / PENDING_FILE).is_file():
        return
    with contextlib.suppress(OSError):
        pending.rename(app)
        for name in (PENDING_FILE, ATTEMPTED_FILE):
            (app / name).unlink(missing_ok=True)


def _launch_app(root: Path) -> None:
    proc = subprocess.Popen([str(app_exe(root))], cwd=str(root))
    ctypes.windll.user32.AllowSetForegroundWindow(proc.pid)


def _create_shortcut(root: Path) -> None:
    appdata = os.environ.get("APPDATA")
    if not appdata:
        return
    lnk = start_menu_shortcut(appdata)
    lnk_ps = str(lnk).replace("'", "''")
    exe_ps = str(app_exe(root)).replace("'", "''")
    dir_ps = str(root).replace("'", "''")
    ps = (
        f"$s=(New-Object -COM WScript.Shell).CreateShortcut('{lnk_ps}');"
        f"$s.TargetPath='{exe_ps}';"
        f"$s.WorkingDirectory='{dir_ps}';"
        f"$s.IconLocation='{exe_ps}';"
        f"$s.Save()"
    )
    subprocess.run(
        ["powershell", "-NoProfile", "-NonInteractive", "-Command", ps],
        creationflags=subprocess.CREATE_NO_WINDOW,
        check=False,
    )


def _register_uninstall(root: Path) -> None:
    import winreg

    exe = app_exe(root)
    with (
        contextlib.suppress(OSError),
        winreg.CreateKey(winreg.HKEY_CURRENT_USER, UNINSTALL_KEY) as k,
    ):
        winreg.SetValueEx(k, "DisplayName", 0, winreg.REG_SZ, APP_NAME)
        winreg.SetValueEx(k, "DisplayIcon", 0, winreg.REG_SZ, str(exe))
        winreg.SetValueEx(k, "InstallLocation", 0, winreg.REG_SZ, str(root))
        winreg.SetValueEx(k, "UninstallString", 0, winreg.REG_SZ, f'"{exe}" --uninstall')
        winreg.SetValueEx(k, "NoModify", 0, winreg.REG_DWORD, 1)
        winreg.SetValueEx(k, "NoRepair", 0, winreg.REG_DWORD, 1)


class _Cancelled(Exception):
    pass


def _latest_release() -> tuple[str, str]:
    req = urllib.request.Request(UPDATE_API_URL, headers={"User-Agent": APP_NAME})
    try:
        with urllib.request.urlopen(req, timeout=HTTP_TIMEOUT, context=SSL_CONTEXT) as r:
            data = json.load(r)
    except urllib.error.HTTPError as exc:
        limited = rate_limit_error(exc)
        if limited is not None:
            raise limited from exc
        raise
    url = next((a["url"] for a in data["assets"] if a["name"] == RUNTIME_ASSET), None)
    if url is None:
        raise LookupError(f"no release asset named {RUNTIME_ASSET}")
    return data["tag_name"], url


class Installer(QThread):
    progress = Signal(int)
    message = Signal(str)
    failed = Signal(str)
    done = Signal()

    def __init__(self, root: Path) -> None:
        super().__init__()
        self._cancel = False
        self.root = root

    def cancel(self) -> None:
        self._cancel = True

    def _check(self) -> None:
        if self._cancel:
            raise _Cancelled

    def run(self) -> None:
        archive = Path(tempfile.gettempdir()) / RUNTIME_ASSET
        last_pct = -1

        def on_progress(fraction: float) -> None:
            nonlocal last_pct
            pct = int(fraction * DOWNLOAD_PCT)
            if pct != last_pct:
                last_pct = pct
                self.progress.emit(pct)

        try:
            _, url = _latest_release()
            self.message.emit("Downloading")
            fetch_to(url, archive, on_progress=on_progress, cancelled=lambda: self._cancel)
            self._check()
            root = self.root
            self._extract(archive, root)
            with contextlib.suppress(OSError):
                _create_shortcut(root)
            _register_uninstall(root)
            self.done.emit()
        except Exception as e:
            shutil.rmtree(self.root / PENDING_SUBDIR, ignore_errors=True)
            with contextlib.suppress(OSError):
                self.root.rmdir()
            if not self._cancel:
                message = (
                    e.message()
                    if isinstance(e, RateLimited)
                    else f"{type(e).__name__}: {e}"[:ERROR_TEXT_MAX]
                )
                self.failed.emit(message)
        finally:
            with contextlib.suppress(OSError):
                archive.unlink()

    def _extract(self, archive: Path, root: Path) -> None:
        app = root / APP_SUBDIR
        pending = root / PENDING_SUBDIR
        previous = root / PREVIOUS_SUBDIR
        shutil.rmtree(pending, ignore_errors=True)
        pending.mkdir(parents=True, exist_ok=True)
        with zipfile.ZipFile(archive) as z:
            members = z.infolist()
            last_pct = -1
            for i, member in enumerate(members):
                self._check()
                z.extract(member, pending)
                pct = DOWNLOAD_PCT + int((i + 1) * (100 - DOWNLOAD_PCT) / len(members))
                if pct != last_pct:
                    last_pct = pct
                    self.progress.emit(pct)
                    name = member.filename.rstrip("/").rsplit("/", 1)[-1]
                    if name:
                        self.message.emit(name)
            missing = [m for m in members if not (pending / m.filename).exists()]
            if missing:
                raise OSError(f"{len(missing)} files are missing after extraction")
        self._check()
        has_backup = False
        if app.exists():
            shutil.rmtree(previous, ignore_errors=True)
            app.rename(previous)
            has_backup = True
        try:
            pending.rename(app)
        except OSError:
            if has_backup:
                with contextlib.suppress(OSError):
                    previous.rename(app)
            raise
        shutil.rmtree(previous, ignore_errors=True)


class IconButton(QPushButton):
    def __init__(self, normal: QIcon, hover: QIcon, inactive: QIcon) -> None:
        super().__init__()
        self._normal = normal
        self._hover = hover
        self._inactive = inactive
        self._active = True
        self.setIcon(normal)
        self.setCursor(Qt.CursorShape.PointingHandCursor)

    def set_active(self, active: bool) -> None:
        self._active = active
        self.setIcon(self._normal if active else self._inactive)
        self.setCursor(
            Qt.CursorShape.PointingHandCursor if active else Qt.CursorShape.ForbiddenCursor
        )

    def enterEvent(self, event) -> None:
        if self._active:
            self.setIcon(self._hover)

    def leaveEvent(self, event) -> None:
        self.setIcon(self._normal if self._active else self._inactive)


class ProgressButton(QPushButton):
    def __init__(self, text: str) -> None:
        super().__init__(text)
        self._progress = 0
        self._hover = False
        self._active = False
        self.setFocusPolicy(Qt.FocusPolicy.NoFocus)
        self.setCursor(Qt.CursorShape.ForbiddenCursor)

    def set_progress(self, value: int) -> None:
        self._progress = value
        self.update()

    def set_active(self, active: bool) -> None:
        self._active = active
        self._hover = active and self.underMouse()
        self.setCursor(
            Qt.CursorShape.PointingHandCursor if active else Qt.CursorShape.ForbiddenCursor
        )
        self.update()

    def mousePressEvent(self, event) -> None:
        if self._active:
            super().mousePressEvent(event)

    def enterEvent(self, event) -> None:
        self._hover = True
        self.update()

    def leaveEvent(self, event) -> None:
        self._hover = False
        self.update()

    def paintEvent(self, event) -> None:
        painter = QPainter(self)
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)
        rect = QRectF(self.rect())
        path = QPainterPath()
        path.addRoundedRect(rect, 6, 6)
        if self._active:
            painter.fillPath(path, QColor(ACCENT_HOVER if self._hover else ACCENT))
        else:
            painter.fillPath(path, QColor(ACCENT_DIM))
            width = rect.width() * self._progress / 100
            if width > 0:
                painter.setClipRect(QRectF(0, 0, width, rect.height()))
                painter.fillPath(path, QColor(ACCENT))
                painter.setClipping(False)
        painter.setPen(QColor("white"))
        painter.setFont(self.font())
        painter.drawText(rect, Qt.AlignmentFlag.AlignCenter, self.text())


class InstallerWindow(QWidget):
    tag_fetched = Signal(str)

    def __init__(self, root: Path) -> None:
        super().__init__()
        self._root = root
        self.setWindowFlags(Qt.WindowType.FramelessWindowHint)
        self.setAttribute(Qt.WidgetAttribute.WA_TranslucentBackground)
        self.setStyleSheet(STYLE)
        self.setFixedWidth(CARD_WIDTH)
        self._drag = None
        self._tag_stale = False

        card = QFrame()
        card.setObjectName("card")
        card.setAttribute(Qt.WidgetAttribute.WA_StyledBackground, True)
        shell = QVBoxLayout(self)
        shell.setContentsMargins(0, 0, 0, 0)
        shell.addWidget(card)

        layout = QVBoxLayout(card)
        layout.setContentsMargins(CARD_PADDING, CARD_PADDING - 4, CARD_PADDING, CARD_PADDING)
        layout.setSpacing(0)

        title = QLabel(APP_NAME)
        title.setObjectName("title")
        body = QLabel("Pick where to install the Launcher. You can add library folders later.")
        body.setObjectName("body")

        path_box = QFrame()
        path_box.setObjectName("path")
        path_box.setAttribute(Qt.WidgetAttribute.WA_StyledBackground, True)
        path_box.setFixedHeight(32)
        self._path_text = QLabel(str(root))
        self._path_text.setObjectName("pathtext")
        self._path_text.setSizePolicy(QSizePolicy.Policy.Ignored, QSizePolicy.Policy.Preferred)
        _fade_right(self._path_text)
        self._browse = IconButton(
            _folder_icon(MUTED), _folder_icon(TEXT), _folder_icon(MUTED, INACTIVE_OPACITY)
        )
        self._browse.setObjectName("browse")
        self._browse.setIconSize(QSize(15, 15))
        self._browse.setFixedSize(QSize(19, 19))
        self._browse.setToolTip("Change folder")
        self._browse.clicked.connect(self._on_browse)
        path_row = QHBoxLayout(path_box)
        path_row.setContentsMargins(8, 0, 8, 0)
        path_row.setSpacing(6)
        path_row.addWidget(self._path_text, 1)
        path_row.addWidget(self._browse)

        self._message = QFrame()
        self._message.setObjectName("message")
        self._message.setVisible(False)
        self._message.setAttribute(Qt.WidgetAttribute.WA_StyledBackground, True)
        self._message.setMaximumWidth(
            CARD_WIDTH - 2 * (CARD_PADDING + 1) - CLOSE_WIDTH - CONTINUE_WIDTH - 2 * BUTTON_GAP
        )
        self._message_text = QLabel()
        self._message_text.setObjectName("messagetext")
        _fade_right(self._message_text)
        self._message_text.graphicsEffect().setEnabled(False)
        message_row = QHBoxLayout(self._message)
        message_row.setContentsMargins(
            MESSAGE_PADDING, MESSAGE_PADDING, MESSAGE_PADDING, MESSAGE_PADDING
        )
        message_row.addWidget(self._message_text)

        self._primary = ProgressButton(self._action())
        self._primary.set_active(True)
        self._primary.clicked.connect(self._on_primary)
        self._primary.setFont(_button_font())
        self._primary.setFixedHeight(32)
        self._primary.setMinimumWidth(CONTINUE_WIDTH)
        self._close = QPushButton("Cancel")
        self._close.setObjectName("close")
        self._close.setFont(_button_font())
        self._close.setFixedHeight(32)
        self._close.setMinimumWidth(CLOSE_WIDTH)
        self._close.setCursor(Qt.CursorShape.PointingHandCursor)
        self._close.clicked.connect(self._on_cancel)
        button_row = QHBoxLayout()
        button_row.setSpacing(BUTTON_GAP)
        button_row.addWidget(self._message, 0, Qt.AlignmentFlag.AlignVCenter)
        button_row.addStretch(1)
        button_row.addWidget(self._close)
        button_row.addWidget(self._primary)

        layout.addWidget(title)
        layout.addWidget(body)
        layout.addSpacing(9)
        layout.addWidget(path_box)
        layout.addSpacing(10)
        layout.addLayout(button_row)

        self._installed = False
        self._installer = Installer(root)
        self._installer.progress.connect(self._primary.set_progress)
        self._installer.message.connect(self._on_message)
        self._installer.failed.connect(self._on_failed)
        self._installer.done.connect(self._on_done)
        self.tag_fetched.connect(self._on_message)
        threading.Thread(target=self._fetch_tag, daemon=True).start()

    def _fetch_tag(self) -> None:
        try:
            tag, _ = _latest_release()
        except Exception:
            return
        with contextlib.suppress(RuntimeError):
            if tag and not self._tag_stale:
                self.tag_fetched.emit(tag)

    def _action(self) -> str:
        return "Update" if app_exe(self._root).exists() else "Install"

    def _on_message(self, text: str) -> None:
        self._message_text.setText(text)
        room = self._message.maximumWidth() - 2 * (MESSAGE_PADDING + 1)
        long = self._message_text.fontMetrics().horizontalAdvance(text) > room
        self._message_text.graphicsEffect().setEnabled(long)
        self._message.setVisible(True)

    def _on_cancel(self) -> None:
        if self._installer.isRunning():
            self._close.setEnabled(False)
            self._on_message("Cancelling")
            self._installer.finished.connect(QApplication.quit)
            self._installer.cancel()
        else:
            QApplication.quit()

    def _on_failed(self, reason: str) -> None:
        self._primary.set_progress(0)
        self._primary.set_active(True)
        self._browse.set_active(True)
        self._on_message(reason)
        self._close.setEnabled(True)
        self._close.setText("Close")

    def _on_primary(self) -> None:
        if self._installer.isRunning():
            return
        self._tag_stale = True
        if self._installed:
            try:
                _launch_app(self._root)
            except OSError as e:
                self._on_message(f"{type(e).__name__}: {e}"[:ERROR_TEXT_MAX])
                return
            QApplication.quit()
            return
        self._primary.set_active(False)
        self._browse.set_active(False)
        self._installer.start()

    def _on_done(self) -> None:
        self._installed = True
        self._browse.setToolTip("Open folder")
        self._browse.set_active(True)
        self._primary.setText("Launch")
        self._primary.set_active(True)
        self._close.setEnabled(True)
        self._close.setText("Close")
        self._on_message("Done")

    def _on_browse(self) -> None:
        if self._installer.isRunning():
            return
        if self._installed:
            QDesktopServices.openUrl(QUrl.fromLocalFile(str(self._root)))
            return
        picked = QFileDialog.getExistingDirectory(
            self, "Choose install folder", str(self._root.parent)
        )
        if not picked:
            return
        self._root = _target_dir(Path(picked))
        self._installer.root = self._root
        self._path_text.setText(str(self._root))
        self._primary.setText(self._action())

    def mousePressEvent(self, event) -> None:
        if event.button() == Qt.MouseButton.LeftButton:
            self._drag = event.globalPosition().toPoint() - self.frameGeometry().topLeft()

    def mouseMoveEvent(self, event) -> None:
        if self._drag is not None and event.buttons() & Qt.MouseButton.LeftButton:
            self.move(event.globalPosition().toPoint() - self._drag)

    def mouseReleaseEvent(self, event) -> None:
        self._drag = None

    def keyPressEvent(self, event) -> None:
        if event.key() == Qt.Key.Key_Escape:
            self._on_cancel()

    def closeEvent(self, event) -> None:
        if self._installer.isRunning():
            event.ignore()
            self._on_cancel()
        else:
            event.accept()


def main() -> int:
    root = _installed_dir() or DEFAULT_DIR
    _apply_pending(root)
    if app_exe(root).exists():
        with contextlib.suppress(OSError):
            _launch_app(root)
            return 0
    app = QApplication(sys.argv)
    lock = QLockFile(str(Path(tempfile.gettempdir()) / f"{DIR_NAME}.lock"))
    lock.setStaleLockTime(0)
    if not lock.tryLock(0):
        return 0
    _load_fonts()
    window = InstallerWindow(root)
    window.show()
    geo = QApplication.primaryScreen().availableGeometry()
    window.move(geo.center() - window.rect().center())
    return app.exec()


if __name__ == "__main__":
    sys.exit(main())
