from pathlib import Path

from PySide6.QtCore import Property, QObject, Slot

from core.constants import NEXT_OUT_DIR
from core.manifest import edition_folder, is_installed, local_downloads, resolve_install


def _key_art_url(key: str) -> str | None:
    if (NEXT_OUT_DIR / "keyart" / f"{key}.webp").exists():
        return f"/keyart/{key}.webp"
    return None


def _season_entry(download: dict, hm: bool) -> dict:
    label = download["label"]
    code, _, name = label.partition(" ")
    return {
        "key": download["key"],
        "id": edition_folder(download["key"], hm),
        "hm": hm,
        "code": code,
        "name": name or label,
        "label": label,
        "sizeGb": download.get("size_gb"),
        "build": str(download.get("build", "")),
        "hmAvailable": bool(download.get("hm")),
        "hmBeta": bool(download.get("hm_beta", False)),
        "partial": False,
        "keyArt": _key_art_url(download["key"]),
    }


class LibraryController(QObject):
    def __init__(self, downloads: list[dict]) -> None:
        super().__init__()
        self._downloads = downloads

    @Property("QVariantList", constant=True)
    def seasons(self) -> list:
        return [
            _season_entry(d, False)
            for d in self._downloads
            if d.get("label") and "manifest_ww" in d
        ]

    @Slot(result="QVariantList")
    def home(self) -> list:
        entries: dict[str, dict] = {}
        folders = [(d, is_installed(d)) for d in local_downloads()]
        for folder, installed in sorted(folders, key=lambda entry: not entry[1]):
            self._merge(entries, folder, partial=not installed)
        return list(entries.values())

    def _merge(self, entries: dict, folder: Path, partial: bool) -> None:
        resolved = resolve_install(folder.name, self._downloads)
        if resolved is None:
            return
        download, is_hm = resolved
        entries.setdefault(
            folder.name,
            {**_season_entry(download, is_hm), "partial": partial},
        )
