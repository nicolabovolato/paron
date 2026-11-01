import { cn } from "cn";
import { startTransition, useActionState, useId, useState } from "react";

import { AlertDialogContent, useDialog } from "@/components/dialog";
import { ErrorAlert } from "@/components/error-alert";
import {
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/shadcn/alert-dialog";
import { Field, FieldLabel } from "@/components/shadcn/field";
import { Input } from "@/components/shadcn/input";
import { Spinner } from "@/components/shadcn/spinner";
import { m } from "@/generated/paraglide/messages";
import { isApiError } from "@/lib/api-error";

/**
 * Runs `onConfirm`, shows it pending, and closes when it resolves. A rejected
 * request stays open with the error under `errorTitle`.
 */
export function ConfirmationDialog({
  title,
  description,
  confirmLabel,
  cancelLabel = m.dialog_cancel(),
  tone,
  typeToConfirm,
  errorTitle,
  onConfirm,
}: {
  title: string;
  description?: string;
  confirmLabel: string;
  cancelLabel?: string;
  tone?: "warning" | "destructive";
  typeToConfirm?: string;
  errorTitle?: string;
  onConfirm?: () => unknown;
}) {
  const dialog = useDialog();
  const inputId = useId();
  const [typed, setTyped] = useState("");

  const [error, confirm, pending] = useActionState(async () => {
    try {
      await onConfirm?.();
      dialog.close();
      return undefined;
    } catch (caught) {
      if (!isApiError(caught) || errorTitle === undefined) throw caught;
      return caught;
    }
  }, undefined);

  return (
    <AlertDialogContent>
      <AlertDialogHeader>
        <AlertDialogTitle>{title}</AlertDialogTitle>
        {description && (
          <AlertDialogDescription>{description}</AlertDialogDescription>
        )}
      </AlertDialogHeader>
      {typeToConfirm !== undefined && (
        <Field>
          <FieldLabel htmlFor={inputId}>
            {m.dialog_type_to_confirm({ value: typeToConfirm })}
          </FieldLabel>
          <Input
            id={inputId}
            autoComplete="off"
            spellCheck={false}
            value={typed}
            onChange={(event) => setTyped(event.target.value)}
          />
        </Field>
      )}
      {error && errorTitle && <ErrorAlert title={errorTitle} error={error} />}
      <AlertDialogFooter>
        <AlertDialogCancel>{cancelLabel}</AlertDialogCancel>
        <AlertDialogAction
          variant={
            tone === "destructive"
              ? "destructive"
              : tone === "warning"
                ? "ghost"
                : "default"
          }
          className={cn(
            tone === "warning" &&
              "bg-warning/10 text-warning hover:bg-warning/20 hover:text-warning dark:hover:bg-warning/20",
          )}
          disabled={
            pending || (typeToConfirm !== undefined && typed !== typeToConfirm)
          }
          onClick={() => startTransition(confirm)}
        >
          {pending && <Spinner />}
          {confirmLabel}
        </AlertDialogAction>
      </AlertDialogFooter>
    </AlertDialogContent>
  );
}
