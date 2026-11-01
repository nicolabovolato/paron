import type { ComponentProps, ReactNode } from "react";

import { Button } from "@/components/shadcn/button";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/shadcn/tooltip";

import { pressKeepingFocus } from "./device/phone-keyboard";

export function ToolbarButton({
  label,
  shortcut,
  onClick,
  children,
  ...button
}: {
  label: string;
  shortcut?: string;
  onClick: () => void;
  children: ReactNode;
} & Omit<ComponentProps<typeof Button>, "children" | "onClick">) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={label}
            {...pressKeepingFocus(onClick)}
            {...button}
          />
        }
      >
        {children}
      </TooltipTrigger>
      <TooltipContent>
        {label}
        {shortcut && (
          <kbd className="ml-2 font-sans text-muted-foreground">{shortcut}</kbd>
        )}
      </TooltipContent>
    </Tooltip>
  );
}
