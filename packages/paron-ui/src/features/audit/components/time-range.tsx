import { cn } from "cn";
import { addDays, isEqual, startOfDay, subDays } from "date-fns";
import { CalendarDays, X } from "lucide-react";
import { useState } from "react";
import type { DateRange } from "react-day-picker";

import { Button } from "@/components/shadcn/button";
import { Calendar } from "@/components/shadcn/calendar";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/shadcn/popover";
import type { AuditFilters } from "@/features/audit/lib/api";
import { m } from "@/generated/paraglide/messages";
import { useNow } from "@/hooks/now";
import {
  formatDate,
  formatDateRange,
  formatDateTime,
  formatDateTimeRange,
} from "@/lib/format";

type Range = Pick<AuditFilters, "from" | "to">;

export function TimeRange({
  from,
  to,
  onChange,
}: Range & { onChange: (range: Range) => void }) {
  const [open, setOpen] = useState(false);
  const [days, setDays] = useState<DateRange | undefined>();
  const today = useNow((now) => startOfDay(now).getTime());
  const label = rangeLabel(from, to, today);

  return (
    <div className="flex">
      <Popover
        open={open}
        onOpenChange={(nextOpen) => {
          if (nextOpen)
            setDays(
              from && to
                ? { from: new Date(from), to: lastDay(to) }
                : undefined,
            );
          setOpen(nextOpen);
        }}
      >
        <PopoverTrigger
          render={
            <Button
              variant="outline"
              size="sm"
              className={cn((from || to) && "rounded-r-none")}
            />
          }
          aria-label={`${m.audit_range_label()}: ${label}`}
        >
          <CalendarDays />
          {label}
        </PopoverTrigger>
        <PopoverContent align="start" className="w-auto gap-2">
          <Calendar
            mode="range"
            selected={days}
            onSelect={setDays}
            resetOnSelect
            fixedWeeks
            defaultMonth={days?.from}
            endMonth={new Date(today)}
            disabled={{ after: new Date(today) }}
          />
          <Button
            size="sm"
            className="justify-self-end"
            disabled={!days?.from}
            onClick={() => {
              if (!days?.from) return;
              setOpen(false);
              onChange({
                from: startOfDay(days.from).toISOString(),
                to: addDays(startOfDay(days.to ?? days.from), 1).toISOString(),
              });
            }}
          >
            {m.audit_range_apply()}
          </Button>
        </PopoverContent>
      </Popover>
      {(from || to) && (
        <Button
          variant="outline"
          size="icon-sm"
          className="rounded-l-none border-l-0"
          aria-label={m.audit_range_clear()}
          onClick={() => onChange({ from: undefined, to: undefined })}
        >
          <X />
        </Button>
      )}
    </div>
  );
}

// The API excludes `to`: a range that ends at midnight ends on the day before.
function lastDay(to: string) {
  return isMidnight(to) ? subDays(to, 1) : new Date(to);
}

function isMidnight(iso: string) {
  return isEqual(startOfDay(iso), new Date(iso));
}

// Ends at local midnight are whole days. Other ends, from a link made in another time zone or by
// hand, show their exact time, so that the label never shows days that the filter does not cover.
function rangeLabel(
  from: string | undefined,
  to: string | undefined,
  today: number,
) {
  if ([from, to].every((end) => end === undefined || isMidnight(end))) {
    if (from && to) return formatDateRange(new Date(from), lastDay(to));
    if (from) return formatDateRange(new Date(from), new Date(today));
    if (to) return m.audit_range_until({ to: formatDate(lastDay(to)) });
    return m.audit_range_any();
  }
  if (from && to) return formatDateTimeRange(from, to);
  if (from) return m.audit_range_since({ from: formatDateTime(from) });
  if (to) return m.audit_range_until({ to: formatDateTime(to) });
  return m.audit_range_any();
}
