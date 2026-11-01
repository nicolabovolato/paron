import { render, screen, waitFor } from "@testing-library/react";
import userEvent, { type UserEvent } from "@testing-library/user-event";
import type { ComponentProps } from "react";
import { describe, expect, test } from "vitest";

import { ConfirmationDialog } from "@/components/confirmation-dialog";
import { DialogProvider, useDialog } from "@/components/dialog";

type ConfirmationDialogProps = Partial<
  ComponentProps<typeof ConfirmationDialog>
>;

function OpenDialogButton(props: ConfirmationDialogProps) {
  const dialog = useDialog();

  return (
    <button
      onClick={() =>
        void dialog.open(
          <ConfirmationDialog
            title="Delete thing?"
            description="This can't be undone."
            confirmLabel="Delete"
            tone="destructive"
            errorTitle="Couldn't delete it"
            onConfirm={() => undefined}
            {...props}
          />,
        )
      }
    >
      Open
    </button>
  );
}

async function openConfirmationDialog(
  user: UserEvent,
  props: ConfirmationDialogProps = {},
) {
  render(
    <DialogProvider>
      <OpenDialogButton {...props} />
    </DialogProvider>,
  );
  await user.click(screen.getByRole("button", { name: "Open" }));
  await screen.findByRole("alertdialog");
}

describe("<ConfirmationDialog />", () => {
  test("confirmed", async () => {
    const user = userEvent.setup();
    await openConfirmationDialog(user);

    await user.click(screen.getByRole("button", { name: "Delete" }));

    await waitFor(() =>
      expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument(),
    );
  });

  test("failed", async () => {
    const user = userEvent.setup();
    await openConfirmationDialog(user, {
      onConfirm: () =>
        // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- the API client rejects with an ApiError object, as here
        Promise.reject({
          status: 500,
          detail: "Server said no",
        }),
    });

    await user.click(screen.getByRole("button", { name: "Delete" }));

    expect(await screen.findByText("Server said no")).toBeInTheDocument();
    expect(screen.getByRole("alertdialog")).toBeInTheDocument();
  });

  test("typed confirmation", async () => {
    const user = userEvent.setup();
    await openConfirmationDialog(user, { typeToConfirm: "thing" });
    const deleteButton = screen.getByRole("button", { name: "Delete" });
    const confirmationField = screen.getByLabelText("Type thing to confirm");
    expect(deleteButton).toBeDisabled();

    await user.type(confirmationField, "thin");
    expect(deleteButton).toBeDisabled();

    await user.type(confirmationField, "g");
    expect(deleteButton).toBeEnabled();
  });

  test("pending", async () => {
    const user = userEvent.setup();
    await openConfirmationDialog(user, {
      onConfirm: () => new Promise(() => {}),
    });

    await user.click(screen.getByRole("button", { name: "Delete" }));

    expect(screen.getByRole("button", { name: /Delete/ })).toBeDisabled();
    expect(screen.getByRole("alertdialog")).toBeInTheDocument();
  });
});
