import { linkOptions } from "@tanstack/react-router";
import { screen } from "@testing-library/react";
import { Users } from "lucide-react";
import { describe, expect, test } from "vitest";

import { renderInRouter } from "@/test/app";

import { Paginated } from "./paginated";

const USERS_LIST = {
  link: (page: number) => linkOptions({ to: "/users", search: { page } }),
  empty: { Icon: Users, title: "No users" },
  loading: false,
};

describe("<Paginated />", () => {
  test("empty", async () => {
    renderInRouter(
      <Paginated {...USERS_LIST} limit={20} offset={0} total={0}>
        <p>Records</p>
      </Paginated>,
    );

    expect(await screen.findByText("No users")).toBeInTheDocument();
    expect(screen.queryByText("Records")).not.toBeInTheDocument();
  });

  test("one page", async () => {
    renderInRouter(
      <Paginated {...USERS_LIST} limit={20} offset={0} total={20}>
        <p>Records</p>
      </Paginated>,
    );

    expect(await screen.findByText("Records")).toBeInTheDocument();
    expect(
      screen.queryByRole("navigation", { name: "Pagination" }),
    ).not.toBeInTheDocument();
  });

  test("middle page", async () => {
    renderInRouter(
      <Paginated {...USERS_LIST} limit={10} offset={20} total={170}>
        <p>Records</p>
      </Paginated>,
    );

    expect(await screen.findByText("21-30 of 170")).toBeInTheDocument();
    expect(
      screen.getByRole("link", { name: "Go to the previous page" }),
    ).toHaveAttribute("href", "/users?page=2");
    expect(
      screen.getByRole("link", { name: "Go to the next page" }),
    ).toHaveAttribute("href", "/users?page=4");
  });

  test("first page", async () => {
    renderInRouter(
      <Paginated {...USERS_LIST} limit={20} offset={0} total={30}>
        <p>Records</p>
      </Paginated>,
    );

    const previousPageLink = await screen.findByRole("link", {
      name: "Go to the previous page",
    });
    expect(previousPageLink).toHaveAttribute("aria-disabled", "true");
    expect(previousPageLink).not.toHaveAttribute("href");
    expect(
      screen.getByRole("link", { name: "Go to the next page" }),
    ).toHaveAttribute("href", "/users?page=2");
  });

  test("last page", async () => {
    renderInRouter(
      <Paginated {...USERS_LIST} limit={20} offset={20} total={30}>
        <p>Records</p>
      </Paginated>,
    );

    expect(await screen.findByText("21-30 of 30")).toBeInTheDocument();
    expect(
      screen.getByRole("link", { name: "Go to the next page" }),
    ).toHaveAttribute("aria-disabled", "true");
    expect(
      screen.getByRole("link", { name: "Go to the previous page" }),
    ).toHaveAttribute("href", "/users?page=1");
  });
});
