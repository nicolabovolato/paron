import "@xterm/xterm/css/xterm.css";

import { cn } from "cn";
import {
  Keyboard,
  KeyboardOff,
  Maximize,
  Minimize,
  Search,
} from "lucide-react";
import type { CSSProperties } from "react";

import type { MachineResponse, Protocol } from "@/generated/api/types.gen";
import { m } from "@/generated/paraglide/messages";

import { MouseContextMenu, TouchContextMenu } from "./context-menu";
import { useFullScreen } from "./device/full-screen";
import {
  useKeyboardHeight,
  useTerminalViewportMeta,
} from "./device/phone-keyboard";
import { useTouchDevice } from "./device/platform";
import { shortcutLabel } from "./device/shortcuts";
import { TerminalKeys } from "./keys";
import { SearchBar } from "./search-bar";
import { StatusLabel, StatusOverlay } from "./status";
import { ToolbarButton } from "./toolbar-button";
import { useTerminal } from "./use-terminal";

import styles from "./terminal.module.css";

export function MachineTerminal({
  machine,
  protocol,
}: {
  machine: MachineResponse;
  protocol: Protocol;
}) {
  const { container, ...terminal } = useTerminal(machine.id, protocol);

  const fullScreen = useFullScreen();

  const touchDevice = useTouchDevice();
  const keyboardHeight = useKeyboardHeight();
  useTerminalViewportMeta();

  return (
    <section
      className={cn(
        "flex min-h-0 flex-1 flex-col gap-3 bg-background data-full-screen:fixed data-full-screen:inset-0 data-full-screen:z-50 data-full-screen:gap-1.5 data-full-screen:p-1.5",
        // The keyboard height wins over the padding of full screen.
        keyboardHeight > 0 && "pb-(--keyboard-height)!",
      )}
      data-full-screen={fullScreen.active || undefined}
      style={{ "--keyboard-height": `${keyboardHeight}px` } as CSSProperties}
    >
      <header className="flex flex-wrap items-center gap-x-4 gap-y-2 in-data-full-screen:flex-nowrap in-data-full-screen:gap-x-2">
        <div className="flex min-w-0 items-baseline gap-3 in-data-full-screen:items-center">
          <h1 className="truncate text-xl font-semibold tracking-tight in-data-full-screen:text-sm in-data-full-screen:font-medium">
            {machine.name}
          </h1>
          <StatusLabel status={terminal.session.status} />
        </div>

        <div className="ml-auto flex items-center gap-1">
          {touchDevice && (
            <ToolbarButton
              label={m.terminal_phone_keyboard()}
              aria-pressed={terminal.phoneKeyboard.enabled}
              onClick={terminal.phoneKeyboard.toggle}
            >
              {terminal.phoneKeyboard.enabled ? <Keyboard /> : <KeyboardOff />}
            </ToolbarButton>
          )}
          <ToolbarButton
            label={m.terminal_search()}
            shortcut={shortcutLabel("search")}
            aria-pressed={terminal.searchBar.open}
            onClick={terminal.searchBar.toggle}
          >
            <Search />
          </ToolbarButton>
          <ToolbarButton
            label={
              fullScreen.active
                ? m.terminal_exit_full_screen()
                : m.terminal_full_screen()
            }
            onClick={() => void fullScreen.toggle()}
          >
            {fullScreen.active ? <Minimize /> : <Maximize />}
          </ToolbarButton>
        </div>
      </header>

      <div className="relative flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl border bg-card">
        <MouseContextMenu actions={terminal.actions}>
          <div
            ref={container}
            role="group"
            aria-label={m.terminal_label({ name: machine.name })}
            className={cn(styles.terminal, "absolute inset-y-2 right-0 left-3")}
          />
        </MouseContextMenu>
        <TouchContextMenu
          actions={terminal.actions}
          open={terminal.touchContextMenu.open}
          anchor={terminal.touchContextMenu.anchor}
          onClose={terminal.touchContextMenu.onClose}
        />
        <SearchBar
          open={terminal.searchBar.open}
          query={terminal.searchBar.query}
          matches={terminal.searchBar.matches}
          onQueryChange={terminal.searchBar.onQueryChange}
          onFind={terminal.searchBar.onFind}
          onClose={terminal.searchBar.onClose}
          field={terminal.searchBar.field}
        />
        <StatusOverlay
          status={terminal.session.status}
          onReconnect={terminal.session.reconnect}
        />
      </div>

      {touchDevice && (
        <TerminalKeys
          modifiers={terminal.keys.modifiers}
          onClick={terminal.keys.onClick}
          disabled={terminal.session.status.kind !== "connected"}
        />
      )}
    </section>
  );
}
