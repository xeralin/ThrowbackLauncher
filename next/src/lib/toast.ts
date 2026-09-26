export const RATE_LIMIT_TOAST = "rate-limit";

type ToastOptions = { key?: string };

export function showToast(text: string, options: ToastOptions = {}): void {
  window.dispatchEvent(
    new CustomEvent("throwback:toast", { detail: { text, ...options } }),
  );
}

export function dismissToast(ref: string | { id: number }): void {
  const detail = typeof ref === "string" ? { key: ref } : ref;
  window.dispatchEvent(new CustomEvent("throwback:toast-dismiss", { detail }));
}
