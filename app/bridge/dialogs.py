from PySide6.QtWidgets import QFileDialog

EXE_FILTER = ".exe installer (*.exe)"


def pick_file(title: str, name_filter: str) -> str:
    picked, _ = QFileDialog.getOpenFileName(None, title, "", name_filter)
    return picked
