import { cn } from "cn";
import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";

export function Row({
  time,
  marker,
  rail,
  wide = false,
  className,
  children,
}: {
  time?: ReactNode;
  marker?: ReactNode;
  rail?: ReactNode;
  wide?: boolean;
  className?: string;
  children: ReactNode;
}) {
  return (
    <div
      className={cn(
        "relative grid grid-cols-[--spacing(11)_--spacing(7)_minmax(0,1fr)] gap-x-3",
        className,
      )}
    >
      {rail}
      {time !== undefined && (
        <div className="col-start-1 text-right text-xs/7">{time}</div>
      )}
      {marker !== undefined && (
        <div className="relative col-start-2 justify-self-center">{marker}</div>
      )}
      <div
        className={cn(
          "relative min-w-0 text-sm/7",
          wide ? "col-span-full" : "col-start-3",
        )}
      >
        {children}
      </div>
    </div>
  );
}

export function Rail({
  dotted = false,
  className,
}: {
  dotted?: boolean;
  className?: string;
}) {
  return (
    <span
      aria-hidden
      className={cn(
        "absolute inset-0 col-start-2 col-end-3 mx-auto w-px",
        dotted
          ? "bg-[repeating-linear-gradient(to_bottom,currentColor_0_2px,transparent_2px_5px)] text-border"
          : "bg-border",
        className,
      )}
    />
  );
}

export function Marker({
  icon: Icon,
  className,
}: {
  icon: LucideIcon;
  className?: string;
}) {
  return (
    <span
      aria-hidden
      className={cn(
        "flex size-7 items-center justify-center rounded-full bg-muted text-muted-foreground",
        className,
      )}
    >
      <Icon className="size-4" />
    </span>
  );
}
