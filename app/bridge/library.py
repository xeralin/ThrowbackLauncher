from PySide6.QtCore import Property, QObject, Slot

from core.constants import NEXT_OUT_DIR
from core.manifest import edition_folder, installed_path, partial_path


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
        "sizeGb": download["size_gb"],
        "build": download["build"],
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
        return [_season_entry(d, False) for d in self._downloads]

    @Slot(result="QVariantList")
    def home(self) -> list:
        entries = []
        for download in self._downloads:
            for hm in (False, True) if download.get("hm") else (False,):
                if installed_path(download["key"], hm) is not None:
                    entries.append(_season_entry(download, hm))
                elif partial_path(download["key"], hm) is not None:
                    entries.append({**_season_entry(download, hm), "partial": True})
        return entries
