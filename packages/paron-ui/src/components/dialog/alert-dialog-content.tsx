import { AlertDialog } from "@base-ui/react/alert-dialog";
import type { ComponentProps } from "react";

import { AlertDialogContent as ShadcnAlertDialogContent } from "@/components/shadcn/alert-dialog";

import { useStackedDialog } from "./stacked-dialog";

export function AlertDialogContent({
  className,
  ...props
}: ComponentProps<typeof ShadcnAlertDialogContent>) {
  const { root, popupProps, above } = useStackedDialog(className);

  return (
    <AlertDialog.Root {...root}>
      <ShadcnAlertDialogContent {...popupProps} {...props} />
      {above}
    </AlertDialog.Root>
  );
}
