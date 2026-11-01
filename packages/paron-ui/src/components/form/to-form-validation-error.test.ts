import { FieldApi, FormApi } from "@tanstack/react-form";
import { describe, expect, test } from "vitest";

import type { ApiError } from "@/lib/api-error";

import { toFormValidationError } from "./to-form-validation-error";

function formWithNameField() {
  const form = new FormApi({ defaultValues: { name: "", hidden: "" } });
  form.mount();
  new FieldApi({ form, name: "name" }).mount();

  return form;
}

describe("toFormValidationError()", () => {
  test("field messages", () => {
    const error = {
      status: 422,
      validation_errors: { name: ["too", "long"] },
    } satisfies ApiError;

    expect(toFormValidationError(formWithNameField(), error)).toEqual({
      form: undefined,
      fields: { name: { message: "too long" } },
    });
  });

  test("unknown field", () => {
    const error = {
      status: 422,
      validation_errors: { name: ["too long"], hidden: ["missing"] },
    } satisfies ApiError;

    expect(toFormValidationError(formWithNameField(), error)).toEqual({
      form: error,
      fields: { name: { message: "too long" } },
    });
  });

  test("409", () => {
    const error = {
      status: 409,
      detail: "taken",
    } satisfies ApiError;

    expect(toFormValidationError(formWithNameField(), error)).toEqual({
      form: error,
      fields: {},
    });
  });
});
