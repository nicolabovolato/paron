import {
  createColumnHelper,
  tableFeatures,
  useTable,
} from "@tanstack/react-table";
import { cn } from "cn";

import { Table } from "@/components/table";
import type { DetailRow } from "@/features/audit/lib/details";
import { m } from "@/generated/paraglide/messages";

const DETAIL_TABLE_FEATURES = tableFeatures({});

const detailColumn = createColumnHelper<
  typeof DETAIL_TABLE_FEATURES,
  DetailRow
>();

const labelColumn = detailColumn.display({
  id: "label",
  header: () => m.audit_column_field(),
  cell: ({ row }) => (
    <span className="text-muted-foreground">{row.original.label}</span>
  ),
});

const CREATED_COLUMNS = detailColumn.columns([
  labelColumn,
  detailColumn.display({
    id: "value",
    header: () => m.audit_column_value(),
    cell: ({ row }) => (
      <DetailValue
        text={row.original.after}
        monospace={row.original.monospace}
      />
    ),
  }),
]);

const CHANGED_COLUMNS = detailColumn.columns([
  labelColumn,
  detailColumn.display({
    id: "before",
    header: () => m.audit_column_before(),
    cell: ({ row }) => (
      <span className="text-muted-foreground">
        <DetailValue
          text={row.original.before}
          monospace={row.original.monospace}
        />
      </span>
    ),
  }),
  detailColumn.display({
    id: "after",
    header: () => m.audit_column_after(),
    cell: ({ row }) => (
      <DetailValue
        text={row.original.after}
        monospace={row.original.monospace}
      />
    ),
  }),
]);

export function DetailsTable({ rows }: { rows: DetailRow[] }) {
  const table = useTable({
    features: DETAIL_TABLE_FEATURES,
    columns: rows.some((row) => row.before !== undefined)
      ? CHANGED_COLUMNS
      : CREATED_COLUMNS,
    data: rows,
    getRowId: (row) => row.key,
  });

  return <Table table={table} size="compact" />;
}

function DetailValue({
  text,
  monospace,
}: {
  text: string | undefined;
  monospace?: boolean;
}) {
  return text === undefined ? (
    <span className="text-muted-foreground">{m.audit_value_not_set()}</span>
  ) : (
    <span
      className={cn(
        "whitespace-normal",
        monospace ? "font-mono wrap-anywhere" : "wrap-break-word",
      )}
    >
      {text}
    </span>
  );
}
