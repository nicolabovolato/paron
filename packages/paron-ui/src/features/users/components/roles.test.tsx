import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, test, vi } from "vitest";

import { RolesField } from "./roles";

describe("<RolesField />", () => {
  test("administrator checked", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<RolesField value={["operator"]} onChange={onChange} />);

    await user.click(screen.getByRole("checkbox", { name: "Administrator" }));

    expect(onChange).toHaveBeenCalledWith(["administrator"]);
  });

  test("administrator", () => {
    render(<RolesField value={["administrator"]} onChange={vi.fn()} />);

    expect(screen.getByRole("checkbox", { name: "Operator" })).toHaveAttribute(
      "aria-disabled",
      "true",
    );
    expect(screen.getByText("Included in Administrator.")).toBeInTheDocument();
  });

  test("role checked", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<RolesField value={[]} onChange={onChange} />);

    await user.click(screen.getByRole("checkbox", { name: "Operator" }));

    expect(onChange).toHaveBeenCalledWith(["operator"]);
  });
});
