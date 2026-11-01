import { linkOptions } from "@tanstack/react-router";
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, test } from "vitest";

import { renderInRouter } from "@/test/app";

import { ActionButton, ActionsDropDownMenu, type Action } from "./actions";

const DISABLED_NEW_MACHINE_LINK = {
  label: "New machine",
  kind: "link",
  link: linkOptions({ to: "/machines/new" }),
  disabled: { reason: "Machine limit reached" },
} satisfies Action;

describe("<ActionButton />", () => {
  test("disabled link", async () => {
    const user = userEvent.setup();
    const router = renderInRouter(
      <ActionButton action={DISABLED_NEW_MACHINE_LINK} />,
    );
    const button = await screen.findByRole("button", { name: "New machine" });

    await user.click(button);

    expect(screen.queryByRole("link")).not.toBeInTheDocument();
    expect(button).toHaveAttribute("aria-disabled", "true");
    expect(router.state.location.pathname).toBe("/");
  });
});

describe("<ActionsDropDownMenu />", () => {
  test("disabled link", async () => {
    const user = userEvent.setup();
    const router = renderInRouter(
      <ActionsDropDownMenu
        label="Actions"
        groups={[[DISABLED_NEW_MACHINE_LINK]]}
      />,
    );
    await user.click(await screen.findByRole("button", { name: "Actions" }));
    const menuItem = await screen.findByRole("menuitem", {
      name: "New machine",
    });

    await user.click(menuItem);

    expect(menuItem).not.toHaveAttribute("href");
    expect(router.state.location.pathname).toBe("/");
  });

  test("empty group", async () => {
    const user = userEvent.setup();
    renderInRouter(
      <ActionsDropDownMenu
        label="Actions"
        groups={[
          [undefined],
          [DISABLED_NEW_MACHINE_LINK],
          [{ ...DISABLED_NEW_MACHINE_LINK, label: "Import machines" }],
        ]}
      />,
    );

    await user.click(await screen.findByRole("button", { name: "Actions" }));

    expect(await screen.findAllByRole("menuitem")).toHaveLength(2);
    expect(screen.getAllByRole("separator")).toHaveLength(1);
  });
});
