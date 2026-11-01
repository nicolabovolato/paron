import { useSuspenseQuery } from "@tanstack/react-query";
import {
  createFileRoute,
  type ErrorComponentProps,
  isMatch,
  Link,
  Outlet,
  redirect,
  useMatches,
  useMatchRoute,
  useNavigate,
  useRouter,
  useRouterState,
} from "@tanstack/react-router";
import { cn } from "cn";
import {
  ChevronsUpDown,
  House,
  LogOut,
  Monitor,
  Moon,
  Server,
  Sun,
  TextSearch,
  User,
  Users,
} from "lucide-react";
import {
  Fragment,
  type ReactNode,
  useState,
  useSyncExternalStore,
} from "react";

import { errorToast } from "@/components/error-toast";
import { RouteError } from "@/components/route/error";
import { RouteNotFound } from "@/components/route/not-found";
import {
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
} from "@/components/shadcn/breadcrumb";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/shadcn/dropdown-menu";
import { Separator } from "@/components/shadcn/separator";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarHeader,
  SidebarInset,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarProvider,
  SidebarRail,
  SidebarTrigger,
  useSidebar,
} from "@/components/shadcn/sidebar";
import { useToastManager } from "@/components/shadcn/toast";
import { useTheme } from "@/components/theme-provider";
import { UserBadge } from "@/features/users/components/user-badge";
import { m } from "@/generated/paraglide/messages";
import { ForbiddenError, usePermissions } from "@/hooks/permissions";
import { authQueries, useSignOut } from "@/lib/auth";

export const Route = createFileRoute("/_authenticated")({
  beforeLoad: async ({ context, location, matches }) => {
    const me = await context.queryClient
      .query({ ...authQueries.me(), staleTime: "static" })
      .catch(() => {
        throw redirect({ to: "/login", search: { redirect: location.href } });
      });

    for (const { staticData } of matches) {
      if (
        staticData.permission &&
        !me.permissions.includes(staticData.permission)
      )
        throw new ForbiddenError();
    }
  },

  component: Shell,
  notFoundComponent: ShellNotFound,
  errorComponent: ShellError,
});

function ShellNotFound() {
  return (
    <Shell>
      <RouteNotFound />
    </Shell>
  );
}

function ShellError(props: ErrorComponentProps) {
  return (
    <Shell>
      <RouteError {...props} />
    </Shell>
  );
}

const NAVIGATION = [
  {
    to: "/",
    route: "/_authenticated/",
    label: m.shell_nav_overview,
    Icon: House,
  },
  {
    to: "/machines",
    route: "/_authenticated/machines",
    label: m.shell_nav_machines,
    Icon: Server,
  },
  {
    to: "/users",
    route: "/_authenticated/users",
    label: m.shell_nav_users,
    Icon: Users,
  },
  {
    to: "/activity",
    route: "/_authenticated/activity",
    label: m.shell_nav_activity,
    Icon: TextSearch,
  },
] as const;

function storedSidebarOpen() {
  try {
    return localStorage.getItem("paron-sidebar") !== "closed";
  } catch {
    return true;
  }
}

function storeSidebarOpen(open: boolean) {
  try {
    localStorage.setItem("paron-sidebar", open ? "open" : "closed");
  } catch {
    // Storage is blocked; the sidebar still toggles for this session.
  }
}

function subscribeToScroll(notify: () => void) {
  window.addEventListener("scroll", notify, { passive: true });
  return () => window.removeEventListener("scroll", notify);
}

function Shell({ children = <Outlet /> }: { children?: ReactNode }) {
  const width = useMatches({
    select: (matches) =>
      matches.findLast(({ staticData }) => staticData.width)?.staticData
        .width ?? "wide",
  });
  const height = useMatches({
    select: (matches) =>
      matches.findLast(({ staticData }) => staticData.height)?.staticData
        .height ?? "auto",
  });
  const hasBreadcrumbs = useMatches({
    select: (matches) =>
      matches.some((match) => isMatch(match, "loaderData.breadcrumb")),
  });
  const [sidebarOpen, setSidebarOpen] = useState(storedSidebarOpen);
  const loading = useRouterState({ select: (state) => state.isLoading });

  const scrolled = useSyncExternalStore(
    subscribeToScroll,
    () => window.scrollY > 0,
  );

  return (
    <SidebarProvider
      className="min-h-[calc(100svh-var(--connection-banner-height))]"
      open={sidebarOpen}
      onOpenChange={(open) => {
        storeSidebarOpen(open);
        setSidebarOpen(open);
      }}
    >
      {loading && (
        <div
          aria-hidden
          className="fixed top-(--connection-banner-height) left-0 z-60 h-0.5 w-[85%] bg-primary transition-[width] delay-150 duration-[8s] ease-out starting:w-0"
        />
      )}
      <AppSidebar />
      <SidebarInset
        className={cn(
          "min-w-0 [--shell-header-height:--spacing(16)]",
          height === "screen" &&
            "h-[calc(100svh-var(--connection-banner-height))]",
        )}
      >
        <header
          data-scrolled={scrolled || undefined}
          className="sticky top-(--connection-banner-height) z-10 flex h-(--shell-header-height) shrink-0 items-center gap-2 border-t-2 border-b border-transparent bg-background px-4 transition-[border-color] data-scrolled:border-b-border md:px-5"
        >
          <SidebarTrigger className="-ml-1" />
          {hasBreadcrumbs && (
            <Separator
              orientation="vertical"
              className="mr-2 data-vertical:h-4 data-vertical:self-auto"
            />
          )}
          <Breadcrumbs />
        </header>
        <div
          className={cn(
            "mx-auto flex w-full flex-1 flex-col px-4 pt-4 md:px-7 md:pt-6",
            height === "auto" && "pb-10 md:pb-12",
            height === "screen" && "overflow-auto pb-4 md:pb-7",
            width === "narrow" && "max-w-3xl",
            width === "wide" && "max-w-6xl",
          )}
        >
          {children}
        </div>
      </SidebarInset>
    </SidebarProvider>
  );
}

function AppSidebar() {
  const router = useRouter();
  const matchRoute = useMatchRoute();
  const permissions = usePermissions();
  const { setOpenMobile } = useSidebar();
  const routes = NAVIGATION.filter(({ route }) => {
    const permission = router.routesById[route].options.staticData?.permission;

    return !permission || permissions.has(permission);
  });

  return (
    <Sidebar
      collapsible="icon"
      className="top-(--connection-banner-height) h-auto"
    >
      <SidebarHeader>
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton
              size="lg"
              render={<Link to="/" onClick={() => setOpenMobile(false)} />}
            >
              <img src="/favicon.svg" alt="" className="size-8" />
              <div className="flex items-baseline gap-1.5">
                <span className="text-lg font-extrabold tracking-tight">
                  PARON
                </span>
                <span className="text-xs text-muted-foreground">
                  {m.shell_version({ version: __PARON_VERSION__ })}
                </span>
              </div>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarHeader>

      <SidebarContent>
        <SidebarGroup>
          <SidebarMenu className="gap-1">
            {routes.map(({ to, label, Icon }) => (
              <SidebarMenuItem key={to}>
                <SidebarMenuButton
                  render={<Link to={to} onClick={() => setOpenMobile(false)} />}
                  tooltip={label()}
                  isActive={Boolean(matchRoute({ to, fuzzy: to !== "/" }))}
                >
                  <Icon />
                  <span>{label()}</span>
                </SidebarMenuButton>
              </SidebarMenuItem>
            ))}
          </SidebarMenu>
        </SidebarGroup>
      </SidebarContent>

      <SidebarFooter>
        <UserMenu />
      </SidebarFooter>
      <SidebarRail />
    </Sidebar>
  );
}

function UserMenu() {
  const { data: user } = useSuspenseQuery(authQueries.me());
  const { theme, setTheme } = useTheme();
  const { isMobile, setOpenMobile } = useSidebar();
  const navigate = useNavigate();
  const signOut = useSignOut();
  const toast = useToastManager();

  return (
    <SidebarMenu>
      <SidebarMenuItem>
        <DropdownMenu>
          <DropdownMenuTrigger
            render={
              <SidebarMenuButton size="lg" className="aria-expanded:bg-muted" />
            }
          >
            <UserBadge user={user} />
            <ChevronsUpDown />
          </DropdownMenuTrigger>
          <DropdownMenuContent
            className="w-56"
            side={isMobile ? "bottom" : "right"}
            align="end"
            sideOffset={4}
          >
            <DropdownMenuGroup>
              <DropdownMenuItem
                render={
                  <Link to="/account" onClick={() => setOpenMobile(false)} />
                }
              >
                <User />
                {m.shell_my_account()}
              </DropdownMenuItem>
            </DropdownMenuGroup>
            <DropdownMenuSeparator />
            <DropdownMenuGroup>
              <DropdownMenuLabel>{m.theme_label()}</DropdownMenuLabel>
              <DropdownMenuRadioGroup value={theme} onValueChange={setTheme}>
                <DropdownMenuRadioItem value="light">
                  <Sun />
                  {m.theme_light()}
                </DropdownMenuRadioItem>
                <DropdownMenuRadioItem value="dark">
                  <Moon />
                  {m.theme_dark()}
                </DropdownMenuRadioItem>
                <DropdownMenuRadioItem value="system">
                  <Monitor />
                  {m.theme_system()}
                </DropdownMenuRadioItem>
              </DropdownMenuRadioGroup>
            </DropdownMenuGroup>
            <DropdownMenuSeparator />
            <DropdownMenuItem
              onClick={() =>
                signOut.mutate(
                  {},
                  {
                    onSuccess: () => {
                      toast.add({
                        title: m.session_signed_out(),
                        type: "success",
                      });
                      void navigate({ to: "/login", ignoreBlocker: true });
                    },
                    onError: (error) =>
                      toast.add(errorToast(m.session_sign_out_error(), error)),
                  },
                )
              }
            >
              <LogOut />
              {m.session_sign_out()}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </SidebarMenuItem>
    </SidebarMenu>
  );
}

function Breadcrumbs() {
  const breadcrumbs = useMatches().filter((match) =>
    isMatch(match, "loaderData.breadcrumb"),
  );

  return (
    <Breadcrumb className="min-w-0">
      <title>
        {m.page_title({
          page: breadcrumbs
            .map((match) => match.loaderData?.breadcrumb)
            .reverse()
            .join(" - "),
        })}
      </title>
      <BreadcrumbList className="grid auto-cols-[minmax(0,max-content)] grid-flow-col">
        {breadcrumbs.map((match, index) => (
          <Fragment key={match.id}>
            {index > 0 && <BreadcrumbSeparator />}
            <BreadcrumbItem className="min-w-0">
              {index === breadcrumbs.length - 1 ? (
                <BreadcrumbPage
                  className="truncate"
                  title={match.loaderData?.breadcrumb}
                >
                  {match.loaderData?.breadcrumb}
                </BreadcrumbPage>
              ) : (
                <BreadcrumbLink
                  className="truncate"
                  title={match.loaderData?.breadcrumb}
                  render={<Link to={match.fullPath} params={match.params} />}
                >
                  {match.loaderData?.breadcrumb}
                </BreadcrumbLink>
              )}
            </BreadcrumbItem>
          </Fragment>
        ))}
      </BreadcrumbList>
    </Breadcrumb>
  );
}
