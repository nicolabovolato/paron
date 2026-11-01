import type { AnyFormApi } from "@tanstack/react-form";
import { useBlocker } from "@tanstack/react-router";
import { useCallback } from "react";

import { ConfirmationDialog } from "@/components/confirmation-dialog";
import { useDialog } from "@/components/dialog";
import { m } from "@/generated/paraglide/messages";

export function useUnsavedChangesGuard(form: AnyFormApi) {
  const dialog = useDialog();

  const unsaved = useCallback(() => !form.state.isDefaultValue, [form]);
  const confirmDiscard = useCallback(
    () =>
      dialog.open(
        <ConfirmationDialog
          title={m.dialog_discard_title()}
          description={m.dialog_discard_description()}
          confirmLabel={m.dialog_discard_confirm()}
          cancelLabel={m.dialog_discard_keep_editing()}
          tone="destructive"
        />,
      ),
    [dialog],
  );
  const shouldBlock = useCallback(
    async () => unsaved() && !(await confirmDiscard()),
    [unsaved, confirmDiscard],
  );

  useBlocker({ shouldBlockFn: shouldBlock, enableBeforeUnload: unsaved });
}
