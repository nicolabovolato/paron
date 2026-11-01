import { Popover } from "@base-ui/react/popover";
import { ClipboardPaste, Copy, Eraser, TextSelect } from "lucide-react";
import type { ReactNode } from "react";

import { Button } from "@/components/shadcn/button";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuShortcut,
  ContextMenuTrigger,
} from "@/components/shadcn/context-menu";
import { m } from "@/generated/paraglide/messages";

import { pressKeepingFocus } from "./device/phone-keyboard";
import { shortcutLabel } from "./device/shortcuts";
import type { useTerminal } from "./use-terminal";

type TerminalActions = ReturnType<typeof useTerminal>["actions"];

export function MouseContextMenu({
  actions,
  children,
}: {
  actions: TerminalActions;
  children: ReactNode;
}) {
  return (
    <ContextMenu>
      <ContextMenuTrigger className="relative min-h-0 flex-1">
        {children}
      </ContextMenuTrigger>
      <ContextMenuContent
        // The closed menu gives the focus to the terminal, and not to the box that opened it.
        finalFocus={() => actions.inputElement() ?? true}
      >
        <ContextMenuItem disabled={!actions.copy} onClick={actions.copy}>
          <Copy />
          {m.terminal_copy()}
          <ContextMenuShortcut>{shortcutLabel("copy")}</ContextMenuShortcut>
        </ContextMenuItem>
        <ContextMenuItem disabled={!actions.paste} onClick={actions.paste}>
          <ClipboardPaste />
          {m.terminal_paste()}
          <ContextMenuShortcut>{shortcutLabel("paste")}</ContextMenuShortcut>
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem onClick={actions.selectAll}>
          <TextSelect />
          {m.terminal_select_all()}
        </ContextMenuItem>
        <ContextMenuItem disabled={!actions.clear} onClick={actions.clear}>
          <Eraser />
          {m.terminal_clear()}
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}

export function TouchContextMenu({
  actions,
  open,
  anchor,
  onClose,
}: {
  actions: TerminalActions;
  open: boolean;
  anchor: DOMRect;
  onClose: () => void;
}) {
  return (
    <Popover.Root
      open={open}
      onOpenChange={(nextOpen) => {
        if (!nextOpen) onClose();
      }}
    >
      <Popover.Portal>
        <Popover.Positioner
          anchor={{ getBoundingClientRect: () => anchor }}
          side="top"
          sideOffset={4}
          className="isolate z-50 outline-none"
        >
          <Popover.Popup
            role="toolbar"
            initialFocus={false}
            finalFocus={false}
            className="z-50 flex origin-(--transform-origin) gap-1 rounded-lg bg-popover p-1 text-popover-foreground shadow-md ring-1 ring-foreground/10 duration-100 outline-none data-[side=bottom]:slide-in-from-top-2 data-[side=top]:slide-in-from-bottom-2 data-open:animate-in data-open:fade-in-0 data-open:zoom-in-95 data-closed:animate-out data-closed:fade-out-0 data-closed:zoom-out-95"
          >
            <Button
              variant="ghost"
              size="sm"
              disabled={!actions.copy}
              {...pressKeepingFocus(() => {
                actions.copy?.();
                actions.clearSelection();
                onClose();
              })}
            >
              <Copy />
              {m.terminal_copy()}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              disabled={!actions.paste}
              {...pressKeepingFocus(() => {
                actions.paste?.();
                actions.clearSelection();
                onClose();
              })}
            >
              <ClipboardPaste />
              {m.terminal_paste()}
            </Button>
          </Popover.Popup>
        </Popover.Positioner>
      </Popover.Portal>
    </Popover.Root>
  );
}
