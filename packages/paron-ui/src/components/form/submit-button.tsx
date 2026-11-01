import { useSelector } from "@tanstack/react-form";
import type { ComponentProps } from "react";

import { Button } from "@/components/shadcn/button";
import { Spinner } from "@/components/shadcn/spinner";

import { useFormContext } from "./context";

export function SubmitButton({
  children,
  ...button
}: Omit<ComponentProps<typeof Button>, "type" | "disabled">) {
  const form = useFormContext();
  const submitting = useSelector(form.store, (state) => state.isSubmitting);

  return (
    <Button
      type="submit"
      disabled={submitting}
      focusableWhenDisabled
      {...button}
    >
      {submitting && <Spinner />}
      {children}
    </Button>
  );
}
