import { keepPreviousData, useQuery } from "@tanstack/react-query";
import {
  createFileRoute,
  linkOptions,
  stripSearchParams,
  useNavigate,
} from "@tanstack/react-router";
import { cn } from "cn";
import { ChevronDown, Funnel, Server, User, X } from "lucide-react";
import { useRef, useState } from "react";
import { z } from "zod";

import { useOptimisticSearch } from "@/hooks/optimistic-search";
import { useSearchInput } from "@/hooks/search-input";

import { Page } from "@/components/route/page";
import { Button } from "@/components/shadcn/button";
import {
  Combobox,
  ComboboxEmpty,
  ComboboxInput,
  ComboboxItem,
  ComboboxList,
} from "@/components/shadcn/combobox";
import { InputGroupAddon } from "@/components/shadcn/input-group";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/shadcn/popover";
import { Spinner } from "@/components/shadcn/spinner";
import { ActivityLog } from "@/features/audit/components/activity-log";
import { useAuditEntries } from "@/features/audit/hooks/audit-entries";
import type { AuditFilters } from "@/features/audit/lib/api";
import { auditQueries } from "@/features/audit/lib/api";
import { machineQueries } from "@/features/machines/lib/api";
import { userQueries } from "@/features/users/lib/api";
import {
  zListAuditEntriesQuery,
  zListMachinesQuery,
  zListUsersQuery,
} from "@/generated/api/zod.gen";
import { m } from "@/generated/paraglide/messages";
import { usePermissions } from "@/hooks/permissions";
import { useEntityPage } from "@/routes/_authenticated/activity/-entity-page";
import {
  ACTIVITY_SEARCH_DEFAULTS,
  activitySearch,
} from "@/routes/_authenticated/activity/-search";

const search = activitySearch.extend({
  entity: zListAuditEntriesQuery.shape.entity
    .unwrap()
    .pick({ type: true, id: true })
    .extend({ name: z.string().optional() })
    .optional()
    .catch(undefined),
});

export const Route = createFileRoute("/_authenticated/activity/")({
  staticData: { width: "narrow" },
  validateSearch: search,
  search: { middlewares: [stripSearchParams(ACTIVITY_SEARCH_DEFAULTS)] },
  loaderDeps: ({ search }) => search,
  loader: async ({ context, deps }) => {
    await context.queryClient.infiniteQuery({
      ...auditQueries.list(queryFromSearch(deps)),
      staleTime: "static",
    });
  },
  component: RouteComponent,
});

type Search = z.output<typeof search>;
type EntityFilter = NonNullable<Search["entity"]>;

function queryFromSearch({ entity, role, ...search }: Search) {
  return {
    ...search,
    entity: entity && { type: entity.type, id: entity.id, role },
  } satisfies AuditFilters;
}

function searchFromQuery(
  { entity, from, to, order }: AuditFilters,
  previous: Search,
) {
  return {
    from,
    to,
    order,
    role: entity?.role ?? ACTIVITY_SEARCH_DEFAULTS.role,
    entity: entity && {
      type: entity.type,
      id: entity.id,
      name:
        entity.id === previous.entity?.id ? previous.entity.name : undefined,
    },
  } satisfies Search;
}

function RouteComponent() {
  const search = Route.useSearch();
  const navigate = useNavigate({ from: Route.fullPath });
  const { shownSearch, changeSearch, pending } = useOptimisticSearch(
    search,
    (nextSearch) => navigate({ search: nextSearch }),
  );
  const entityPage = useEntityPage();
  const query = queryFromSearch(search);
  const auditEntries = useAuditEntries(query);

  return (
    <Page title={m.audit_title()}>
      <ActivityLog
        filters={queryFromSearch(shownSearch)}
        defaultFilters={queryFromSearch(ACTIVITY_SEARCH_DEFAULTS)}
        onFiltersChange={(filters) =>
          changeSearch(searchFromQuery(filters, shownSearch))
        }
        pending={pending}
        auditEntries={auditEntries}
        entityActions={(entity) => ({
          filter:
            entity.id === shownSearch.entity?.id
              ? undefined
              : linkOptions({
                  to: "/activity",
                  search: {
                    ...shownSearch,
                    entity: {
                      type: entity.type,
                      id: entity.id,
                      name: entity.name,
                    },
                    role: ACTIVITY_SEARCH_DEFAULTS.role,
                  },
                }),
          open: entityPage(entity),
        })}
        entityPicker={
          <EntityPicker
            value={shownSearch.entity}
            onChange={(entity) =>
              changeSearch({
                ...shownSearch,
                entity,
                role: ACTIVITY_SEARCH_DEFAULTS.role,
              })
            }
          />
        }
      />
    </Page>
  );
}

type EntityType = EntityFilter["type"];

const RESULT_LIMIT = 10;

const ENTITY_TYPES = {
  user: {
    permission: "user:read",
    Icon: User,
    typeName: m.audit_entity_type_user,
    searchPlaceholder: m.audit_entity_search_users,
    searchSchema: zListUsersQuery.shape.search,
    countText: m.audit_entity_count_users,
  },
  machine: {
    permission: "machine:read",
    Icon: Server,
    typeName: m.audit_entity_type_machine,
    searchPlaceholder: m.audit_entity_search_machines,
    searchSchema: zListMachinesQuery.shape.search,
    countText: m.audit_entity_count_machines,
  },
} as const;

function useEntityNameSearch(type: EntityType, search: string | undefined) {
  const users = useQuery({
    ...userQueries.list({ search, limit: RESULT_LIMIT }),
    enabled: type === "user",
    placeholderData: keepPreviousData,
    select: ({ items, total }) => ({
      total,
      entities: items.map(
        (user) =>
          ({
            type: "user",
            id: user.id,
            name: user.username,
          }) satisfies EntityFilter,
      ),
    }),
  });
  const machines = useQuery({
    ...machineQueries.list({ search, limit: RESULT_LIMIT }),
    enabled: type === "machine",
    placeholderData: keepPreviousData,
    select: ({ items, total }) => ({
      total,
      entities: items.map(
        (machine) =>
          ({
            type: "machine",
            id: machine.id,
            name: machine.name,
          }) satisfies EntityFilter,
      ),
    }),
  });

  return type === "user" ? users : machines;
}

function EntityPicker({
  value,
  onChange,
}: {
  value: EntityFilter | undefined;
  onChange: (entity: EntityFilter | undefined) => void;
}) {
  const permissions = usePermissions();
  const trigger = useRef<HTMLButtonElement>(null);
  const [popupOpen, setPopupOpen] = useState(false);
  const [searchType, setSearchType] = useState<EntityType>();

  const readableTypes = zListAuditEntriesQuery.shape.entity
    .unwrap()
    .shape.type.options.filter((type) =>
      permissions.has(ENTITY_TYPES[type].permission),
    );
  if (readableTypes.length === 0) return null;

  const entityType = value && ENTITY_TYPES[value.type];

  return (
    <div className="flex min-w-0">
      <Popover
        open={popupOpen}
        onOpenChange={(open) => {
          if (open)
            setSearchType(
              value?.type ??
                (readableTypes.length === 1 ? readableTypes[0] : undefined),
            );
          setPopupOpen(open);
        }}
      >
        <PopoverTrigger
          render={
            <Button
              ref={trigger}
              variant="outline"
              size="sm"
              className={cn("min-w-0 shrink", value && "rounded-r-none")}
            />
          }
        >
          {value && entityType ? (
            <>
              <entityType.Icon />
              <span className="text-muted-foreground">
                {entityType.typeName()}
              </span>{" "}
              <span className="truncate" title={value.name}>
                {value.name ?? value.id}
              </span>
            </>
          ) : (
            <>
              <Funnel />
              {m.audit_entity_filter_by()}
              <ChevronDown />
            </>
          )}
        </PopoverTrigger>
        <PopoverContent align="start" className="w-72 gap-0 p-1">
          {searchType ? (
            <EntitySearch
              key={searchType}
              type={searchType}
              onChoose={(entity) => {
                setPopupOpen(false);
                onChange(entity);
              }}
            />
          ) : (
            readableTypes.map((type) => {
              const { Icon, typeName } = ENTITY_TYPES[type];
              return (
                <Button
                  key={type}
                  variant="ghost"
                  size="sm"
                  className="justify-start"
                  onClick={() => setSearchType(type)}
                >
                  <Icon />
                  {typeName()}
                </Button>
              );
            })
          )}
        </PopoverContent>
      </Popover>
      {value && (
        <Button
          variant="outline"
          size="icon-sm"
          className="rounded-l-none border-l-0"
          aria-label={m.audit_entity_clear()}
          onClick={() => {
            onChange(undefined);
            trigger.current?.focus();
          }}
        >
          <X />
        </Button>
      )}
    </div>
  );
}

function EntitySearch({
  type,
  onChoose,
}: {
  type: EntityType;
  onChoose: (entity: EntityFilter) => void;
}) {
  const { Icon, searchPlaceholder, searchSchema, countText } =
    ENTITY_TYPES[type];
  const [search, setSearch] = useState<string>();
  const { changeInputText, pending } = useSearchInput(
    search,
    setSearch,
    searchSchema,
  );
  const results = useEntityNameSearch(type, search);
  const searching = pending || results.isFetching;

  return (
    <Combobox
      items={results.data?.entities ?? []}
      filter={null}
      autoHighlight
      inline
      open
      value={null}
      onValueChange={(entity: EntityFilter | null) => {
        if (entity) onChoose(entity);
      }}
      onInputValueChange={(text) => changeInputText(text)}
      itemToStringLabel={(entity: EntityFilter) => entity.name ?? entity.id}
      isItemEqualToValue={(item: EntityFilter, selected: EntityFilter) =>
        item.id === selected.id
      }
    >
      <ComboboxInput
        autoFocus
        aria-label={searchPlaceholder()}
        placeholder={searchPlaceholder()}
        showTrigger={false}
        className="w-full"
      >
        <InputGroupAddon align="inline-start">
          {searching ? <Spinner /> : <Icon />}
        </InputGroupAddon>
      </ComboboxInput>
      {results.data ? (
        <ComboboxEmpty>{m.audit_entity_none()}</ComboboxEmpty>
      ) : (
        <div className="flex justify-center py-6">
          <Spinner />
        </div>
      )}
      <ComboboxList>
        {(entity: EntityFilter) => (
          <ComboboxItem key={entity.id} value={entity}>
            <Icon />
            <span className="truncate" title={entity.name}>
              {entity.name}
            </span>
          </ComboboxItem>
        )}
      </ComboboxList>
      {results.data && results.data.total > results.data.entities.length && (
        <p className="border-t px-3 py-2 text-xs text-muted-foreground tabular-nums">
          {countText({
            listed: results.data.entities.length,
            total: results.data.total,
          })}
        </p>
      )}
    </Combobox>
  );
}
