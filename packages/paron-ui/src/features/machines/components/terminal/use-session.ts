import type { Terminal } from "@xterm/xterm";
import { useEffect, useState } from "react";

import { openMachineTerminal } from "@/features/machines/lib/api";
import type { Protocol } from "@/generated/api/types.gen";
import type { ApiError } from "@/lib/api-error";

import { isTouchDevice } from "./device/platform";
import { HIDE_CURSOR, SHOW_CURSOR, type Emulator } from "./xterm/use-emulator";

export type TerminalStatus =
  | { kind: "connecting" }
  | { kind: "connected" }
  | { kind: "exited"; status?: number; signal?: string }
  | { kind: "failed"; error?: ApiError }
  | { kind: "disconnected"; error?: ApiError };

const ACKNOWLEDGMENT_INTERVAL_BYTES = 32 * 1024;
const FAILURE_DELAY_MS = 500;

// A program sets modes in the terminal. If its connection breaks, the modes stay set. This
// sequence sets them off, in this order:
// - the alternate screen. In that screen the wheel sends arrow keys and does not scroll. This
//   also puts the cursor where it was saved, so the sequence saves the cursor first (DECSC):
//   with no alternate screen, the cursor then stays where it is.
// - the mouse reports and the focus reports.
// - bracketed paste.
// - a soft reset (DECSTR). It sets the cursor keys, the scroll region, the colours and the
//   character sets to their defaults.
// The scrollback does not change.
const RESET_PROGRAM_MODES =
  "\x1b7\x1b[?1049l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1004l\x1b[?2004l\x1b[!p";

export function useSession(
  emulator: Emulator | undefined,
  machineId: string,
  protocol: Protocol,
) {
  const [status, setStatus] = useState<TerminalStatus>({ kind: "connecting" });
  const [sessionNumber, setSessionNumber] = useState(0);

  useEffect(() => {
    if (!emulator) return;
    const { xterm } = emulator;
    setInputEnabled(xterm, false);

    let unacknowledgedBytes = 0;

    let shellOpened = false;
    const setClosedStatus = (closedStatus: TerminalStatus) => {
      setInputEnabled(xterm, false);
      xterm.write(RESET_PROGRAM_MODES + HIDE_CURSOR);
      setStatus(closedStatus);
    };

    const connection = openMachineTerminal(
      machineId,
      {
        cols: xterm.cols,
        rows: xterm.rows,
        terminal: "xterm-truecolor",
        protocol,
      },
      {
        onOpen: () => {
          shellOpened = true;

          const { baseY, cursorX, cursorY } = xterm.buffer.active;
          if (baseY + cursorY + cursorX > 0) xterm.write("\r\n\r\n");

          connection.resize(xterm);
          xterm.write(SHOW_CURSOR);
          setInputEnabled(xterm, true);
          setStatus({ kind: "connected" });

          if (!isTouchDevice()) xterm.focus();
        },
        onOutput: (bytes) => {
          xterm.write(bytes, () => {
            unacknowledgedBytes += bytes.length;

            if (unacknowledgedBytes < ACKNOWLEDGMENT_INTERVAL_BYTES) return;

            connection.ack(unacknowledgedBytes);
            unacknowledgedBytes = 0;
          });
        },
        onClose: (message) => {
          switch (message?.type) {
            case "exit_status":
              setClosedStatus({ kind: "exited", status: message.status });
              break;
            case "exit_signal":
              setClosedStatus({ kind: "exited", signal: message.signal });
              break;
            default:
              setClosedStatus({
                kind: shellOpened ? "disconnected" : "failed",
                error: message?.error,
              });
          }
        },
      },
    );

    const encoder = new TextEncoder();

    let resizeTimer: ReturnType<typeof setTimeout> | undefined;
    const subscriptions = [
      xterm.onData((text) => connection.input(encoder.encode(text))),
      xterm.onBinary((byteString) =>
        connection.input(
          Uint8Array.from(byteString, (character) => character.charCodeAt(0)),
        ),
      ),
      xterm.onResize((size) => {
        clearTimeout(resizeTimer);
        resizeTimer = setTimeout(() => connection.resize(size), 100);
      }),
    ];

    return () => {
      setInputEnabled(xterm, false);
      for (const subscription of subscriptions) subscription.dispose();
      clearTimeout(resizeTimer);
      connection.close();
    };
  }, [emulator, machineId, protocol, sessionNumber]);

  return {
    status: useDelayedFailure(status),
    reconnect: () => {
      setStatus({ kind: "connecting" });
      setSessionNumber((previousNumber) => previousNumber + 1);
    },
  };
}

function useDelayedFailure(status: TerminalStatus) {
  const [shownFailure, setShownFailure] = useState<TerminalStatus>();

  useEffect(() => {
    if (status.kind !== "failed") return;

    const timer = setTimeout(() => setShownFailure(status), FAILURE_DELAY_MS);
    return () => clearTimeout(timer);
  }, [status]);

  return status.kind === "failed" && status !== shownFailure
    ? ({ kind: "connecting" } satisfies TerminalStatus)
    : status;
}

// xterm.js is configured by assignment; React state holds the instance, so the assignment is here.
function setInputEnabled(xterm: Terminal, enabled: boolean) {
  xterm.options.disableStdin = !enabled;
}
