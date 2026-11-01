import { cn } from "cn";
import { Check, Copy, X } from "lucide-react";
import { type ComponentProps, useEffect, useState } from "react";

import { Button } from "@/components/shadcn/button";
import { m } from "@/generated/paraglide/messages";

export function CopyButton({
  value,
  label,
  ...button
}: { value: string; label?: string } & Omit<
  ComponentProps<typeof Button>,
  "onClick" | "children"
>) {
  const [state, setState] = useState<"idle" | "copied" | "failed">("idle");

  useEffect(() => {
    if (state === "idle") return;

    const timer = setTimeout(() => setState("idle"), 1500);
    return () => clearTimeout(timer);
  }, [state]);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(value);
      setState("copied");
    } catch {
      setState("failed");
    }
  };

  const Icon = { idle: Copy, copied: Check, failed: X }[state];
  const text = {
    idle: undefined,
    copied: m.copy_copied(),
    failed: m.copy_failed(),
  }[state];

  if (label) {
    return (
      <Button variant="ghost" size="sm" {...button} onClick={() => void copy()}>
        <Icon />
        {text ?? label}
      </Button>
    );
  }

  return (
    <Button
      variant="ghost"
      size="icon-sm"
      aria-label={text ?? m.copy_label()}
      {...button}
      onClick={() => void copy()}
    >
      <Icon />
    </Button>
  );
}

export function CodeBlock({
  value,
  copyable = false,
}: {
  value: string;
  copyable?: boolean;
}) {
  return (
    <div
      className={cn(
        "relative rounded-lg bg-muted p-3 font-mono text-xs break-all whitespace-pre-wrap",
        copyable && "pr-11",
      )}
    >
      {value}
      {copyable && (
        <div className="absolute top-1.5 right-1.5">
          <CopyButton value={value} />
        </div>
      )}
    </div>
  );
}
