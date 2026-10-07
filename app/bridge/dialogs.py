from PySide6.QtCore import QStandardPaths
from PySide6.QtWidgets import QFileDialog

EXE_FILTER = ".exe installer (*.exe)"


def pick_file(title: str, name_filter: str) -> str:
    downloads = QStandardPaths.writableLocation(QStandardPaths.StandardLocation.DownloadLocation)
    picked, _ = QFileDialog.getOpenFileName(None, title, downloads, name_filter)
    return picked
