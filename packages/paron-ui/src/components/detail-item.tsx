import { cn } from "cn";
import type { ReactNode } from "react";

export function DetailItem({
  term,
  className,
  valueClassName,
  children,
}: {
  term: string;
  className?: string;
  valueClassName?: string;
  children: ReactNode;
}) {
  return (
    <div className={cn("flex flex-col gap-1", className)}>
      <dt className="text-muted-foreground">{term}</dt>
      <dd className={cn("font-medium", valueClassName)}>{children}</dd>
    </div>
  );
}
