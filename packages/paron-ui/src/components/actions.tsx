import { Link, type LinkOptions } from "@tanstack/react-router";
import { cn } from "cn";
import { Ellipsis, type LucideIcon } from "lucide-react";
import { Fragment, type ComponentProps, type ReactElement } from "react";

import { Button, buttonVariants } from "@/components/shadcn/button";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuTrigger,
} from "@/components/shadcn/context-menu";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/shadcn/dropdown-menu";
import { Spinner } from "@/components/shadcn/spinner";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/shadcn/tooltip";

export type Action = {
  label: string;
  Icon?: LucideIcon;
  tone?: "warning" | "destructive";
  disabled?: { reason: string } | false;
} & (
  | { kind: "link"; link: LinkOptions }
  | {
      kind: "perform";
      perform: () => boolean | Promise<boolean>;
      pending?: boolean;
    }
);

export type ActionGroups = Array<Array<Action | undefined>>;

function ActionMenuItems({ groups }: { groups: ActionGroups }) {
  return groups
    .map((group, position) => ({
      position,
      actions: group.filter((action) => action !== undefined),
    }))
    .filter(({ actions }) => actions.length > 0)
    .map(({ position, actions }, index) => (
      <Fragment key={position}>
        {index > 0 && <DropdownMenuSeparator />}
        {actions.map((action) => (
          <ActionMenuItem key={action.label} action={action} />
        ))}
      </Fragment>
    ));
}

function ActionMenuItem({ action }: { action: Action }) {
  const pending = action.kind === "perform" && action.pending;
  const item = (
    <DropdownMenuItem
      variant={action.tone === "destructive" ? "destructive" : "default"}
      className={cn(
        action.tone === "warning" &&
          "text-warning **:text-warning! focus:bg-warning/10 focus:text-warning dark:focus:bg-warning/20",
        action.disabled && "data-disabled:pointer-events-auto",
      )}
      disabled={Boolean(action.disabled) || pending}
      render={
        action.kind === "link" && !action.disabled ? (
          <Link {...action.link} />
        ) : undefined
      }
      onClick={() => {
        if (action.kind === "perform") void action.perform();
      }}
    >
      {pending ? <Spinner /> : action.Icon && <action.Icon />}
      {action.label}
    </DropdownMenuItem>
  );

  if (!action.disabled) return item;

  return (
    <Tooltip>
      <TooltipTrigger render={item} />
      <TooltipContent side="right">{action.disabled.reason}</TooltipContent>
    </Tooltip>
  );
}

export function ActionsDropDownMenu({
  label,
  groups,
}: {
  label: string;
  groups: ActionGroups;
}) {
  if (groups.flat().every((action) => action === undefined)) return null;

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button
            variant="ghost"
            size="icon-sm"
            className="dark:hover:bg-muted"
            aria-label={label}
          />
        }
      >
        <Ellipsis />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-48">
        <ActionMenuItems groups={groups} />
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export function ActionsContextMenu({
  groups,
  ...props
}: ComponentProps<typeof ContextMenuTrigger> & { groups: ActionGroups }) {
  return (
    <ContextMenu
      disabled={groups.flat().every((action) => action === undefined)}
    >
      <ContextMenuTrigger {...props} />
      <ContextMenuContent className="w-48">
        <ActionMenuItems groups={groups} />
      </ContextMenuContent>
    </ContextMenu>
  );
}

export function ActionButton({
  action,
  variant = "outline",
  iconOnly = false,
  className,
  onPerformed,
}: {
  action: Action;
  variant?: "default" | "outline" | "ghost";
  iconOnly?: boolean;
  className?: string;
  /** Runs when the action happened. */
  onPerformed?: () => void;
}) {
  const ghost = variant === "ghost";
  const buttonVariant =
    action.tone === "destructive" && !ghost ? "destructive" : variant;
  const buttonSize = iconOnly ? "icon" : "default";
  const toneClassName = cn(
    action.tone === "warning" && "text-warning hover:text-warning",
    action.tone === "destructive" &&
      ghost &&
      "text-destructive hover:bg-destructive/10 hover:text-destructive",
  );
  const tooltip = action.disabled
    ? action.disabled.reason
    : iconOnly && action.label;

  const withTooltip = (control: ReactElement) =>
    tooltip ? (
      <Tooltip>
        <TooltipTrigger render={control} />
        <TooltipContent>{tooltip}</TooltipContent>
      </Tooltip>
    ) : (
      control
    );

  if (action.kind === "link" && !action.disabled)
    return withTooltip(
      <Link
        {...action.link}
        aria-label={iconOnly ? action.label : undefined}
        className={cn(
          buttonVariants({ variant: buttonVariant, size: buttonSize }),
          toneClassName,
          className,
        )}
      >
        {action.Icon && <action.Icon />}
        {!iconOnly && action.label}
      </Link>,
    );

  const pending = action.kind === "perform" && action.pending;
  const perform = async () => {
    if (action.kind === "perform" && (await action.perform())) onPerformed?.();
  };

  return withTooltip(
    <Button
      variant={buttonVariant}
      size={buttonSize}
      aria-label={iconOnly ? action.label : undefined}
      className={cn("aria-disabled:opacity-50", toneClassName, className)}
      disabled={Boolean(action.disabled) || pending}
      focusableWhenDisabled
      onClick={() => void perform()}
    >
      {pending ? <Spinner /> : action.Icon && <action.Icon />}
      {!iconOnly && action.label}
    </Button>,
  );
}
