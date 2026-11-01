import { act, screen, waitFor } from "@testing-library/react";
import userEvent, { type UserEvent } from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, test } from "vitest";

import {
  AlertDialogCancel,
  AlertDialogTitle,
} from "@/components/shadcn/alert-dialog";
import { DialogTitle } from "@/components/shadcn/dialog";
import { renderInRouter } from "@/test/app";

import { AlertDialogContent, DialogContent, useDialog } from ".";

function NamedDialog({ name, saved }: { name: string; saved: Promise<void> }) {
  const dialog = useDialog();
  const [note, setNote] = useState("");

  return (
    <DialogContent>
      <DialogTitle>{name}</DialogTitle>
      <input
        aria-label={`Note ${name}`}
        value={note}
        onChange={(event) => setNote(event.target.value)}
      />
      <button
        onClick={() =>
          void dialog.open(<NamedDialog name={`${name}+`} saved={saved} />)
        }
      >
        Open child of {name}
      </button>
      <button onClick={() => void saved.then(() => dialog.close())}>
        Save {name}
      </button>
      <button onClick={() => void dialog.open(<DeleteAlert name={name} />)}>
        Alert from {name}
      </button>
      <button onClick={() => dialog.close()}>Close {name}</button>
    </DialogContent>
  );
}

let countedDialogRenders = 0;

function CountedDialog({ saved }: { saved: Promise<void> }) {
  const dialog = useDialog();
  countedDialogRenders++;

  return (
    <DialogContent>
      <DialogTitle>Counted</DialogTitle>
      <button
        onClick={() =>
          void dialog.open(<NamedDialog name="Child" saved={saved} />)
        }
      >
        Open child of Counted
      </button>
    </DialogContent>
  );
}

function DeleteAlert({ name }: { name: string }) {
  const dialog = useDialog();

  return (
    <AlertDialogContent>
      <AlertDialogTitle>Delete {name}?</AlertDialogTitle>
      <AlertDialogCancel>Cancel</AlertDialogCancel>
      <button onClick={() => dialog.close()}>Confirm</button>
    </AlertDialogContent>
  );
}

function OwnedDialogOpener({ onHide }: { onHide: () => void }) {
  const dialog = useDialog();

  return (
    <button
      onClick={() =>
        void dialog.open(
          <DialogContent>
            <DialogTitle>Owned</DialogTitle>
            <button onClick={onHide}>Hide opener</button>
          </DialogContent>,
        )
      }
    >
      Open owned
    </button>
  );
}

function DialogOpenersPage({
  saved = new Promise(() => {}),
}: {
  saved?: Promise<void>;
}) {
  const dialog = useDialog();
  const [message, setMessage] = useState("");
  const [openerMounted, setOpenerMounted] = useState(true);

  return (
    <>
      {openerMounted && (
        <OwnedDialogOpener onHide={() => setOpenerMounted(false)} />
      )}
      <button
        onClick={() => void dialog.open(<NamedDialog name="A" saved={saved} />)}
      >
        Open A
      </button>
      <button onClick={() => void dialog.open(<CountedDialog saved={saved} />)}>
        Open Counted
      </button>
      <button
        onClick={() =>
          void dialog
            .open(<DeleteAlert name="thing" />)
            .then((completed) =>
              setMessage(completed ? "Completed" : "Dismissed"),
            )
        }
      >
        Open alert with result
      </button>
      <button
        onClick={() => {
          try {
            dialog.close();
          } catch (caught) {
            setMessage((caught as Error).message);
          }
        }}
      >
        Close from the page
      </button>
      <p>{message}</p>
    </>
  );
}

const queryDialog = (name: string) =>
  screen.queryByRole("dialog", { name, hidden: true });

async function openDialogA(user: UserEvent) {
  await user.click(await screen.findByRole("button", { name: "Open A" }));
  await screen.findByRole("dialog", { name: "A" });
}

async function openChildDialogOf(user: UserEvent, name: string) {
  await user.click(
    await screen.findByRole("button", { name: `Open child of ${name}` }),
  );
  await screen.findByRole("dialog", { name: `${name}+` });
}

describe("useDialog()", () => {
  describe("open()", () => {
    test("nested", async () => {
      const user = userEvent.setup();
      renderInRouter(<DialogOpenersPage />);
      await openDialogA(user);

      await openChildDialogOf(user, "A");

      expect(queryDialog("A")).toHaveAttribute("data-nested-dialog-open");
      expect(queryDialog("A+")).toHaveAttribute("data-nested");
    });

    test("fresh state", async () => {
      const user = userEvent.setup();
      renderInRouter(<DialogOpenersPage />);
      await openDialogA(user);
      await user.type(screen.getByLabelText("Note A"), "gone");
      await user.click(screen.getByRole("button", { name: "Close A" }));
      await waitFor(() => expect(queryDialog("A")).not.toBeInTheDocument());

      await openDialogA(user);

      expect(screen.getByLabelText("Note A")).toHaveValue("");
    });

    test("parent state", async () => {
      const user = userEvent.setup();
      renderInRouter(<DialogOpenersPage />);
      await openDialogA(user);
      await user.type(screen.getByLabelText("Note A"), "kept");
      await openChildDialogOf(user, "A");

      await user.keyboard("{Escape}");

      await waitFor(() => expect(queryDialog("A+")).not.toBeInTheDocument());
      expect(screen.getByLabelText("Note A")).toHaveValue("kept");
    });

    test("no rerender", async () => {
      const user = userEvent.setup();
      renderInRouter(<DialogOpenersPage />);
      await user.click(
        await screen.findByRole("button", { name: "Open Counted" }),
      );
      await screen.findByRole("dialog", { name: "Counted" });
      const rendersBeforeChild = countedDialogRenders;

      await user.click(
        screen.getByRole("button", { name: "Open child of Counted" }),
      );
      await screen.findByRole("dialog", { name: "Child" });
      await user.keyboard("{Escape}");
      await waitFor(() => expect(queryDialog("Child")).not.toBeInTheDocument());

      expect(countedDialogRenders).toBe(rendersBeforeChild);
    });

    test("background inert", async () => {
      const user = userEvent.setup();
      renderInRouter(<DialogOpenersPage />);
      await openDialogA(user);
      expect(
        screen.queryByRole("button", { name: "Open A" }),
      ).not.toBeInTheDocument();

      await openChildDialogOf(user, "A");

      expect(
        screen.queryByRole("button", { name: "Close A" }),
      ).not.toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Close A+" })).toBeVisible();
    });

    test("outside click", async () => {
      const user = userEvent.setup();
      renderInRouter(<DialogOpenersPage />);
      await openDialogA(user);

      await user.click(document.body);

      expect(queryDialog("A")).toBeInTheDocument();
    });

    test("completed", async () => {
      const user = userEvent.setup();
      renderInRouter(<DialogOpenersPage />);
      await user.click(
        await screen.findByRole("button", { name: "Open alert with result" }),
      );
      await screen.findByRole("alertdialog");

      await user.click(screen.getByRole("button", { name: "Confirm" }));

      expect(await screen.findByText("Completed")).toBeInTheDocument();
    });

    test.each([
      ["Escape", (user: UserEvent) => user.keyboard("{Escape}")],
      [
        "Cancel",
        (user: UserEvent) =>
          user.click(screen.getByRole("button", { name: "Cancel" })),
      ],
    ])("dismissed by %s", async (_, dismiss) => {
      const user = userEvent.setup();
      renderInRouter(<DialogOpenersPage />);
      await user.click(
        await screen.findByRole("button", { name: "Open alert with result" }),
      );
      await screen.findByRole("alertdialog");

      await dismiss(user);

      expect(await screen.findByText("Dismissed")).toBeInTheDocument();
    });
  });

  describe("close()", () => {
    test("dialogs above", async () => {
      const user = userEvent.setup();
      let finishSave = () => {};
      const saved = new Promise<void>((resolve) => {
        finishSave = resolve;
      });
      renderInRouter(<DialogOpenersPage saved={saved} />);
      await openDialogA(user);
      await openChildDialogOf(user, "A");
      await user.click(screen.getByRole("button", { name: "Save A+" }));
      await openChildDialogOf(user, "A+");

      finishSave();
      await act(() => saved);

      await waitFor(() => expect(queryDialog("A+")).not.toBeInTheDocument());
      expect(queryDialog("A++")).not.toBeInTheDocument();
      expect(queryDialog("A")).toBeInTheDocument();
    });

    test("focus returned", async () => {
      const user = userEvent.setup();
      renderInRouter(<DialogOpenersPage />);
      await openDialogA(user);
      await openChildDialogOf(user, "A");

      await user.keyboard("{Escape}");
      await waitFor(() => expect(queryDialog("A+")).not.toBeInTheDocument());
      expect(
        screen.getByRole("button", { name: "Open child of A" }),
      ).toHaveFocus();

      await user.keyboard("{Escape}");
      await waitFor(() => expect(queryDialog("A")).not.toBeInTheDocument());
      expect(screen.getByRole("button", { name: "Open A" })).toHaveFocus();
    });

    test("opener unmounted", async () => {
      const user = userEvent.setup();
      renderInRouter(<DialogOpenersPage />);
      await user.click(
        await screen.findByRole("button", { name: "Open owned" }),
      );
      await screen.findByRole("dialog", { name: "Owned" });

      await user.click(screen.getByRole("button", { name: "Hide opener" }));

      await waitFor(() => expect(queryDialog("Owned")).not.toBeInTheDocument());
      expect(
        screen.queryByRole("button", { name: "Open owned" }),
      ).not.toBeInTheDocument();
    });

    test("page change", async () => {
      const user = userEvent.setup();
      const router = renderInRouter(<DialogOpenersPage />);
      await openDialogA(user);
      await openChildDialogOf(user, "A");

      await act(() => router.navigate({ href: "/other" }));

      expect(await screen.findByText("Other page")).toBeInTheDocument();
      await waitFor(() => expect(queryDialog("A")).not.toBeInTheDocument());
      expect(queryDialog("A+")).not.toBeInTheDocument();
    });

    test("outside dialog", async () => {
      const user = userEvent.setup();
      renderInRouter(<DialogOpenersPage />);

      await user.click(
        await screen.findByRole("button", { name: "Close from the page" }),
      );

      expect(
        screen.getByText("useDialog().close() must be called inside a dialog"),
      ).toBeInTheDocument();
    });
  });
});

describe("<AlertDialogContent />", () => {
  test("nested", async () => {
    const user = userEvent.setup();
    renderInRouter(<DialogOpenersPage />);
    await openDialogA(user);

    await user.click(screen.getByRole("button", { name: "Alert from A" }));

    expect(await screen.findByRole("alertdialog")).toBeInTheDocument();
    expect(queryDialog("A")).toHaveAttribute("data-nested-dialog-open");

    await user.keyboard("{Escape}");

    await waitFor(() =>
      expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument(),
    );
    expect(queryDialog("A")).toBeInTheDocument();
  });
});
