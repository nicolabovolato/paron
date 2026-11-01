import { Dialog } from "@base-ui/react/dialog";
import type { ComponentProps } from "react";

import { DialogContent as ShadcnDialogContent } from "@/components/shadcn/dialog";

import { useStackedDialog } from "./stacked-dialog";

export function DialogContent({
  className,
  ...props
}: ComponentProps<typeof ShadcnDialogContent>) {
  const { root, popupProps, above } = useStackedDialog(className);

  return (
    <Dialog.Root {...root} disablePointerDismissal>
      <ShadcnDialogContent {...popupProps} {...props} />
      {above}
    </Dialog.Root>
  );
}
