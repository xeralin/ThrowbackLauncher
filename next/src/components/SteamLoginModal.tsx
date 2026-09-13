"use client";

import { useState } from "react";
import Link from "next/link";
import { Button } from "@/components/Button";
import { Modal } from "@/components/Modal";
import { inputClasses } from "@/components/ui";
import { useDownloader } from "@/lib/bridge";

function EyeIcon({ open }: { open: boolean }) {
  return (
    <svg
      viewBox="0 0 24 24"
      className="size-4"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7Z" />
      <circle cx="12" cy="12" r="3" />
      {!open && <line x1="4" y1="20" x2="20" y2="4" />}
    </svg>
  );
}

function PasswordInput({
  value,
  placeholder,
  autoFocus = false,
  onChange,
}: {
  value: string;
  placeholder?: string;
  autoFocus?: boolean;
  onChange: (value: string) => void;
}) {
  const [show, setShow] = useState(false);

  return (
    <div className="relative w-full">
      <input
        type={show ? "text" : "password"}
        value={value}
        placeholder={placeholder}
        autoFocus={autoFocus}
        onChange={(event) => onChange(event.target.value)}
        className={`w-full ${inputClasses} pr-10`}
      />
      <button
        type="button"
        aria-label={show ? "Hide password" : "Show password"}
        onClick={() => setShow((value) => !value)}
        className="absolute inset-y-0 right-0 flex items-center px-3 text-text-muted transition-colors hover:text-text"
      >
        <EyeIcon open={show} />
      </button>
    </div>
  );
}

export function SteamLoginModal() {
  const [override, setOverride] = useState<string | null | undefined>(
    undefined,
  );
  const [loginText, setLoginText] = useState("");
  const [loginAccount, setLoginAccount] = useState("");

  const dl = useDownloader({
    onLogin: (kind) => setOverride(kind),
    onDone: () => setOverride(null),
  });

  const loginKind = override === undefined ? dl.loginKind || null : override;

  const canSubmit =
    loginKind === "account"
      ? !!loginAccount.trim() && !!loginText.length
      : !!loginText.length;

  function reset() {
    setLoginText("");
    setLoginAccount("");
    setOverride(null);
  }

  function submitLogin() {
    if (!canSubmit) return;
    if (loginKind === "account") {
      dl.submitAccountLogin(loginAccount.trim(), loginText);
    } else {
      dl.submitLogin(loginText);
    }
    reset();
  }

  function cancelLogin() {
    dl.cancel();
    reset();
  }

  if (!loginKind) return null;
  const isGuard = loginKind.startsWith("guard");
  return (
    <Modal
      title={isGuard ? "Steam Guard" : "Steam login"}
      onClose={cancelLogin}
      onConfirm={submitLogin}
      footer={
        <>
          {loginKind === "account" && (
            <Link
              href="/faq/general#steam-login"
              className="mr-auto self-center text-ui text-link hover:underline"
            >
              Why does the Launcher need my Steam login?
            </Link>
          )}
          <Button variant="secondary" onClick={cancelLogin}>
            Cancel
          </Button>
          <Button variant="primary" disabled={!canSubmit} onClick={submitLogin}>
            Log in
          </Button>
        </>
      }
    >
      {loginKind === "account" ? (
        <>
          <input
            type="text"
            value={loginAccount}
            autoFocus
            placeholder="Account name"
            onChange={(event) => setLoginAccount(event.target.value)}
            className={`mb-2 w-full ${inputClasses}`}
          />
          <PasswordInput
            value={loginText}
            placeholder="Password"
            onChange={setLoginText}
          />
        </>
      ) : isGuard ? (
        <input
          type="text"
          value={loginText}
          autoFocus
          placeholder={loginKind === "guard_email" ? "Email code" : "Code"}
          onChange={(event) => setLoginText(event.target.value)}
          className={`w-full ${inputClasses}`}
        />
      ) : (
        <PasswordInput
          value={loginText}
          autoFocus
          placeholder="Password"
          onChange={setLoginText}
        />
      )}
    </Modal>
  );
}
