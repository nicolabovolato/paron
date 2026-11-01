import { act, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { userEvent } from "vitest/browser";

import { ThemeProvider } from "@/components/theme-provider";
import { MACHINE } from "@/test/fixtures";

import { MachineTerminal } from ".";

class FakeWebSocket extends EventTarget {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  static instances: FakeWebSocket[] = [];

  readyState = FakeWebSocket.CONNECTING;
  binaryType = "blob";
  sentFrames: (string | Uint8Array)[] = [];
  readonly url: string;

  constructor(url: string) {
    super();
    this.url = url;
    FakeWebSocket.instances.push(this);
  }

  send(data: string | Uint8Array) {
    this.sentFrames.push(data);
  }

  close() {
    if (this.readyState === FakeWebSocket.CLOSED) return;
    this.readyState = FakeWebSocket.CLOSED;
    this.dispatchEvent(new CloseEvent("close"));
  }

  accept() {
    this.readyState = FakeWebSocket.OPEN;
    this.dispatchEvent(new Event("open"));
  }

  openTerminal() {
    this.accept();
    this.dispatchMessage(JSON.stringify({ type: "opened" }));
  }

  dispatchMessage(data: string | ArrayBufferLike) {
    this.dispatchEvent(new MessageEvent("message", { data }));
  }

  sentInput() {
    return this.sentFrames
      .filter((frame) => frame instanceof Uint8Array)
      .map((frame) => new TextDecoder().decode(frame))
      .join("");
  }

  sentControlMessages() {
    return this.sentFrames
      .filter((frame) => typeof frame === "string")
      .map((frame) => JSON.parse(frame) as { type: string; bytes?: number });
  }
}

function renderTerminal() {
  return render(
    <ThemeProvider>
      <div className="flex h-svh flex-col">
        <MachineTerminal machine={MACHINE} protocol="ssh" />
      </div>
    </ThemeProvider>,
  );
}

// Under StrictMode an effect runs twice. The socket that stays is the last one.
const currentSocket = () =>
  waitFor(() => {
    const socket = FakeWebSocket.instances.at(-1);
    if (!socket) throw new Error("No socket is open");
    return socket;
  });

async function renderConnectedTerminal() {
  const view = renderTerminal();
  const socket = await currentSocket();
  socket.openTerminal();
  await waitFor(() =>
    expect(screen.getByRole("status")).toHaveTextContent("Connected"),
  );

  return { view, socket };
}

beforeEach(() => {
  FakeWebSocket.instances = [];
  vi.stubGlobal("WebSocket", FakeWebSocket);
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("<MachineTerminal />", () => {
  test("one terminal", async () => {
    await renderConnectedTerminal();

    expect(
      screen.getAllByRole("textbox", { name: "Terminal input" }),
    ).toHaveLength(1);
  });

  test("opened", async () => {
    renderTerminal();
    const socket = await currentSocket();
    // The status in the header and the notice in the terminal both say it.
    expect(screen.getAllByText("Connecting")).toHaveLength(2);

    socket.openTerminal();

    await waitFor(() =>
      expect(screen.queryByText("Connecting")).not.toBeInTheDocument(),
    );
  });

  test("typing", async () => {
    const user = userEvent.setup();
    const { socket } = await renderConnectedTerminal();

    await user.keyboard("ls{Enter}");

    await waitFor(() => expect(socket.sentInput()).toBe("ls\r"));
  });

  test("paste", async () => {
    renderTerminal();
    const socket = await currentSocket();
    const pastedText = "y".repeat(40 * 1024);
    const clipboardData = new DataTransfer();
    clipboardData.setData("text/plain", pastedText);
    const status = screen.getByRole("status");
    const input = screen.getByRole("textbox", { name: "Terminal input" });

    const pasteOnConnected = new MutationObserver(() => {
      if (status.textContent !== "Connected") return;
      pasteOnConnected.disconnect();
      input.dispatchEvent(
        new ClipboardEvent("paste", { clipboardData, bubbles: true }),
      );
    });
    pasteOnConnected.observe(status, {
      childList: true,
      characterData: true,
      subtree: true,
    });
    socket.openTerminal();

    await waitFor(() => expect(socket.sentInput()).toBe(pastedText));
  });

  test("search kept", async () => {
    const user = userEvent.setup();
    const { socket } = await renderConnectedTerminal();
    socket.dispatchMessage(new TextEncoder().encode("needle").buffer);

    await user.click(screen.getByRole("button", { name: "Search" }));
    const field = screen.getByRole("textbox", { name: "Search" });
    await waitFor(() => expect(field).toHaveFocus());
    await user.keyboard("needle");
    await user.click(screen.getByRole("button", { name: "Close search" }));
    await waitFor(() => expect(field).not.toBeInTheDocument());
    await user.click(screen.getByRole("button", { name: "Search" }));

    const reopenedField = screen.getByRole("textbox", { name: "Search" });
    await waitFor(() => expect(reopenedField).toHaveFocus());
    expect(reopenedField).toHaveSelection("needle");
    await waitFor(() => expect(screen.getByText("1 of 1")).toBeVisible());
  });

  test("search closed", async () => {
    const user = userEvent.setup();
    await renderConnectedTerminal();

    await user.click(screen.getByRole("button", { name: "Search" }));
    const field = screen.getByRole("textbox", { name: "Search" });
    await waitFor(() => expect(field).toHaveFocus());
    await user.keyboard("{Escape}");

    await waitFor(() => expect(field).not.toBeInTheDocument());
  });

  test("ack", async () => {
    const { socket } = await renderConnectedTerminal();

    socket.dispatchMessage(new Uint8Array(40 * 1024).fill(0x61).buffer);

    await waitFor(() =>
      expect(socket.sentControlMessages()).toContainEqual({
        type: "ack",
        bytes: 40 * 1024,
      }),
    );
  });

  test("resize", async () => {
    const { view, socket } = await renderConnectedTerminal();
    const resizeCount = () =>
      socket.sentControlMessages().filter(({ type }) => type === "resize")
        .length;
    const resizeCountAtOpen = resizeCount();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });

    for (const width of [600, 560, 520, 480]) {
      view.container.style.width = `${width}px`;
      await new Promise((resolve) => requestAnimationFrame(resolve));
    }
    // The resize observer of the last width reports in the frame after it.
    await new Promise((resolve) => requestAnimationFrame(resolve));
    await act(() => vi.advanceTimersByTimeAsync(100));

    expect(resizeCount() - resizeCountAtOpen).toBe(1);
  });

  test("exit", async () => {
    const { socket } = await renderConnectedTerminal();

    socket.dispatchMessage(JSON.stringify({ type: "exit_status", status: 3 }));

    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent("Exited (3)"),
    );
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Reconnect" })).toBeVisible(),
    );
  });

  test("disconnected", async () => {
    const { socket } = await renderConnectedTerminal();

    socket.close();

    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent("Disconnected"),
    );
  });

  test("refused", async () => {
    renderTerminal();
    const socket = await currentSocket();
    socket.accept();

    socket.dispatchMessage(
      JSON.stringify({
        type: "error",
        error: {
          status: 502,
          detail: "the machine could not be reached",
        },
      }),
    );

    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent("Couldn't connect"),
    );
    expect(
      screen.getByText("The machine could not be reached"),
    ).toBeInTheDocument();
  });
});
