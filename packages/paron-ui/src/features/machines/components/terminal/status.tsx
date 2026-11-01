import { cn } from "cn";
import { AlertCircle, RotateCcw, Unplug } from "lucide-react";

import { Button } from "@/components/shadcn/button";
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/shadcn/empty";
import { Spinner } from "@/components/shadcn/spinner";
import { m } from "@/generated/paraglide/messages";
import { errorMessage } from "@/lib/api-error";

import type { TerminalStatus } from "./use-session";

export function StatusOverlay({
  status,
  onReconnect,
}: {
  status: TerminalStatus;
  onReconnect: () => void;
}) {
  if (status.kind === "connected") return null;

  const error = "error" in status ? status.error : undefined;

  return (
    <Empty
      role={error ? "alert" : undefined}
      className="pointer-events-none absolute inset-0 z-10 bg-card/90 *:pointer-events-auto"
    >
      {status.kind === "connecting" ? (
        <EmptyHeader
          key="connecting"
          className="animate-in duration-200 fade-in-0 zoom-in-95"
        >
          <EmptyMedia>
            <Spinner className="text-muted-foreground" aria-hidden />
          </EmptyMedia>
          <EmptyDescription>{m.terminal_status_connecting()}</EmptyDescription>
        </EmptyHeader>
      ) : (
        <>
          <EmptyHeader
            key="closed"
            className="animate-in duration-200 fade-in-0 zoom-in-95"
          >
            <EmptyMedia>
              {error ? (
                <AlertCircle className="size-6 text-destructive" aria-hidden />
              ) : (
                <Unplug className="size-6 text-muted-foreground" aria-hidden />
              )}
            </EmptyMedia>
            <EmptyTitle>
              {status.kind === "failed"
                ? m.terminal_failed_title()
                : status.kind === "disconnected"
                  ? m.terminal_disconnected_title()
                  : statusText(status)}
            </EmptyTitle>
            <EmptyDescription>
              {error ? errorMessage(error) : m.terminal_closed_description()}
            </EmptyDescription>
          </EmptyHeader>
          <EmptyContent className="animate-in duration-200 fade-in-0 zoom-in-95">
            <Button onClick={onReconnect}>
              <RotateCcw />
              {m.terminal_reconnect()}
            </Button>
          </EmptyContent>
        </>
      )}
    </Empty>
  );
}

export function StatusLabel({ status }: { status: TerminalStatus }) {
  return (
    <span
      role="status"
      className="flex min-w-0 items-baseline gap-2 text-sm text-muted-foreground"
    >
      {status.kind === "connecting" ? (
        <Spinner className="size-3.5 self-center" aria-hidden />
      ) : (
        <span
          aria-hidden
          className={cn(
            "size-2 self-center rounded-full",
            status.kind === "connected" && "bg-success",
            status.kind === "exited" && "bg-muted-foreground/50",
            (status.kind === "failed" || status.kind === "disconnected") &&
              "bg-destructive",
          )}
        />
      )}
      <span
        className={cn(
          "truncate",
          status.kind === "connected" && "in-data-full-screen:sr-only",
        )}
      >
        {statusText(status)}
      </span>
    </span>
  );
}

function statusText(status: TerminalStatus) {
  switch (status.kind) {
    case "connecting":
      return m.terminal_status_connecting();
    case "connected":
      return m.terminal_status_connected();
    case "exited":
      if (status.status !== undefined)
        return m.terminal_status_exited({ status: status.status });
      if (status.signal !== undefined)
        return m.terminal_status_killed({ signal: status.signal });
      return m.terminal_status_ended();
    case "failed":
      return m.terminal_status_failed();
    case "disconnected":
      return m.terminal_status_disconnected();
  }
}
