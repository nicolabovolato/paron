import { useWindowVirtualizer } from "@tanstack/react-virtual";
import { cn } from "cn";
import { ArrowUp } from "lucide-react";
import {
  useCallback,
  useEffect,
  useEffectEvent,
  useMemo,
  useState,
  type KeyboardEvent,
} from "react";

import { Button } from "@/components/shadcn/button";
import { Spinner } from "@/components/shadcn/spinner";
import {
  Entry,
  type EntityActions,
} from "@/features/audit/components/timeline/entry";
import { timelineItems } from "@/features/audit/components/timeline/items";
import { Rail, Row } from "@/features/audit/components/timeline/layout";
import type { AuditEntries } from "@/features/audit/hooks/audit-entries";
import { m } from "@/generated/paraglide/messages";
import { formatDay } from "@/lib/format";

export function AuditTimeline({
  auditEntries: { entries, moreEntries, loadingMore, loadMoreFailed, loadMore },
  entityActions,
}: {
  auditEntries: AuditEntries;
  entityActions: EntityActions;
}) {
  const LOAD_WHEN_ROWS_LEFT = 10;
  const DAY_HEIGHT = "h-14";

  const items = useMemo(() => timelineItems(entries), [entries]);
  const days = useMemo(
    () =>
      items.flatMap((item, rowIndex) =>
        item.kind === "day" ? [{ ...item, rowIndex }] : [],
      ),
    [items],
  );
  const [expandedEntries, setExpandedEntries] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const [listOffsetTop, setListOffsetTop] = useState(0);
  const [headerHeight, setHeaderHeight] = useState(0);

  const measureList = useCallback((list: HTMLDivElement) => {
    const measure = () => {
      setListOffsetTop(list.getBoundingClientRect().top + window.scrollY);
      setHeaderHeight(
        parseFloat(
          getComputedStyle(list).getPropertyValue("--shell-header-height"),
        ) || 0,
      );
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(document.body);
    return () => observer.disconnect();
  }, []);

  const virtualizer = useWindowVirtualizer({
    count: items.length,
    estimateSize: () => 48,
    getItemKey: (index) => items[index]?.key ?? index,
    overscan: 10,
    scrollMargin: listOffsetTop,
  });
  const virtualRows = virtualizer.getVirtualItems();
  const firstVisibleIndex = virtualizer.range?.startIndex ?? 0;
  const lastVisibleIndex = virtualizer.range?.endIndex ?? 0;
  const headerBottom = (virtualizer.scrollOffset ?? 0) + headerHeight;

  const dayTracks = days.flatMap((day, dayIndex) => {
    const nextDay = days[dayIndex + 1];
    const start = virtualizer.measurementsCache[day.rowIndex]?.start;
    const end =
      nextDay === undefined
        ? listOffsetTop + virtualizer.getTotalSize()
        : virtualizer.measurementsCache[nextDay.rowIndex]?.start;
    if (
      start === undefined ||
      end === undefined ||
      day.rowIndex > lastVisibleIndex ||
      (nextDay !== undefined && nextDay.rowIndex <= firstVisibleIndex)
    )
      return [];
    return [
      {
        key: day.key,
        day: day.day,
        firstDay: dayIndex === 0,
        top: start - listOffsetTop,
        height: end - start,
        pinned: start <= headerBottom && headerBottom < end,
      },
    ];
  });

  const lastRenderedIndex = virtualRows.at(-1)?.index ?? 0;
  const shouldLoadMore =
    lastRenderedIndex >= items.length - 1 - LOAD_WHEN_ROWS_LEFT &&
    moreEntries &&
    !loadingMore &&
    !loadMoreFailed;
  const loadMoreEntries = useEffectEvent(loadMore);
  useEffect(() => {
    if (shouldLoadMore) loadMoreEntries();
  }, [shouldLoadMore]);

  const [entryToFocus, setEntryToFocus] = useState<number>();

  useEffect(() => {
    if (entryToFocus === undefined) return;
    const index = items.findIndex(
      (item) => item.kind === "entry" && item.position === entryToFocus,
    );
    if (index !== -1) virtualizer.scrollToIndex(index, { align: "center" });
  }, [entryToFocus, items, virtualizer]);

  const moveFocusInFeed = (event: KeyboardEvent<HTMLDivElement>) => {
    const direction = { PageDown: 1, PageUp: -1 }[event.key];
    const focusedArticle =
      event.target instanceof Element ? event.target.closest("article") : null;
    if (direction === undefined || focusedArticle === null) return;
    const targetPosition =
      (entryToFocus ?? Number(focusedArticle.getAttribute("aria-posinset"))) +
      direction;
    const targetLoaded =
      targetPosition >= 1 && targetPosition <= entries.length;
    const targetOnNextPage =
      targetPosition === entries.length + 1 && moreEntries && !loadMoreFailed;
    if (!targetLoaded && !targetOnNextPage) return;
    event.preventDefault();
    setEntryToFocus(targetPosition);
  };

  const setEntryExpanded = useCallback(
    (id: string, expanded: boolean) =>
      setExpandedEntries((current) => {
        const next = new Set(current);
        if (expanded) next.add(id);
        else next.delete(id);
        return next;
      }),
    [],
  );

  return (
    <div>
      <div
        ref={measureList}
        role="feed"
        aria-label={m.audit_title()}
        aria-busy={loadingMore}
        onKeyDown={moveFocusInFeed}
        onFocus={() => setEntryToFocus(undefined)}
      >
        <div
          className="relative"
          style={{ height: virtualizer.getTotalSize() }}
        >
          {dayTracks.map((track) => (
            <div
              key={track.key}
              className="pointer-events-none absolute left-0 z-2 w-full"
              style={{ top: track.top, height: track.height }}
            >
              <div
                className={cn(
                  "pointer-events-auto sticky top-[calc(var(--connection-banner-height)+var(--shell-header-height))] bg-background",
                  DAY_HEIGHT,
                )}
              >
                <DayDivider
                  day={track.day}
                  firstDay={track.firstDay}
                  onBackToTop={track.pinned ? scrollToTop : undefined}
                />
              </div>
            </div>
          ))}
          {virtualRows.map((virtualRow) => {
            const item = items[virtualRow.index];
            if (!item) return null;

            return (
              <div
                key={virtualRow.key}
                data-index={virtualRow.index}
                ref={virtualizer.measureElement}
                className="absolute top-0 left-0 w-full"
                style={{
                  transform: `translateY(${virtualRow.start - listOffsetTop}px)`,
                }}
              >
                {item.kind === "day" ? (
                  <div className={DAY_HEIGHT} />
                ) : (
                  <Entry
                    entry={item.entry}
                    minutesAfterPrevious={item.minutesAfterPrevious}
                    lastOfDay={item.lastOfDay}
                    position={item.position}
                    focusRequested={item.position === entryToFocus}
                    expanded={expandedEntries.has(item.entry.id)}
                    onExpandedChange={setEntryExpanded}
                    entityActions={entityActions}
                  />
                )}
              </div>
            );
          })}
        </div>
      </div>
      <Row
        className="min-h-9 items-center"
        rail={
          moreEntries ? (
            <Rail dotted className="mask-b-from-20%" />
          ) : (
            <Rail className="bottom-1/2" />
          )
        }
        marker={
          moreEntries ? undefined : (
            <span className="block size-1.5 rounded-full bg-border" />
          )
        }
      >
        {!moreEntries ? (
          <p className="text-xs text-muted-foreground">{m.audit_end()}</p>
        ) : loadMoreFailed ? (
          <div className="flex items-center gap-3">
            <p className="text-xs text-muted-foreground">
              {m.audit_load_failed()}
            </p>
            <Button variant="outline" size="xs" onClick={loadMore}>
              {m.audit_load_retry()}
            </Button>
          </div>
        ) : (
          <p className="flex items-center gap-2 text-xs text-muted-foreground">
            <Spinner />
            {m.audit_loading_more()}
          </p>
        )}
      </Row>
    </div>
  );
}

function DayDivider({
  day,
  firstDay,
  onBackToTop,
}: {
  day: number;
  firstDay: boolean;
  onBackToTop: (() => void) | undefined;
}) {
  return (
    <Row
      wide
      className="h-full items-center"
      rail={<Rail dotted className={cn(firstDay && "top-1/2")} />}
    >
      <div className="flex items-center">
        <h2 className="rounded-full border bg-background px-3 py-0.5 text-xs font-medium whitespace-nowrap text-muted-foreground">
          {formatDay(day)}
        </h2>
        <span aria-hidden className="h-px flex-1 bg-border" />
        {onBackToTop && (
          <Button
            variant="ghost"
            size="xs"
            className="text-muted-foreground"
            onClick={onBackToTop}
          >
            <ArrowUp />
            {m.audit_back_to_top()}
          </Button>
        )}
      </div>
    </Row>
  );
}

function scrollToTop() {
  window.scrollTo({
    top: 0,
    behavior: matchMedia("(prefers-reduced-motion: reduce)").matches
      ? "auto"
      : "smooth",
  });
}
