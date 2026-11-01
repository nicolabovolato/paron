import { setupServer } from "msw/node";

import {
  freshInstall,
  recoveryOptions,
  signedInAsAdministrator,
  startedSignIn,
} from "@/test/fixtures";

export const server = setupServer(
  signedInAsAdministrator,
  startedSignIn,
  recoveryOptions,
  ...freshInstall,
);
