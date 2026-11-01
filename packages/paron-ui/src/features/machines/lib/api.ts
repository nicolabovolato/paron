import {
  createMachineMutation,
  issueMachineEnrollmentTokenMutation,
  listMachinesOptions,
  readMachineOptions,
  removeMachineMutation,
  testMachineConnectionMutation,
  testMachineConnectionMutationKey,
  testNewMachineConnectionMutation,
  updateMachineMutation,
  withdrawMachineEnrollmentMutation,
} from "@/generated/api/@tanstack/react-query.gen";
import { client } from "@/generated/api/client.gen";
import type { Options } from "@/generated/api/sdk.gen";
import type {
  ListMachinesData,
  OpenMachineTerminalData,
  TerminalClientMessage,
  TerminalServerMessage,
  TestMachineConnectionData,
} from "@/generated/api/types.gen";
import { zTerminalServerMessage } from "@/generated/api/zod.gen";
import { useIsMutating, useMutation } from "@tanstack/react-query";

export const machineQueries = {
  list: (query: ListMachinesData["query"]) => listMachinesOptions({ query }),
  detail: (id: string) => readMachineOptions({ path: { id } }),
};

export function useTestMachineConnection() {
  return useMutation(testMachineConnectionMutation());
}

export function useTestNewMachineConnection() {
  return useMutation(testNewMachineConnectionMutation());
}

export function useIsTestingMachineConnection(id: string | undefined) {
  return (
    useIsMutating({
      mutationKey: testMachineConnectionMutationKey(),
      predicate: (mutation) =>
        id !== undefined &&
        (mutation.state.variables as Options<TestMachineConnectionData>).path
          .id === id,
    }) > 0
  );
}

export function useCreateMachine() {
  return useMutation(createMachineMutation());
}

export function useUpdateMachine() {
  return useMutation(updateMachineMutation());
}

export function useRemoveMachine() {
  return useMutation(removeMachineMutation());
}

export function useIssueMachineEnrollmentToken() {
  return useMutation(issueMachineEnrollmentTokenMutation());
}

export function useWithdrawMachineEnrollment() {
  return useMutation(withdrawMachineEnrollmentMutation());
}

type TerminalLastMessage = Extract<
  TerminalServerMessage,
  { type: "exit_status" | "exit_signal" | "error" }
>;
export function openMachineTerminal(
  id: string,
  query: OpenMachineTerminalData["query"],
  handlers: {
    onOpen: () => void;
    onOutput: (bytes: Uint8Array<ArrayBuffer>) => void;
    onClose: (message: TerminalLastMessage | undefined) => void;
  },
) {
  const TERMINAL_INPUT_CHUNK_BYTES = 16 * 1024;

  const url = new URL(
    client.buildUrl<OpenMachineTerminalData>({
      url: "/api/machines/{id}/terminal",
      path: { id },
      query,
    }),
    location.href,
  );
  url.protocol = url.protocol.replace("http", "ws");

  const ws = new WebSocket(url);
  ws.binaryType = "arraybuffer";
  const ac = new AbortController();

  const send = (message: TerminalClientMessage) => {
    if (ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify(message));
  };
  const end = (message: TerminalLastMessage | undefined) => {
    ac.abort();
    ws.close();
    handlers.onClose(message);
  };

  ws.addEventListener(
    "message",
    (event: MessageEvent<ArrayBuffer | string>) => {
      if (event.data instanceof ArrayBuffer) {
        handlers.onOutput(new Uint8Array(event.data));
        return;
      }

      let message;
      try {
        message = zTerminalServerMessage.parse(JSON.parse(event.data));
      } catch (error) {
        console.error(error);
        end(undefined);
        return;
      }

      switch (message.type) {
        case "opened":
          handlers.onOpen();
          break;
        case "exit_status":
        case "exit_signal":
        case "error":
          end(message);
          break;
      }
    },
    { signal: ac.signal },
  );
  ws.addEventListener("close", () => end(undefined), {
    signal: ac.signal,
  });

  return {
    input: (bytes: Uint8Array<ArrayBuffer>) => {
      if (ws.readyState !== WebSocket.OPEN) return;
      for (
        let start = 0;
        start < bytes.length;
        start += TERMINAL_INPUT_CHUNK_BYTES
      )
        ws.send(bytes.subarray(start, start + TERMINAL_INPUT_CHUNK_BYTES));
    },
    resize: ({ cols, rows }: { cols: number; rows: number }) =>
      send({ type: "resize", cols, rows }),
    ack: (bytes: number) => send({ type: "ack", bytes }),
    close: () => {
      ac.abort();
      ws.close();
    },
  };
}
