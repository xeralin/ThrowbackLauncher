from PySide6.QtWidgets import QFileDialog

EXE_FILTER = ".exe installer (*.exe)"


def pick_file(title: str, name_filter: str) -> str:
    picked, _ = QFileDialog.getOpenFileName(None, title, "", name_filter)
    return picked


def save_file(title: str, suggested: str) -> str:
    picked, _ = QFileDialog.getSaveFileName(None, title, suggested)
    return picked
