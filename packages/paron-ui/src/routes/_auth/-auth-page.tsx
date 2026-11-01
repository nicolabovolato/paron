import { useRouter, type ErrorComponentProps } from "@tanstack/react-router";
import { cn } from "cn";
import { RotateCw, ServerOff, type LucideIcon } from "lucide-react";
import type { ReactNode } from "react";

import { CopyButton } from "@/components/code-block";
import { Button } from "@/components/shadcn/button";
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/shadcn/empty";
import { m } from "@/generated/paraglide/messages";
import { errorMessage, isApiError, reportableRequestId } from "@/lib/api-error";

export function PageTitle({
  title,
  description,
}: {
  title: string;
  description: string;
}) {
  return (
    <div className="flex flex-col items-center gap-2 text-center">
      <h1 className="text-2xl font-bold">{title}</h1>
      <p className="text-balance text-muted-foreground">{description}</p>
    </div>
  );
}

export function AuthMessage({
  Icon,
  tone,
  title,
  description,
  children,
}: {
  Icon: LucideIcon;
  tone?: "destructive";
  title: string;
  description: ReactNode;
  children?: ReactNode;
}) {
  return (
    <Empty className="md:p-8">
      <EmptyHeader>
        <EmptyMedia
          variant="icon"
          className={cn(
            tone === "destructive" && "bg-destructive/10 text-destructive",
          )}
        >
          <Icon />
        </EmptyMedia>
        <EmptyTitle>
          <h1>{title}</h1>
        </EmptyTitle>
        <EmptyDescription>{description}</EmptyDescription>
      </EmptyHeader>
      {children && <EmptyContent>{children}</EmptyContent>}
    </Empty>
  );
}

/** The error component of the auth routes: the page could not get its data from the server. */
export function ServerUnreachable({ error, reset }: ErrorComponentProps) {
  const router = useRouter();
  if (!isApiError(error)) throw error;
  const requestId = reportableRequestId(error);

  return (
    <AuthMessage
      Icon={ServerOff}
      tone="destructive"
      title={m.sign_in_unavailable_title()}
      description={errorMessage(error)}
    >
      <Button
        variant="outline"
        onClick={() => {
          reset();
          void router.invalidate();
        }}
      >
        <RotateCw />
        {m.sign_in_unavailable_retry()}
      </Button>
      {requestId && (
        <CopyButton
          value={requestId}
          label={m.request_id_copy()}
          variant="outline"
          size="xs"
        />
      )}
    </AuthMessage>
  );
}
