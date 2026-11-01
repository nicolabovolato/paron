import { m } from "@/generated/paraglide/messages";

export function endedWithoutPasskey(error: unknown) {
  return (
    error instanceof DOMException &&
    (error.name === "NotAllowedError" || error.name === "AbortError")
  );
}

export function passkeyErrorMessage(error: unknown) {
  if (!(error instanceof DOMException)) return undefined;
  if (endedWithoutPasskey(error)) return m.passkey_cancelled();
  if (error.name === "InvalidStateError") return m.passkey_already_on_device();
  return m.passkey_unavailable();
}
