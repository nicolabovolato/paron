import { useRouter } from "@tanstack/react-router";
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, test } from "vitest";

import { useAppForm } from "@/components/form";
import { useUnsavedChangesGuard } from "@/hooks/unsaved-changes-guard";
import { renderInRouter } from "@/test/app";

function TwoForms() {
  const router = useRouter();
  const firstForm = useAppForm({
    defaultValues: { value: "" },
    onSubmit: ({ value, formApi }) => {
      formApi.reset(value);
      router.history.push("/other");
    },
  });
  const secondForm = useAppForm({ defaultValues: { value: "" } });
  useUnsavedChangesGuard(firstForm);
  useUnsavedChangesGuard(secondForm);

  return (
    <>
      <firstForm.Field name="value">
        {(field) => (
          <input
            aria-label="First"
            value={field.state.value}
            onChange={(event) => field.handleChange(event.target.value)}
          />
        )}
      </firstForm.Field>
      <secondForm.Field name="value">
        {(field) => (
          <input
            aria-label="Second"
            value={field.state.value}
            onChange={(event) => field.handleChange(event.target.value)}
          />
        )}
      </secondForm.Field>
      <button onClick={() => void firstForm.handleSubmit()}>Save first</button>
      <button onClick={() => router.history.push("/other")}>Leave</button>
      <button
        onClick={() =>
          router.history.push("/other", undefined, { ignoreBlocker: true })
        }
      >
        Cancel
      </button>
    </>
  );
}

describe("useUnsavedChangesGuard()", () => {
  test("no changes", async () => {
    const user = userEvent.setup();
    renderInRouter(<TwoForms />);

    await user.click(await screen.findByRole("button", { name: "Leave" }));

    expect(await screen.findByText("Other page")).toBeInTheDocument();
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
  });

  test("discard", async () => {
    const user = userEvent.setup();
    renderInRouter(<TwoForms />);
    await user.type(await screen.findByLabelText("First"), "x");

    await user.click(screen.getByRole("button", { name: "Leave" }));
    await user.click(await screen.findByRole("button", { name: "Discard" }));

    expect(await screen.findByText("Other page")).toBeInTheDocument();
  });

  test("keep editing", async () => {
    const user = userEvent.setup();
    renderInRouter(<TwoForms />);
    await user.type(await screen.findByLabelText("First"), "x");

    await user.click(screen.getByRole("button", { name: "Leave" }));
    await user.click(
      await screen.findByRole("button", { name: "Keep editing" }),
    );

    await waitFor(() =>
      expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument(),
    );
    expect(screen.getByLabelText("First")).toHaveValue("x");
    expect(screen.queryByText("Other page")).not.toBeInTheDocument();
  });

  test("ignoreBlocker", async () => {
    const user = userEvent.setup();
    renderInRouter(<TwoForms />);
    await user.type(await screen.findByLabelText("First"), "x");

    await user.click(screen.getByRole("button", { name: "Cancel" }));

    expect(await screen.findByText("Other page")).toBeInTheDocument();
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
  });

  test("saved form", async () => {
    const user = userEvent.setup();
    renderInRouter(<TwoForms />);
    await user.type(await screen.findByLabelText("First"), "x");

    await user.click(screen.getByRole("button", { name: "Save first" }));

    expect(await screen.findByText("Other page")).toBeInTheDocument();
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
  });

  test("other dirty", async () => {
    const user = userEvent.setup();
    renderInRouter(<TwoForms />);
    await user.type(await screen.findByLabelText("First"), "x");
    await user.type(screen.getByLabelText("Second"), "y");

    await user.click(screen.getByRole("button", { name: "Save first" }));

    expect(await screen.findByRole("alertdialog")).toBeInTheDocument();
    expect(screen.queryByText("Other page")).not.toBeInTheDocument();
  });
});
