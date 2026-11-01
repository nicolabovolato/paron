import { queryOptions, useSuspenseQuery } from "@tanstack/react-query";
import { createFileRoute, Link, linkOptions } from "@tanstack/react-router";
import { cn } from "cn";
import { Check, KeyRound, Server, SquareTerminal } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";

import paronUrl from "@/assets/paron.svg";
import { ActionButton } from "@/components/actions";
import { Page } from "@/components/route/page";
import { Button, buttonVariants } from "@/components/shadcn/button";
import { Card } from "@/components/shadcn/card";
import {
  useMachineActions,
  useMachinesActions,
} from "@/features/machines/hooks/machine-actions";
import { machineQueries } from "@/features/machines/lib/api";
import { useUsersActions } from "@/features/users/hooks/user-actions";
import { userQueries } from "@/features/users/lib/api";
import { listAuditEntriesOptions } from "@/generated/api/@tanstack/react-query.gen";
import type {
  MachineResponse,
  ReadSessionResponse,
} from "@/generated/api/types.gen";
import { m } from "@/generated/paraglide/messages";
import { usePermissions } from "@/hooks/permissions";
import { authQueries } from "@/lib/auth";
import { randomSaying } from "@/lib/cowsay";
import { useRecoveryUrl } from "@/routes/_auth/-recovery-link";

const setupProgressQuery = (me: ReadSessionResponse) =>
  queryOptions({
    queryKey: ["setup-progress", me.id, me.permissions],
    queryFn: async ({ client }) => {
      const permissions = new Set(me.permissions);
      const [credentials, machines, openedTerminals, users] = await Promise.all(
        [
          client.query(authQueries.credentials()),
          permissions.has("machine:read")
            ? client.query(machineQueries.list({ limit: 1 }))
            : undefined,
          permissions.has("machine:operate")
            ? client.query(
                listAuditEntriesOptions({
                  query: { type: "machine.terminal_opened", limit: 1 },
                }),
              )
            : undefined,
          permissions.has("user:read")
            ? client.query(userQueries.list({ limit: 1 }))
            : undefined,
        ],
      );

      return {
        accountSecured:
          credentials.totp != null || credentials.passkeys.length > 0,
        firstMachine: machines?.items[0],
        terminalOpened: (openedTerminals?.items.length ?? 0) > 0,
        teammateAdded: (users?.total ?? 0) > 1,
      };
    },
  });

export const Route = createFileRoute("/_authenticated/")({
  loader: async ({ context: { queryClient } }) => {
    const me = await queryClient.query({
      ...authQueries.me(),
      staleTime: "static",
    });
    const setupEnd = storedSetupEnd(me.id);
    if (!setupEnd) await queryClient.query(setupProgressQuery(me));

    return { breadcrumb: m.overview_title(), setupEnd };
  },
  component: RouteComponent,
});

type SetupEnd = "done" | "skipped";

const setupEndKey = (userId: string) => `paron-setup:${userId}`;

function storedSetupEnd(userId: string) {
  try {
    const stored = localStorage.getItem(setupEndKey(userId));
    return stored === "done" || stored === "skipped" ? stored : undefined;
  } catch {
    return undefined;
  }
}

function storeSetupEnd(userId: string, setupEnd: SetupEnd | undefined) {
  try {
    if (setupEnd) localStorage.setItem(setupEndKey(userId), setupEnd);
    else localStorage.removeItem(setupEndKey(userId));
  } catch {
    // Storage is blocked; the setup shows again at the next visit.
  }
}

function RouteComponent() {
  const loaderData = Route.useLoaderData();
  const [setupEnd, setSetupEnd] = useState(loaderData.setupEnd);
  const { data: me } = useSuspenseQuery(authQueries.me());

  const endSetup = (end: SetupEnd | undefined) => {
    storeSetupEnd(me.id, end);
    setSetupEnd(end);
  };

  if (setupEnd === "done") return <SetupComplete />;
  if (setupEnd === "skipped")
    return <SetupComplete onShowSetup={() => endSetup(undefined)} />;

  return <Setup me={me} onSkip={() => endSetup("skipped")} />;
}

type StepName = "account" | "machine" | "terminal" | "teammate";

function Setup({
  me,
  onSkip,
}: {
  me: ReadSessionResponse;
  onSkip: () => void;
}) {
  const { data: progress } = useSuspenseQuery(setupProgressQuery(me));
  const permissions = usePermissions();
  const createMachine = useMachinesActions().create;
  const addUser = useUsersActions({ recoveryUrl: useRecoveryUrl() }).add;
  const canOpenTerminal = permissions.has("machine:operate");
  const { firstMachine } = progress;

  const done: Record<StepName, boolean> = {
    account: progress.accountSecured,
    machine: firstMachine !== undefined,
    terminal: progress.terminalOpened,
    teammate: progress.teammateAdded,
  };
  const shown = (
    [
      "account",
      createMachine && "machine",
      canOpenTerminal && "terminal",
      addUser && "teammate",
    ] as const
  ).filter((name) => name !== undefined && name !== false);
  const next = shown.find((name) => !done[name]);

  useEffect(() => {
    if (!next) storeSetupEnd(me.id, "done");
  }, [next, me.id]);

  if (!next) return <SetupComplete />;

  const step = (name: StepName) =>
    ({
      number: shown.indexOf(name) + 1,
      state: done[name] ? "done" : name === next ? "next" : "open",
    }) as const;
  const variant = (name: StepName) => (name === next ? "default" : "outline");

  return (
    <Page title={m.overview_title()}>
      <div className="grid grid-cols-1 gap-8 md:grid-cols-[minmax(0,1fr)_12.5rem]">
        <section className="flex min-w-0 flex-col gap-3">
          <div className="flex items-center justify-between gap-3">
            <h2 className="font-semibold">{m.overview_setup_title()}</h2>
            <Button variant="ghost" size="sm" onClick={onSkip}>
              {m.overview_setup_skip()}
            </Button>
          </div>
          <SetupProgress
            done={shown.filter((name) => done[name]).length}
            total={shown.length}
          />

          <Card className="gap-0 py-0">
            <ol className="flex flex-col divide-y">
              <SetupStep
                {...step("account")}
                title={m.overview_step_account_title()}
                description={m.overview_step_account_description()}
              >
                <ActionButton
                  variant={variant("account")}
                  action={{
                    label: m.shell_my_account(),
                    Icon: KeyRound,
                    kind: "link",
                    link: linkOptions({ to: "/account" }),
                  }}
                />
              </SetupStep>

              {createMachine && (
                <SetupStep
                  {...step("machine")}
                  title={m.overview_step_machine_title()}
                  description={m.overview_step_machine_description()}
                >
                  <ActionButton
                    variant={variant("machine")}
                    action={createMachine}
                  />
                </SetupStep>
              )}

              {canOpenTerminal && (
                <SetupStep
                  {...step("terminal")}
                  title={m.overview_step_terminal_title()}
                  description={m.overview_step_terminal_description()}
                >
                  {firstMachine ? (
                    <OpenTerminalButton
                      machine={firstMachine}
                      variant={variant("terminal")}
                    />
                  ) : (
                    <ActionButton
                      variant={variant("terminal")}
                      action={{
                        label: m.machine_action_open_terminal(),
                        Icon: SquareTerminal,
                        kind: "perform",
                        perform: () => false,
                        disabled: {
                          reason: m.overview_step_terminal_needs_machine(),
                        },
                      }}
                    />
                  )}
                </SetupStep>
              )}

              {addUser && (
                <SetupStep
                  {...step("teammate")}
                  title={m.overview_step_teammate_title()}
                  description={m.overview_step_teammate_description()}
                >
                  <ActionButton
                    variant={variant("teammate")}
                    action={addUser}
                  />
                </SetupStep>
              )}
            </ol>
          </Card>
        </section>

        <div className="flex items-center gap-4 md:flex-col md:pt-1 md:text-center">
          <img src={paronUrl} alt="" className="size-14 shrink-0 md:size-32" />
          <Saying />
        </div>
      </div>
    </Page>
  );
}

function SetupProgress({ done, total }: { done: number; total: number }) {
  return (
    <div className="flex items-center gap-3 text-sm text-muted-foreground tabular-nums">
      {m.overview_setup_progress({ done, total })}
      <div
        aria-hidden
        className="h-1 max-w-56 flex-1 overflow-hidden rounded-full bg-secondary"
      >
        <div
          className="h-full bg-primary transition-[width] duration-500"
          style={{ width: `${(done / total) * 100}%` }}
        />
      </div>
    </div>
  );
}

function SetupStep({
  number,
  state,
  title,
  description,
  children,
}: {
  number: number;
  state: "done" | "next" | "open";
  title: string;
  description: string;
  children: ReactNode;
}) {
  return (
    <li className="flex flex-wrap items-center gap-x-3 gap-y-2 px-4 py-3.5">
      <span
        className={cn(
          "grid size-6 shrink-0 place-items-center rounded-full border text-xs font-semibold tabular-nums",
          state === "done" &&
            "border-primary bg-primary text-primary-foreground",
          state === "next" && "border-primary text-primary",
          state === "open" && "border-input text-muted-foreground",
        )}
      >
        {state === "done" ? (
          <>
            <Check className="size-3.5" />
            <span className="sr-only">{m.overview_step_done()}</span>
          </>
        ) : (
          number
        )}
      </span>
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span
          className={cn(
            "font-medium",
            state === "done" && "text-muted-foreground",
          )}
        >
          {title}
        </span>
        <span className="text-sm text-muted-foreground">{description}</span>
      </div>
      {state !== "done" && children}
    </li>
  );
}

function OpenTerminalButton({
  machine,
  variant,
}: {
  machine: MachineResponse;
  variant: "default" | "outline";
}) {
  const { openTerminal } = useMachineActions(machine);

  return (
    openTerminal && <ActionButton variant={variant} action={openTerminal} />
  );
}

function SetupComplete({ onShowSetup }: { onShowSetup?: () => void }) {
  return (
    <section className="flex flex-1 flex-col gap-6 md:justify-center">
      <div className="flex flex-1 flex-col items-center justify-center gap-4 text-center md:flex-none">
        <img src={paronUrl} alt="" className="size-40 md:size-44" />
        <h1 className="text-2xl font-semibold tracking-tight">
          {m.overview_complete_title()}
        </h1>
        <Saying className="md:text-lg" />
      </div>
      <div className="flex flex-col gap-2 md:flex-row md:justify-center">
        {onShowSetup && (
          <Button variant="outline" size="lg" onClick={onShowSetup}>
            {m.overview_complete_show_setup()}
          </Button>
        )}
        <Link to="/machines" className={buttonVariants({ size: "lg" })}>
          <Server />
          {m.overview_complete_open_machines()}
        </Link>
      </div>
    </section>
  );
}

function Saying({ className }: { className?: string }) {
  const [saying] = useState(randomSaying);

  return (
    <p className={cn("text-balance", className)}>
      {/* eslint-disable-next-line i18next/no-literal-string */}
      <span className="font-mono text-warning">[¬_¬]</span>{" "}
      <span className="font-bold italic">{saying}</span>
    </p>
  );
}
