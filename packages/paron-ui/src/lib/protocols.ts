import type { Protocol } from "@/generated/api/types.gen";
import { zProtocol } from "@/generated/api/zod.gen";

// Protocol names are the same in every language.
export const PROTOCOL_NAMES = {
  ssh: "SSH",
} satisfies Record<Protocol, string>;

// A record can name a protocol that this version does not know: it shows as recorded.
export function protocolName(recorded: string) {
  return zProtocol.validate(recorded) ? PROTOCOL_NAMES[recorded] : recorded;
}
