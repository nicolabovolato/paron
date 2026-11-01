import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

import { openMachineTerminal } from "./api";

class FakeWebSocket extends EventTarget {
  static readonly OPEN = 1;
  static readonly CLOSED = 3;
  static lastCreated: FakeWebSocket;

  readyState = FakeWebSocket.OPEN;
  binaryType = "blob";
  sentFrames: (string | Uint8Array)[] = [];
  readonly url: URL;

  constructor(url: URL) {
    super();
    this.url = url;
    FakeWebSocket.lastCreated = this;
  }

  send(data: string | Uint8Array) {
    this.sentFrames.push(data);
  }

  close() {
    if (this.readyState === FakeWebSocket.CLOSED) return;
    this.readyState = FakeWebSocket.CLOSED;
    this.dispatchEvent(new CloseEvent("close"));
  }

  dispatchMessage(data: string | ArrayBufferLike) {
    this.dispatchEvent(new MessageEvent("message", { data }));
  }
}

function openTerminal() {
  const handlers = {
    onOpen: vi.fn(),
    onOutput: vi.fn(),
    onClose: vi.fn(),
  };
  const connection = openMachineTerminal(
    "01a0fe93",
    { cols: 80, rows: 24, terminal: "xterm-truecolor", protocol: "ssh" },
    handlers,
  );

  return { handlers, connection, socket: FakeWebSocket.lastCreated };
}

beforeEach(() => {
  vi.stubGlobal("WebSocket", FakeWebSocket);
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("openMachineTerminal()", () => {
  test("URL", () => {
    const { url } = openTerminal().socket;

    expect(url.href).toBe(
      "wss://paron.test/api/machines/01a0fe93/terminal?cols=80&rows=24&terminal=xterm-truecolor&protocol=ssh",
    );
  });

  test("chunked input", () => {
    const { connection, socket } = openTerminal();

    connection.input(new Uint8Array(40 * 1024));

    expect(socket.sentFrames.map((frame) => frame.length)).toEqual([
      16 * 1024,
      16 * 1024,
      8 * 1024,
    ]);
  });

  test("control messages", () => {
    const { connection, socket } = openTerminal();

    connection.resize({ cols: 132, rows: 40 });
    connection.ack(32768);

    expect(socket.sentFrames).toEqual([
      '{"type":"resize","cols":132,"rows":40}',
      '{"type":"ack","bytes":32768}',
    ]);
  });

  test("output", () => {
    const { handlers, socket } = openTerminal();

    socket.dispatchMessage(new Uint8Array([104, 105]).buffer);

    expect(handlers.onOutput).toHaveBeenCalledWith(new Uint8Array([104, 105]));
  });

  test("open", () => {
    const { handlers, socket } = openTerminal();

    socket.dispatchEvent(new Event("open"));
    expect(handlers.onOpen).not.toHaveBeenCalled();
    socket.dispatchMessage('{"type":"opened"}');

    expect(handlers.onOpen).toHaveBeenCalledOnce();
  });

  test("exit_status", () => {
    const { handlers, socket } = openTerminal();

    socket.dispatchMessage('{"type":"opened"}');
    socket.dispatchMessage('{"type":"exit_status","status":3}');

    expect(handlers.onClose).toHaveBeenCalledExactlyOnceWith({
      type: "exit_status",
      status: 3,
    });
  });

  test.each(['{"type":"bell"}', "not JSON"])(
    "incorrect message: %s",
    (message) => {
      vi.spyOn(console, "error").mockImplementation(() => {});
      const { handlers, socket } = openTerminal();

      socket.dispatchMessage(message);

      expect(handlers.onClose).toHaveBeenCalledExactlyOnceWith(undefined);
      expect(console.error).toHaveBeenCalledOnce();
    },
  );

  test("closed", () => {
    const { handlers, connection, socket } = openTerminal();

    connection.close();
    socket.dispatchMessage(new Uint8Array([104, 105]).buffer);

    expect(handlers.onOutput).not.toHaveBeenCalled();
  });

  test("close event", () => {
    const { handlers, socket } = openTerminal();

    socket.dispatchMessage('{"type":"opened"}');
    socket.dispatchEvent(new CloseEvent("close"));

    expect(handlers.onClose).toHaveBeenCalledExactlyOnceWith(undefined);
  });
});
