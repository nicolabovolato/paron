import { differenceInMinutes, isSameDay, startOfDay } from "date-fns";

import type { AuditEntryResponse } from "@/generated/api/types.gen";

export type TimelineItem =
  | { kind: "day"; key: string; day: number }
  | {
      kind: "entry";
      key: string;
      entry: AuditEntryResponse;
      minutesAfterPrevious: number;
      lastOfDay: boolean;
      position: number;
    };

export function timelineItems(entries: AuditEntryResponse[]) {
  return entries.flatMap((entry, index) => {
    const previous = entries[index - 1];
    const next = entries[index + 1];
    const newDay =
      previous === undefined ||
      !isSameDay(previous.created_at, entry.created_at);
    const day = startOfDay(entry.created_at).getTime();
    const item = {
      kind: "entry",
      key: entry.id,
      entry,
      minutesAfterPrevious: newDay
        ? 0
        : Math.abs(differenceInMinutes(entry.created_at, previous.created_at)),
      lastOfDay:
        next !== undefined && !isSameDay(next.created_at, entry.created_at),
      position: index + 1,
    } satisfies TimelineItem;

    return newDay
      ? [{ kind: "day", key: `day-${day}`, day } satisfies TimelineItem, item]
      : [item];
  });
}
