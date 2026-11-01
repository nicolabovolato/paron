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
import { SearchX, Users as UsersIcon } from "lucide-react";
import type { ComponentProps } from "react";

import { ActionButton } from "@/components/actions";
import { ListSearch } from "@/components/list-search";
import { Paginated } from "@/components/paginated";
import { Page } from "@/components/route/page";
import { Button } from "@/components/shadcn/button";
import { Item, ItemFooter } from "@/components/shadcn/item";
import { RowLink, Table, type RowActions } from "@/components/table";
import { RoleBadges } from "@/features/users/components/roles";
import { UserBadge } from "@/features/users/components/user-badge";
import { UserStatusBadge } from "@/features/users/components/user-status-badge";
import {
  useUserActions,
  useUsersActions,
} from "@/features/users/hooks/user-actions";
import { userQueries } from "@/features/users/lib/api";
import type { UserResponse } from "@/generated/api/types.gen";
import { zListUsersQuery } from "@/generated/api/zod.gen";
import { m } from "@/generated/paraglide/messages";
import { useNarrowScreen } from "@/hooks/narrow-screen";
import { authQueries } from "@/lib/auth";
import { formatDateTime } from "@/lib/format";
import { pageQuery, pageSearch } from "@/lib/pagination";
import { useRecoveryUrl } from "@/routes/_auth/-recovery-link";

export const Route = createFileRoute("/_authenticated/users/")({
  validateSearch: pageSearch.extend({
    search: zListUsersQuery.shape.search.catch(undefined),
  }),
  search: { middlewares: [stripSearchParams({ page: 1 })] },
  loaderDeps: ({ search: { page, search } }) => ({ page, search }),
  loader: async ({ context, deps }) => {
    await context.queryClient.query({
      ...userQueries.list({ ...pageQuery(deps.page), search: deps.search }),
      staleTime: "static",
    });
  },
  component: RouteComponent,
});

function RouteComponent() {
  const { page, search } = Route.useSearch();
  const navigate = Route.useNavigate();
  const recoveryUrl = useRecoveryUrl();
  const collectionActions = useUsersActions({ recoveryUrl });
  const listQuery = { ...pageQuery(page), search };
  const { data: result } = useSuspenseQuery(userQueries.list(listQuery));
  const table = useTable({
    features: USER_TABLE_FEATURES,
    columns: USER_COLUMNS,
    data: result.items,
    getRowId: (user) => user.id,
  });

  const narrowScreen = useNarrowScreen();

  const matchRoute = useMatchRoute();
  const loading = matchRoute({ to: "/users", pending: true }) !== false;

  const changeSearch = (search: string | undefined) =>
    void navigate({ search: { search }, replace: true });

  return (
    <Page
      title={m.users_list_title()}
      actions={
        collectionActions.add && (
          <ActionButton action={collectionActions.add} variant="default" />
        )
      }
    >
      <Paginated
        loading={loading}
        limit={listQuery.limit}
        offset={listQuery.offset}
        total={result.total}
        link={(page) => linkOptions({ to: "/users", search: { page, search } })}
        filters={
          (search !== undefined || result.total > 0) && (
            <ListSearch
              search={search}
              searchSchema={zListUsersQuery.shape.search}
              onSearchChange={changeSearch}
              placeholder={m.users_search()}
              loading={loading}
            />
          )
        }
        empty={
          search === undefined
            ? { Icon: UsersIcon, title: m.users_empty() }
            : {
                Icon: SearchX,
                title: m.users_none_found({ search }),
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
            {result.items.map((user) => (
              <UserCard key={user.id} user={user} />
            ))}
          </div>
        ) : (
          <Table table={table} RowActions={UserRowActions} />
        )}
      </Paginated>
    </Page>
  );
}

const USER_TABLE_FEATURES = tableFeatures({});

const userColumn = createColumnHelper<
  typeof USER_TABLE_FEATURES,
  UserResponse
>();

const USER_COLUMNS = userColumn.columns([
  userColumn.display({
    id: "user",
    header: () => m.users_column_user(),
    cell: ({ row }) => <UserLink user={row.original} />,
  }),
  userColumn.display({
    id: "roles",
    header: () => m.roles_title(),
    cell: ({ row }) => <RoleBadges roles={row.original.roles} />,
  }),
  userColumn.display({
    id: "status",
    header: () => m.users_column_status(),
    cell: ({ row }) => <UserStatusBadge status={row.original.status} />,
  }),
  userColumn.display({
    id: "created",
    header: () => m.user_created_label(),
    cell: ({ row }) => (
      <span className="text-muted-foreground">
        {formatDateTime(row.original.created_at)}
      </span>
    ),
  }),
]);

function UserRowActions({
  item: user,
  children,
}: ComponentProps<RowActions<UserResponse>>) {
  const recoveryUrl = useRecoveryUrl();
  const actions = useUserActions(user, { recoveryUrl });

  return children({
    label: m.users_row_actions({ username: user.username }),
    groups: [
      [actions.edit, actions.startRecovery, actions.suspend, actions.activate],
      [actions.delete],
    ],
  });
}

function UserLink({ user }: { user: UserResponse }) {
  const { data: me } = useSuspenseQuery(authQueries.me());

  return (
    <RowLink
      link={linkOptions({ to: "/users/$userId", params: { userId: user.id } })}
    >
      <UserBadge user={user} isMe={me.id === user.id} />
    </RowLink>
  );
}

function UserCard({ user }: { user: UserResponse }) {
  const { data: me } = useSuspenseQuery(authQueries.me());

  return (
    <Item
      variant="outline"
      className="active:bg-muted/50"
      render={<Link to="/users/$userId" params={{ userId: user.id }} />}
    >
      <UserBadge user={user} isMe={me.id === user.id} showStatus />
      {user.roles.length > 0 && (
        <ItemFooter>
          <RoleBadges roles={user.roles} />
        </ItemFooter>
      )}
    </Item>
  );
}
