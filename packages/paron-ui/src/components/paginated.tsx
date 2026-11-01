import { createLink, type LinkOptions } from "@tanstack/react-router";
import { cn } from "cn";
import { ChevronLeft, ChevronRight, type LucideIcon } from "lucide-react";
import type { ReactNode } from "react";

import {
  Empty,
  EmptyContent,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/shadcn/empty";
import {
  Pagination,
  PaginationContent,
  PaginationItem,
  PaginationLink,
} from "@/components/shadcn/pagination";
import { m } from "@/generated/paraglide/messages";
import { pageOfQuery } from "@/lib/pagination";

export function Paginated({
  limit,
  offset,
  total,
  link,
  filters,
  empty,
  loading,
  children,
}: {
  limit: number;
  offset: number;
  total: number;
  link: (page: number) => LinkOptions;
  filters?: ReactNode;
  empty: {
    Icon: LucideIcon;
    title: string;
    action?: ReactNode;
  };
  loading: boolean;
  children: ReactNode;
}) {
  return (
    <div className="grid grid-cols-1 gap-4 md:grid-cols-[minmax(0,1fr)_auto]">
      {filters && <div className="min-w-0">{filters}</div>}
      {total > limit && (
        <div className="flex justify-end max-md:order-last md:col-start-2">
          <ListPagination
            limit={limit}
            offset={offset}
            total={total}
            link={link}
          />
        </div>
      )}
      {total === 0 ? (
        <Empty className="border md:col-span-2">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <empty.Icon />
            </EmptyMedia>
            <EmptyTitle>{empty.title}</EmptyTitle>
          </EmptyHeader>
          {empty.action && <EmptyContent>{empty.action}</EmptyContent>}
        </Empty>
      ) : (
        <div
          aria-busy={loading}
          className={cn(
            "transition-opacity md:col-span-2",
            loading && "opacity-60",
          )}
        >
          {children}
        </div>
      )}
    </div>
  );
}

// shadcn renders these anchors through Base UI's Button, which gives them role="button".
const PageLink = createLink(PaginationLink);

function ListPagination({
  limit,
  offset,
  total,
  link,
}: {
  limit: number;
  offset: number;
  total: number;
  link: (page: number) => LinkOptions;
}) {
  const page = pageOfQuery({ limit, offset });
  const first = offset + 1;
  const last = Math.min(offset + limit, total);

  return (
    <Pagination aria-label={m.pagination_label()} className="mx-0 w-auto">
      <PaginationContent>
        <PaginationItem className="px-2 text-sm text-muted-foreground tabular-nums">
          {m.pagination_range({ first, last, total })}
        </PaginationItem>
        <PaginationItem>
          <PageLink
            {...link(page - 1)}
            role="link"
            aria-label={m.pagination_previous_label()}
            disabled={page === 1}
            className="aria-disabled:pointer-events-none aria-disabled:opacity-50"
          >
            <ChevronLeft />
          </PageLink>
        </PaginationItem>
        <PaginationItem>
          <PageLink
            {...link(page + 1)}
            role="link"
            aria-label={m.pagination_next_label()}
            disabled={last === total}
            className="aria-disabled:pointer-events-none aria-disabled:opacity-50"
          >
            <ChevronRight />
          </PageLink>
        </PaginationItem>
      </PaginationContent>
    </Pagination>
  );
}
