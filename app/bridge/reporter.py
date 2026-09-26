from collections.abc import Callable


class SignalReporter:
    def __init__(
        self,
        progress_emit: Callable[[float], None] | None = None,
        fail_emit: Callable[[str], None] | None = None,
        step_emit: Callable[[str], None] | None = None,
    ) -> None:
        self._step_emit = step_emit
        self._progress_emit = progress_emit
        self._fail_emit = fail_emit

    def update(self, text: str) -> None:
        if self._step_emit is not None:
            self._step_emit(text)

    def fail(self, text: str) -> None:
        if self._fail_emit is not None:
            self._fail_emit(text)

    def progress(self, fraction: float) -> None:
        if self._progress_emit is not None:
            self._progress_emit(fraction)
