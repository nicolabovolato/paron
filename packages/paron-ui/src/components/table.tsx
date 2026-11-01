import { Link, type LinkOptions } from "@tanstack/react-router";
import {
  type ReactTable,
  type RowData,
  type TableFeatures,
} from "@tanstack/react-table";
import { cn } from "cn";
import type { ComponentType, ReactNode } from "react";

import {
  ActionsContextMenu,
  ActionsDropDownMenu,
  type ActionGroups,
} from "@/components/actions";
import {
  Table as ShadcnTable,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/shadcn/table";

export type RowActions<T> = ComponentType<{
  item: T;
  children: (actions: { label: string; groups: ActionGroups }) => ReactNode;
}>;

const SIZES = {
  default: {
    frame: "rounded-xl",
    table: "",
    head: "first:pl-5 last:pr-5",
    cell: "first:pl-5 last:pr-5",
  },
  compact: {
    frame: "rounded-lg",
    table: "text-xs",
    head: "h-8 px-3",
    cell: "px-3 py-1.5",
  },
};

const ACTIONS_CELL = "w-0 last:pr-3";

export function Table<TFeatures extends TableFeatures, T extends RowData>({
  table,
  size = "default",
  RowActions,
}: {
  table: ReactTable<TFeatures, T>;
  size?: "default" | "compact";
  RowActions?: RowActions<T>;
}) {
  const sized = SIZES[size];

  return (
    <div className={cn("overflow-hidden border bg-card", sized.frame)}>
      <ShadcnTable className={sized.table}>
        <TableHeader className="bg-muted">
          {table.getHeaderGroups().map((group) => (
            <TableRow key={group.id}>
              {group.headers.map((header) => (
                <TableHead key={header.id} className={sized.head}>
                  {header.isPlaceholder ? null : (
                    <table.FlexRender header={header} />
                  )}
                </TableHead>
              ))}
              {RowActions && (
                <TableHead className={cn(sized.head, ACTIONS_CELL)} />
              )}
            </TableRow>
          ))}
        </TableHeader>
        <TableBody>
          {table.getRowModel().rows.map((row) => {
            const cells = row.getAllCells().map((cell) => (
              <TableCell key={cell.id} className={sized.cell}>
                <table.FlexRender cell={cell} />
              </TableCell>
            ));

            if (!RowActions)
              return (
                <TableRow key={row.id} className="relative align-top">
                  {cells}
                </TableRow>
              );

            return (
              <RowActions key={row.id} item={row.original}>
                {({ label, groups }) => (
                  <ActionsContextMenu
                    groups={groups}
                    render={<TableRow className="relative align-top" />}
                  >
                    {cells}
                    <TableCell
                      className={cn(
                        sized.cell,
                        ACTIONS_CELL,
                        "relative z-10 text-right",
                      )}
                    >
                      <ActionsDropDownMenu label={label} groups={groups} />
                    </TableCell>
                  </ActionsContextMenu>
                )}
              </RowActions>
            );
          })}
        </TableBody>
      </ShadcnTable>
    </div>
  );
}

export function RowLink({
  link,
  children,
}: {
  link: LinkOptions;
  children: ReactNode;
}) {
  return (
    <Link
      {...link}
      className="flex rounded-sm outline-none after:absolute after:inset-0 focus-visible:ring-3 focus-visible:ring-ring/50"
    >
      {children}
    </Link>
  );
}
