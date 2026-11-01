import { cn } from "cn";
import type { ComponentProps } from "react";

import {
  CardAction as ShadcnCardAction,
  CardHeader as ShadcnCardHeader,
} from "@/components/shadcn/card";

export function CardHeader({
  className,
  ...props
}: ComponentProps<typeof ShadcnCardHeader>) {
  return (
    <ShadcnCardHeader
      className={cn(
        "flex flex-wrap items-center justify-between gap-x-4 gap-y-3",
        className,
      )}
      {...props}
    />
  );
}

export function CardAction({
  className,
  ...props
}: ComponentProps<typeof ShadcnCardAction>) {
  return (
    <ShadcnCardAction
      className={cn("flex flex-wrap gap-2 self-center", className)}
      {...props}
    />
  );
}
