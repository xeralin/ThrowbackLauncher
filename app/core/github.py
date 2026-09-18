import json
import math
import ssl
import threading
import time
import urllib.error
import urllib.request
from collections.abc import Callable
from pathlib import Path

import truststore

from core.constants import API_CACHE_FILE, HTTP_TIMEOUT

SSL_CONTEXT = truststore.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
USER_AGENT = "Throwback"
_API_CACHE_TTL = 600.0
_ATTEMPTS = 3
_RETRY_DELAY = 2
_CHUNK = 65536


class CancelledError(Exception):
    pass


class RateLimitError(Exception):
    def __init__(self, reset: str = "", retry_after: str = "") -> None:
        super().__init__("GitHub rate limit reached")
        self.minutes = _minutes(reset, time.time()) or _minutes(retry_after)

    def message(self) -> str:
        base = str(self)
        if self.minutes <= 0:
            return f"{base}, try again later"
        unit = "minute" if self.minutes == 1 else "minutes"
        return f"{base}, try again in {self.minutes} {unit}"


def _minutes(value: str, offset: float = 0.0) -> int:
    try:
        remaining = int(value) - offset
    except ValueError:
        return 0
    return math.ceil(remaining / 60) if remaining > 0 else 0


def rate_limit_error(exc: urllib.error.HTTPError) -> RateLimitError | None:
    if exc.code == 429 or (
        exc.code == 403
        and (exc.headers.get("x-ratelimit-remaining") == "0" or exc.headers.get("retry-after"))
    ):
        return RateLimitError(
            exc.headers.get("x-ratelimit-reset", ""), exc.headers.get("retry-after", "")
        )
    return None


_api_cache_lock = threading.Lock()


def _api_cache_read() -> dict:
    try:
        data = json.loads(API_CACHE_FILE.read_text())
        return data if isinstance(data, dict) else {}
    except OSError, ValueError:
        return {}


def _api_cache_flush(cache: dict) -> None:
    API_CACHE_FILE.parent.mkdir(parents=True, exist_ok=True)
    tmp = API_CACHE_FILE.with_name(API_CACHE_FILE.name + ".tmp")
    tmp.write_text(json.dumps(cache))
    tmp.replace(API_CACHE_FILE)


def _api_cache_store(api_url: str, etag: str, data: dict) -> None:
    with _api_cache_lock:
        cache = _api_cache_read()
        cache[api_url] = {"etag": etag, "data": data, "ts": time.time()}
        _api_cache_flush(cache)


def invalidate_api_cache() -> None:
    with _api_cache_lock:
        cache = _api_cache_read()
        for entry in cache.values():
            if isinstance(entry, dict):
                entry["ts"] = 0
        if cache:
            _api_cache_flush(cache)


def github_json(api_url: str, bypass_ttl: bool = False) -> dict:
    with _api_cache_lock:
        entry = _api_cache_read().get(api_url)
    usable = isinstance(entry, dict) and entry.get("etag") and "data" in entry
    if usable and not bypass_ttl and 0 <= time.time() - entry.get("ts", 0) < _API_CACHE_TTL:
        return entry["data"]
    headers = {"User-Agent": USER_AGENT}
    if usable:
        headers["If-None-Match"] = entry["etag"]
    req = urllib.request.Request(api_url, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=HTTP_TIMEOUT, context=SSL_CONTEXT) as r:
            data = json.load(r)
            etag = r.headers.get("ETag", "")
    except urllib.error.HTTPError as exc:
        if exc.code == 304 and isinstance(entry, dict):
            _api_cache_store(api_url, entry["etag"], entry["data"])
            return entry["data"]
        limited = rate_limit_error(exc)
        if limited is not None:
            raise limited from exc
        raise
    if etag:
        _api_cache_store(api_url, etag, data)
    return data


def _fetch_part(
    url: str,
    part: Path,
    on_progress: Callable[[float], None] | None,
    cancelled: Callable[[], bool] | None,
) -> None:
    offset = part.stat().st_size if part.exists() else 0
    headers = {"User-Agent": USER_AGENT, "Accept": "application/octet-stream"}
    if offset:
        headers["Range"] = f"bytes={offset}-"
    req = urllib.request.Request(url, headers=headers)
    try:
        r = urllib.request.urlopen(req, timeout=HTTP_TIMEOUT, context=SSL_CONTEXT)
    except urllib.error.HTTPError as exc:
        if exc.code == 416 and offset:
            return
        raise
    with r:
        if offset and r.status != 206:
            offset = 0
        length = r.headers.get("Content-Length")
        try:
            total = offset + int(length) if length else 0
        except ValueError:
            total = 0
        with part.open("ab" if offset else "wb") as f:
            done = offset
            if on_progress is not None and total > 0:
                on_progress(min(done / total, 1.0))
            while chunk := r.read(_CHUNK):
                if cancelled is not None and cancelled():
                    raise CancelledError
                f.write(chunk)
                done += len(chunk)
                if on_progress is not None and total > 0:
                    on_progress(min(done / total, 1.0))
            if 0 < total != f.tell():
                raise OSError(f"incomplete download ({f.tell()} of {total} bytes)")


def fetch_to(
    url: str,
    dest: Path,
    on_progress: Callable[[float], None] | None = None,
    cancelled: Callable[[], bool] | None = None,
) -> None:
    part = dest.with_name(dest.name + ".part")
    part.unlink(missing_ok=True)
    last: Exception | None = None
    for attempt in range(_ATTEMPTS):
        if attempt:
            if cancelled is not None and cancelled():
                part.unlink(missing_ok=True)
                raise CancelledError
            time.sleep(_RETRY_DELAY)
        try:
            _fetch_part(url, part, on_progress, cancelled)
            last = None
            break
        except CancelledError:
            part.unlink(missing_ok=True)
            raise
        except urllib.error.HTTPError as exc:
            limited = rate_limit_error(exc)
            if limited is not None:
                part.unlink(missing_ok=True)
                raise limited from exc
            last = exc
            if 400 <= exc.code < 500:
                break
        except OSError as exc:
            last = exc
    if last is not None:
        part.unlink(missing_ok=True)
        raise last
    part.replace(dest)


def _find_asset(data: dict, suffix: str) -> str | None:
    return next((a["url"] for a in data["assets"] if a["name"].endswith(suffix)), None)


def github_asset(api_url: str, suffix: str) -> tuple[str, str]:
    data = github_json(api_url)
    url = _find_asset(data, suffix)
    if url is None:
        data = github_json(api_url, bypass_ttl=True)
        url = _find_asset(data, suffix)
    if url is None:
        raise LookupError(f"no release asset matching *{suffix}")
    return data["tag_name"], url


def github_tag(api_url: str) -> str:
    return github_json(api_url)["tag_name"]
