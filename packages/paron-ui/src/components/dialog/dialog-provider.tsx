import { useState, type ReactNode } from "react";

import { StackedDialogContext } from "./stacked-dialog";
import { DialogContext, type DialogScope } from "./use-dialog";

type Entry = {
  id: number;
  parentId: number | undefined;
  content: ReactNode;
  status: "open" | "closing";
  dialog: DialogScope;
  dismiss: () => void;
};

// Entries are kept in the order they opened, so a parent always comes first.
function withDescendants(entries: Entry[], ids: Set<number>) {
  for (const entry of entries)
    if (entry.parentId !== undefined && ids.has(entry.parentId))
      ids.add(entry.id);
  return ids;
}

export function DialogProvider({ children }: { children: ReactNode }) {
  const [entries, setEntries] = useState<Entry[]>([]);

  const [stack] = useState(() => {
    let lastId = 0;
    const resolvers = new Map<number, (completed: boolean) => void>();

    const resolve = (id: number, completed: boolean) => {
      resolvers.get(id)?.(completed);
      resolvers.delete(id);
    };

    const close = (id: number, completed: boolean) => {
      resolve(id, completed);
      setEntries((all) => {
        const closing = withDescendants(all, new Set([id]));
        return all.map((entry) =>
          closing.has(entry.id) ? { ...entry, status: "closing" } : entry,
        );
      });
    };

    const open: DialogScope["open"] = (content) => {
      const id = ++lastId;
      const completed = new Promise<boolean>((resolve) =>
        resolvers.set(id, resolve),
      );
      const dismiss = () => close(id, false);
      setEntries((all) => [
        ...all,
        {
          id,
          parentId: all.findLast((entry) => entry.status !== "closing")?.id,
          content,
          status: "open",
          dialog: { open, close: () => close(id, true) },
          dismiss,
        },
      ]);

      return { completed, dismiss };
    };

    const page: DialogScope = {
      open,
      close: () => {
        throw new Error("useDialog().close() must be called inside a dialog");
      },
    };

    return { page, resolve };
  });

  const renderDialogs = (parentId: number | undefined) =>
    entries
      .filter((entry) => entry.parentId === parentId)
      .map((entry) => {
        const rootProps = {
          open: entry.status === "open",
          onOpenChange: (open: boolean) => {
            if (!open) entry.dismiss();
          },
          onOpenChangeComplete: (open: boolean) => {
            if (open || entry.status !== "closing") return;
            const removed = withDescendants(entries, new Set([entry.id]));
            for (const id of removed) stack.resolve(id, false);
            setEntries((all) => all.filter(({ id }) => !removed.has(id)));
          },
        };
        return (
          <DialogContext key={entry.id} value={entry.dialog}>
            <StackedDialogContext
              value={{
                root: rootProps,
                above: renderDialogs(entry.id),
              }}
            >
              {entry.content}
            </StackedDialogContext>
          </DialogContext>
        );
      });

  return (
    <DialogContext value={stack.page}>
      {children}
      {renderDialogs(undefined)}
    </DialogContext>
  );
}
