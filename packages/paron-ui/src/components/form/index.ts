import { createFormHook, revalidateLogic } from "@tanstack/react-form";

import { fieldContext, formContext } from "./context";
import { CodeField, NumberField, TextareaField, TextField } from "./fields";
import { SubmitButton } from "./submit-button";
import { SubmitError } from "./submit-error";

export { toFormValidationError } from "./to-form-validation-error";

export const { useAppForm, withFieldGroup } = createFormHook({
  fieldContext,
  formContext,
  fieldComponents: { TextField, NumberField, TextareaField, CodeField },
  formComponents: { SubmitButton, SubmitError },
});

const onBlur = revalidateLogic({
  mode: "blur",
  modeAfterSubmission: "change",
});
const onChange = revalidateLogic({
  mode: "change",
  modeAfterSubmission: "change",
});

// Every form checks a field when the user leaves it. After a submit, or while the form has an
// error, it checks on every change: an error that stays after the value is correct stops a submit
// with the Enter key, because the key does not make the field lose the focus.
export const validationLogic: typeof onBlur = (props) =>
  (props.form.state.isValid ? onBlur : onChange)(props);
