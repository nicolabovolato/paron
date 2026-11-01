import { useSuspenseQuery } from "@tanstack/react-query";
import {
  createFileRoute,
  Link,
  linkOptions,
  stripSearchParams,
  useMatchRoute,
} from "@tanstack/react-router";
import {
  createColumnHelper,
  tableFeatures,
  useTable,
} from "@tanstack/react-table";
import { SearchX, Server } from "lucide-react";
import type { ComponentProps } from "react";

import { ActionButton } from "@/components/actions";
import { ListSearch } from "@/components/list-search";
import { Paginated } from "@/components/paginated";
import { Page } from "@/components/route/page";
import { Button } from "@/components/shadcn/button";
import { Item, ItemContent, ItemTitle } from "@/components/shadcn/item";
import { Spinner } from "@/components/shadcn/spinner";
import { RowLink, Table, type RowActions } from "@/components/table";
import {
  MachineConnectionBadge,
  MachineProtocolsBadge,
} from "@/features/machines/components/machine-badges";
import {
  useMachineActions,
  useMachinesActions,
} from "@/features/machines/hooks/machine-actions";
import {
  machineQueries,
  useIsTestingMachineConnection,
} from "@/features/machines/lib/api";
import type { MachineResponse } from "@/generated/api/types.gen";
import { zListMachinesQuery } from "@/generated/api/zod.gen";
import { m } from "@/generated/paraglide/messages";
import { useNarrowScreen } from "@/hooks/narrow-screen";
import { pageQuery, pageSearch } from "@/lib/pagination";

export const Route = createFileRoute("/_authenticated/machines/")({
  validateSearch: pageSearch.extend({
    search: zListMachinesQuery.shape.search.catch(undefined),
  }),
  search: { middlewares: [stripSearchParams({ page: 1 })] },
  loaderDeps: ({ search: { page, search } }) => ({ page, search }),
  loader: async ({ context, deps }) => {
    await context.queryClient.query({
      ...machineQueries.list({ ...pageQuery(deps.page), search: deps.search }),
      staleTime: "static",
    });
  },
  component: RouteComponent,
});

function RouteComponent() {
  const { page, search } = Route.useSearch();
  const navigate = Route.useNavigate();
  const collectionActions = useMachinesActions();
  const listQuery = { ...pageQuery(page), search };
  const { data: result } = useSuspenseQuery(machineQueries.list(listQuery));
  const table = useTable({
    features: MACHINE_TABLE_FEATURES,
    columns: MACHINE_TABLE_COLUMNS,
    data: result.items,
    getRowId: (machine) => machine.id,
  });

  const narrowScreen = useNarrowScreen();

  const matchRoute = useMatchRoute();
  const loading = matchRoute({ to: "/machines", pending: true }) !== false;

  const changeSearch = (search: string | undefined) =>
    void navigate({ search: { search }, replace: true });

  return (
    <Page
      title={m.machines_list_title()}
      actions={
        collectionActions.create && (
          <ActionButton action={collectionActions.create} variant="default" />
        )
      }
    >
      <Paginated
        loading={loading}
        limit={listQuery.limit}
        offset={listQuery.offset}
        total={result.total}
        link={(page) =>
          linkOptions({ to: "/machines", search: { page, search } })
        }
        filters={
          (search !== undefined || result.total > 0) && (
            <ListSearch
              search={search}
              searchSchema={zListMachinesQuery.shape.search}
              onSearchChange={changeSearch}
              placeholder={m.machines_list_search()}
              loading={loading}
            />
          )
        }
        empty={
          search === undefined
            ? { Icon: Server, title: m.machines_list_empty() }
            : {
                Icon: SearchX,
                title: m.machines_list_none_found({ search }),
                action: (
                  <Button
                    variant="outline"
                    onClick={() => changeSearch(undefined)}
                  >
                    {m.list_search_clear()}
                  </Button>
                ),
              }
        }
      >
        {narrowScreen ? (
          <div className="flex flex-col gap-3">
            {result.items.map((machine) => (
              <MachineCard key={machine.id} machine={machine} />
            ))}
          </div>
        ) : (
          <Table table={table} RowActions={MachineRowActions} />
        )}
      </Paginated>
    </Page>
  );
}

const MACHINE_TABLE_FEATURES = tableFeatures({});

const machineColumn = createColumnHelper<
  typeof MACHINE_TABLE_FEATURES,
  MachineResponse
>();

const MACHINE_TABLE_COLUMNS = machineColumn.columns([
  machineColumn.display({
    id: "name",
    header: () => m.machine_field_name(),
    cell: ({ row }) => (
      <RowLink
        link={linkOptions({
          to: "/machines/$machineId",
          params: { machineId: row.original.id },
        })}
      >
        <span className="font-medium wrap-anywhere whitespace-normal">
          {row.original.name}
        </span>
      </RowLink>
    ),
  }),
  machineColumn.display({
    id: "connection",
    header: () => m.machine_field_connection(),
    cell: ({ row }) => <MachineConnection machine={row.original} />,
  }),
  machineColumn.display({
    id: "protocol",
    header: () => m.machine_field_protocol(),
    cell: ({ row }) => (
      <MachineProtocolsBadge protocols={row.original.protocols} />
    ),
  }),
]);

function MachineRowActions({
  item: machine,
  children,
}: ComponentProps<RowActions<MachineResponse>>) {
  const actions = useMachineActions(machine);

  return children({
    label: m.machines_list_row_actions({ name: machine.name }),
    groups: [
      [actions.openTerminal, actions.testConnection],
      [
        actions.edit,
        actions.setUpTunnel,
        actions.cancelTunnelSetup,
        actions.revokeTunnelAccess,
      ],
      [actions.delete],
    ],
  });
}

function MachineConnection({ machine }: { machine: MachineResponse }) {
  const testing = useIsTestingMachineConnection(machine.id);

  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
      <MachineConnectionBadge connectivity={machine.connectivity} />
      {testing && (
        <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <Spinner />
          {m.connection_test_pending()}
        </span>
      )}
    </div>
  );
}

function MachineCard({ machine }: { machine: MachineResponse }) {
  return (
    <Item
      variant="outline"
      className="active:bg-muted/50"
      render={
        <Link to="/machines/$machineId" params={{ machineId: machine.id }} />
      }
    >
      <ItemContent className="gap-2">
        <ItemTitle>{machine.name}</ItemTitle>
        <MachineConnectionBadge connectivity={machine.connectivity} />
        <MachineProtocolsBadge protocols={machine.protocols} />
      </ItemContent>
    </Item>
  );
}
