import { cn } from "cn";
import { ClockArrowDown, ClockArrowUp, TextSearch } from "lucide-react";
import type { ReactNode } from "react";

import { Button } from "@/components/shadcn/button";
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/shadcn/empty";
import { RoleFilter } from "@/features/audit/components/role-filter";
import { TimeRange } from "@/features/audit/components/time-range";
import { AuditTimeline } from "@/features/audit/components/timeline";
import type { EntityActions } from "@/features/audit/components/timeline/entry";
import type { AuditEntries } from "@/features/audit/hooks/audit-entries";
import type { AuditFilters } from "@/features/audit/lib/api";
import { m } from "@/generated/paraglide/messages";

export function ActivityLog({
  filters,
  defaultFilters,
  onFiltersChange,
  pending,
  auditEntries,
  entityActions,
  entityPicker,
}: {
  filters: AuditFilters;
  defaultFilters: AuditFilters;
  onFiltersChange: (filters: AuditFilters) => void;
  pending: boolean;
  auditEntries: AuditEntries;
  entityActions: EntityActions;
  entityPicker?: ReactNode;
}) {
  const { entity, from, to, order } = filters;
  const filtered =
    from !== defaultFilters.from ||
    to !== defaultFilters.to ||
    entity?.id !== defaultFilters.entity?.id ||
    entity?.role !== defaultFilters.entity?.role;

  return (
    <div className="flex flex-col gap-6">
      <div className="flex items-start gap-2">
        <div className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
          {entityPicker}
          {entity && (
            <RoleFilter
              role={entity.role}
              onChange={(role) =>
                onFiltersChange({ ...filters, entity: { ...entity, role } })
              }
            />
          )}
          <TimeRange
            from={from}
            to={to}
            onChange={(range) => onFiltersChange({ ...filters, ...range })}
          />
        </div>
        <Button
          variant="outline"
          size="sm"
          className="shrink-0"
          title={m.audit_order_change()}
          onClick={() =>
            onFiltersChange({
              ...filters,
              order: order === "newest_first" ? "oldest_first" : "newest_first",
            })
          }
        >
          {order === "newest_first" ? <ClockArrowDown /> : <ClockArrowUp />}
          <span className="max-sm:sr-only">
            {order === "newest_first"
              ? m.audit_order_newest_first()
              : m.audit_order_oldest_first()}
          </span>
        </Button>
      </div>

      <div
        aria-busy={pending}
        className={cn("transition-opacity", pending && "opacity-60")}
      >
        {auditEntries.entries.length === 0 ? (
          <Empty className="border">
            <EmptyHeader>
              <EmptyMedia variant="icon">
                <TextSearch />
              </EmptyMedia>
              <EmptyTitle>{m.audit_empty_title()}</EmptyTitle>
              <EmptyDescription>
                {filtered
                  ? m.audit_empty_filtered_description()
                  : m.audit_empty_description()}
              </EmptyDescription>
            </EmptyHeader>
            {filtered && (
              <EmptyContent>
                <Button
                  variant="outline"
                  onClick={() => onFiltersChange({ ...defaultFilters, order })}
                >
                  {m.audit_filter_clear()}
                </Button>
              </EmptyContent>
            )}
          </Empty>
        ) : (
          <AuditTimeline
            auditEntries={auditEntries}
            entityActions={entityActions}
          />
        )}
      </div>
    </div>
  );
}
