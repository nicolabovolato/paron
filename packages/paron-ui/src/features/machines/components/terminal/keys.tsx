import { cn } from "cn";
import { ArrowDown, ArrowLeft, ArrowRight, ArrowUp } from "lucide-react";
import {
  useEffect,
  useEffectEvent,
  useState,
  type ComponentProps,
  type ReactNode,
} from "react";

import { Button } from "@/components/shadcn/button";
import { m } from "@/generated/paraglide/messages";

import { pressKeepingFocus } from "./device/phone-keyboard";
import {
  isModifier,
  type ModifierKey,
  type Modifiers,
  type SpecialKey,
} from "./xterm/key-encoding";

const REPEAT_DELAY_MS = 400;
const REPEAT_INTERVAL_MS = 100;

type RowKey = SpecialKey | ModifierKey;

const KEYS: RowKey[] = [
  "Escape",
  "Tab",
  "Home",
  "ArrowUp",
  "End",
  "PageUp",
  "ctrl",
  "alt",
  "ArrowLeft",
  "ArrowDown",
  "ArrowRight",
  "PageDown",
];
const FUNCTION_KEYS: RowKey[] = [
  "F1",
  "F2",
  "F3",
  "F4",
  "F5",
  "F6",
  "F7",
  "F8",
  "F9",
  "F10",
  "F11",
  "F12",
];

const FUNCTION_KEYS_LABEL = "Fn";

export function TerminalKeys({
  modifiers,
  onClick,
  disabled,
}: {
  modifiers: Modifiers;
  onClick: (key: RowKey) => void;
  disabled: boolean;
}) {
  const [showFunctionKeys, setShowFunctionKeys] = useState(false);

  const keyToElement: Partial<Record<RowKey, ReactNode>> = {
    Escape: "Esc",
    PageUp: "PgUp",
    PageDown: "PgDn",
    ctrl: "Ctrl",
    alt: "Alt",
    ArrowUp: <ArrowUp role="img" aria-label={m.terminal_key_up()} />,
    ArrowLeft: <ArrowLeft role="img" aria-label={m.terminal_key_left()} />,
    ArrowDown: <ArrowDown role="img" aria-label={m.terminal_key_down()} />,
    ArrowRight: <ArrowRight role="img" aria-label={m.terminal_key_right()} />,
  };

  return (
    <div className="grid shrink-0 grid-cols-7 gap-1">
      <ToggleKey
        pressed={showFunctionKeys}
        className="col-start-7 row-span-2 row-start-1 h-auto"
        onClick={() => setShowFunctionKeys(!showFunctionKeys)}
      >
        {FUNCTION_KEYS_LABEL}
      </ToggleKey>

      {(showFunctionKeys ? FUNCTION_KEYS : KEYS).map((key) =>
        isModifier(key) ? (
          <ToggleKey
            key={key}
            pressed={modifiers[key]}
            disabled={disabled}
            onClick={() => onClick(key)}
          >
            {keyToElement[key] ?? key}
          </ToggleKey>
        ) : (
          <Key key={key} disabled={disabled} onClick={() => onClick(key)}>
            {keyToElement[key] ?? key}
          </Key>
        ),
      )}
    </div>
  );
}

type KeyProps = { onClick: () => void } & Omit<
  ComponentProps<typeof Button>,
  "onClick"
>;

function KeyButton({ className, onClick, ...props }: KeyProps) {
  return (
    <Button
      variant="outline"
      className={cn("h-10 min-w-0 px-0 font-mono text-xs", className)}
      {...pressKeepingFocus(onClick)}
      {...props}
    />
  );
}

function ToggleKey({
  pressed,
  ...props
}: { pressed: boolean; children: ReactNode } & Omit<KeyProps, "variant">) {
  return (
    <KeyButton
      aria-pressed={pressed}
      variant={pressed ? "default" : "outline"}
      {...props}
    />
  );
}

function Key({ onClick, disabled, ...props }: KeyProps) {
  const [held, setHeld] = useState(false);
  // A disabled button gets no pointer events, and thus no release.
  if (held && disabled) setHeld(false);

  const repeatKey = useEffectEvent(onClick);
  useEffect(() => {
    if (!held) return;

    let repeatTimer = setTimeout(function repeat() {
      repeatKey();
      repeatTimer = setTimeout(repeat, REPEAT_INTERVAL_MS);
    }, REPEAT_DELAY_MS);
    return () => clearTimeout(repeatTimer);
  }, [held]);

  const release = () => setHeld(false);

  return (
    <KeyButton
      {...props}
      disabled={disabled}
      onClick={onClick}
      onPointerDown={(event) => {
        event.preventDefault();
        onClick();
        setHeld(true);
      }}
      onPointerUp={release}
      onPointerCancel={release}
      onPointerLeave={release}
    />
  );
}
