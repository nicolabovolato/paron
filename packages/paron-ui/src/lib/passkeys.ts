import type {
  PasskeyAssertionJson,
  PasskeyCreationOptionsJson,
  PasskeyRegistrationJson,
  PasskeyRequestOptionsJson,
} from "@/generated/api/types.gen";

export function passkeysSupported() {
  return (
    typeof PublicKeyCredential !== "undefined" &&
    "parseRequestOptionsFromJSON" in PublicKeyCredential
  );
}

function publicKeyCredential(credential: Credential | null) {
  if (!(credential instanceof PublicKeyCredential)) {
    throw new DOMException("No passkey was given.", "NotAllowedError");
  }

  return credential;
}

export async function createPasskey(options: PasskeyCreationOptionsJson) {
  const credential = publicKeyCredential(
    await navigator.credentials.create({
      publicKey: PublicKeyCredential.parseCreationOptionsFromJSON(
        options as unknown as PublicKeyCredentialCreationOptionsJSON,
      ),
    }),
  );

  return credential.toJSON() as unknown as PasskeyRegistrationJson;
}

export async function getPasskey(
  options: PasskeyRequestOptionsJson,
  request: Pick<CredentialRequestOptions, "mediation" | "signal"> = {},
) {
  const credential = publicKeyCredential(
    await navigator.credentials.get({
      ...request,
      publicKey: PublicKeyCredential.parseRequestOptionsFromJSON(
        options as unknown as PublicKeyCredentialRequestOptionsJSON,
      ),
    }),
  );

  return credential.toJSON() as unknown as PasskeyAssertionJson;
}
