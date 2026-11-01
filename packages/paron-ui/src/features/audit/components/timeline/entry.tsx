import { ParaglideMessage } from "@inlang/paraglide-js-react";
import { Link, type LinkOptions } from "@tanstack/react-router";
import { cn } from "cn";
import {
  ChevronRight,
  ExternalLink,
  Funnel,
  KeyRound,
  LogIn,
  LogOut,
  Pencil,
  Plus,
  ShieldMinus,
  ShieldPlus,
  SquareTerminal,
  SquareX,
  Trash2,
  TriangleAlert,
  type LucideIcon,
} from "lucide-react";
import { memo, useEffect, useId, useRef, type ReactNode } from "react";

import { CodeBlock } from "@/components/code-block";
import { Button } from "@/components/shadcn/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/components/shadcn/dropdown-menu";
import { DetailsTable } from "@/features/audit/components/timeline/details-table";
import { Marker, Rail, Row } from "@/features/audit/components/timeline/layout";
import {
  authMethodName,
  changedMachineRows,
  changedUserRows,
  createdMachineRows,
  createdUserRows,
  recoveryStartedRows,
  signedInRows,
} from "@/features/audit/lib/details";
import type {
  AuditActorResponse,
  AuditEntityResponse,
  AuditEntryResponse,
  AuditEventResponse,
} from "@/generated/api/types.gen";
import { m } from "@/generated/paraglide/messages";
import { formatDateTime, formatTime } from "@/lib/format";
import { protocolName } from "@/lib/protocols";

type EventType = AuditEventResponse["type"];

type Events = {
  [Type in EventType]: Extract<AuditEventResponse, { type: Type }>;
};

type DescriptionMarkup = { actor: () => ReactNode; target: () => ReactNode };

type EventView<Event> = {
  icon: LucideIcon;
  color?: string;
  description: (event: Event, markup: DescriptionMarkup) => ReactNode;
  details?: (event: Event) => ReactNode;
};

const EVENT_VIEWS: { [Type in EventType]: EventView<Events[Type]> } = {
  "user.signed_in": {
    icon: LogIn,
    description: (_, markup) => (
      <ParaglideMessage
        message={m.audit_event_user_signed_in}
        markup={markup}
      />
    ),
    details: ({ payload }) =>
      payload.methods.length > 0 ? (
        <DetailsTable rows={signedInRows(payload)} />
      ) : undefined,
  },
  "user.signed_out": {
    icon: LogOut,
    description: (_, markup) => (
      <ParaglideMessage
        message={m.audit_event_user_signed_out}
        markup={markup}
      />
    ),
  },
  "user.created": {
    icon: Plus,
    description: (_, markup) => (
      <ParaglideMessage message={m.audit_event_user_created} markup={markup} />
    ),
    details: ({ payload }) => <DetailsTable rows={createdUserRows(payload)} />,
  },
  "user.updated": {
    icon: Pencil,
    description: (_, markup) => (
      <ParaglideMessage message={m.audit_event_user_updated} markup={markup} />
    ),
    details: ({ payload }) => <DetailsTable rows={changedUserRows(payload)} />,
  },
  "user.deleted": {
    icon: Trash2,
    color: "text-destructive",
    description: (_, markup) => (
      <ParaglideMessage message={m.audit_event_user_deleted} markup={markup} />
    ),
  },
  "user.recovery_started": {
    icon: KeyRound,
    color: "text-warning",
    description: (_, markup) => (
      <ParaglideMessage
        message={m.audit_event_user_recovery_started}
        markup={markup}
      />
    ),
    details: ({ payload }) => (
      <DetailsTable rows={recoveryStartedRows(payload)} />
    ),
  },
  "user.credential_added": {
    icon: ShieldPlus,
    description: ({ payload }, markup) => {
      switch (payload.kind) {
        case "password":
          return (
            <ParaglideMessage
              message={m.audit_event_user_password_set}
              markup={markup}
            />
          );
        case "recovery_code":
          return (
            <ParaglideMessage
              message={m.audit_event_user_recovery_codes_generated}
              markup={markup}
            />
          );
        default:
          return (
            <ParaglideMessage
              message={m.audit_event_user_credential_added}
              inputs={{ kind: authMethodName(payload.kind) }}
              markup={markup}
            />
          );
      }
    },
  },
  "user.credential_removed": {
    icon: ShieldMinus,
    color: "text-warning",
    description: ({ payload }, markup) => (
      <ParaglideMessage
        message={m.audit_event_user_credential_removed}
        inputs={{ kind: authMethodName(payload.kind) }}
        markup={markup}
      />
    ),
  },
  "machine.created": {
    icon: Plus,
    description: (_, markup) => (
      <ParaglideMessage
        message={m.audit_event_machine_created}
        markup={markup}
      />
    ),
    details: ({ payload }) => (
      <DetailsTable rows={createdMachineRows(payload)} />
    ),
  },
  "machine.updated": {
    icon: Pencil,
    description: (_, markup) => (
      <ParaglideMessage
        message={m.audit_event_machine_updated}
        markup={markup}
      />
    ),
    details: ({ payload }) => (
      <DetailsTable rows={changedMachineRows(payload)} />
    ),
  },
  "machine.deleted": {
    icon: Trash2,
    color: "text-destructive",
    description: (_, markup) => (
      <ParaglideMessage
        message={m.audit_event_machine_deleted}
        markup={markup}
      />
    ),
  },
  "machine.terminal_opened": {
    icon: SquareTerminal,
    description: ({ payload }, markup) => (
      <ParaglideMessage
        message={m.audit_event_machine_terminal_opened}
        inputs={{ protocol: protocolName(payload.protocol) }}
        markup={markup}
      />
    ),
  },
  "machine.terminal_closed": {
    icon: SquareX,
    description: (_, markup) => (
      <ParaglideMessage
        message={m.audit_event_machine_terminal_closed}
        markup={markup}
      />
    ),
  },
  unknown: {
    icon: TriangleAlert,
    color: "text-warning",
    description: ({ payload }, markup) => (
      <ParaglideMessage
        message={m.audit_event_unknown}
        inputs={{ type: payload.entry_type }}
        markup={{
          ...markup,
          code: ({ children }) => (
            <code className="font-mono text-xs">{children}</code>
          ),
        }}
      />
    ),
    details: ({ payload }) => (
      <div className="flex flex-col gap-2">
        <p className="text-xs text-muted-foreground">
          {m.audit_unknown_description()}
        </p>
        <CodeBlock value={payload.payload} />
      </div>
    ),
  },
};

function eventView<Type extends EventType>(type: Type) {
  const view: EventView<Events[Type]> = EVENT_VIEWS[type];
  return view;
}

export type EntityActions = (entity: AuditEntityResponse) => {
  filter?: LinkOptions;
  open?: LinkOptions;
};

export const Entry = memo(function Entry({
  entry,
  minutesAfterPrevious,
  lastOfDay,
  position,
  focusRequested,
  expanded,
  onExpandedChange,
  entityActions,
}: {
  entry: AuditEntryResponse;
  minutesAfterPrevious: number;
  lastOfDay: boolean;
  position: number;
  focusRequested: boolean;
  expanded: boolean;
  onExpandedChange: (id: string, expanded: boolean) => void;
  entityActions: EntityActions;
}) {
  const detailsId = useId();
  const timeId = useId();
  const descriptionId = useId();
  const { actor, target, event } = entry;
  const view = eventView(event.type);
  const details = view.details?.(event);
  const article = useRef<HTMLElement>(null);

  useEffect(() => {
    if (focusRequested) article.current?.focus({ preventScroll: true });
  }, [focusRequested]);

  return (
    <article
      ref={article}
      tabIndex={0}
      aria-labelledby={`${timeId} ${descriptionId}`}
      aria-posinset={position}
      aria-setsize={-1}
      className="rounded-md outline-none focus-visible:ring-3 focus-visible:ring-ring/50"
    >
      <Row
        className={cn(
          minutesAfterPrevious >= 5 && "pt-2",
          minutesAfterPrevious >= 15 && "pt-4",
          minutesAfterPrevious >= 60 && "pt-7",
          minutesAfterPrevious >= 3 * 60 && "pt-11",
          minutesAfterPrevious >= 8 * 60 && "pt-16",
          lastOfDay && "pb-4",
        )}
        rail={<Rail />}
        time={
          <time
            id={timeId}
            dateTime={entry.created_at}
            title={formatDateTime(entry.created_at)}
            className="text-muted-foreground tabular-nums"
          >
            {formatTime(entry.created_at)}
          </time>
        }
        marker={<Marker icon={view.icon} className={view.color} />}
      >
        <div className={expanded ? "pb-7" : "pb-4"}>
          <p>
            <span id={descriptionId}>
              {view.description(event, {
                actor: () => (
                  <EntityName entity={actor} entityActions={entityActions} />
                ),
                target: () => (
                  <EntityName entity={target} entityActions={entityActions} />
                ),
              })}
            </span>
            {details !== undefined && (
              <Button
                variant="ghost"
                size="xs"
                className="ml-1.5 align-middle text-muted-foreground"
                aria-expanded={expanded}
                aria-controls={detailsId}
                onClick={() => onExpandedChange(entry.id, !expanded)}
              >
                <ChevronRight
                  className={cn(
                    "transition-transform motion-reduce:transition-none",
                    expanded && "rotate-90",
                  )}
                />
                {expanded ? m.audit_changes_hide() : m.audit_changes_show()}
              </Button>
            )}
          </p>
          {expanded && (
            <div
              id={detailsId}
              className="mt-2 transition-[opacity,translate] duration-200 ease-out motion-reduce:transition-none starting:-translate-y-1 starting:opacity-0"
            >
              {details}
            </div>
          )}
        </div>
      </Row>
    </article>
  );
});

function EntityName({
  entity,
  entityActions,
}: {
  entity: AuditActorResponse;
  entityActions: EntityActions;
}) {
  if (entity.type === "system")
    return (
      <span className="text-muted-foreground">{m.audit_actor_system()}</span>
    );

  const { filter, open } = entityActions(entity);
  if (!filter && !open)
    return <span className="font-medium">{entity.name}</span>;

  const user = entity.type === "user";

  return (
    <DropdownMenu>
      <DropdownMenuTrigger className="cursor-pointer rounded-sm text-left font-medium underline decoration-foreground/40 underline-offset-4 outline-none hover:decoration-foreground focus-visible:ring-3 focus-visible:ring-ring/50 data-popup-open:decoration-foreground">
        {entity.name}
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-auto max-w-96">
        <DropdownMenuGroup>
          <DropdownMenuLabel>
            {user ? m.audit_entity_type_user() : m.audit_entity_type_machine()}
          </DropdownMenuLabel>
          {filter && (
            <DropdownMenuItem render={<Link {...filter} />}>
              <Funnel />
              <span className="truncate">
                {m.audit_entity_filter({ name: entity.name })}
              </span>
            </DropdownMenuItem>
          )}
          {open && (
            <DropdownMenuItem render={<Link {...open} />}>
              <ExternalLink />
              <span className="truncate">
                {m.route_go_to({ page: entity.name })}
              </span>
            </DropdownMenuItem>
          )}
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
