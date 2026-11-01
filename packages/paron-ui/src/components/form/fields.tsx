import { useSelector } from "@tanstack/react-form";
import { cn } from "cn";
import { Eye, EyeOff } from "lucide-react";
import {
  Fragment,
  useId,
  useState,
  type ComponentProps,
  type ReactNode,
} from "react";

import { Button } from "@/components/shadcn/button";
import {
  Field,
  FieldDescription,
  FieldError,
  FieldLabel,
} from "@/components/shadcn/field";
import { Input } from "@/components/shadcn/input";
import {
  InputOTP,
  InputOTPGroup,
  InputOTPSeparator,
  InputOTPSlot,
} from "@/components/shadcn/input-otp";
import { Textarea } from "@/components/shadcn/textarea";
import { m } from "@/generated/paraglide/messages";

import { useFieldContext } from "./context";

type Labelled = {
  label: string;
  description?: string;
  /** For a screen reader only, when the place of the field already names it. */
  labelHidden?: boolean;
};

type InputProps = {
  id: string;
  name: string;
  onBlur: () => void;
  "aria-invalid": boolean;
  "aria-describedby": string | undefined;
};

// Props the field sets from the form, so a caller cannot pass them.
type SetByField = keyof InputProps | "value" | "onChange";

export function TextField({
  label,
  description,
  labelHidden,
  type,
  ...input
}: Labelled & Omit<ComponentProps<typeof Input>, SetByField>) {
  const field = useFieldContext<string>();
  const [shown, setShown] = useState(false);
  const password = type === "password";

  return (
    <LabelledField
      label={label}
      description={description}
      labelHidden={labelHidden}
    >
      {(inputProps) => {
        const control = (
          <Input
            {...input}
            {...inputProps}
            type={password && shown ? "text" : type}
            value={field.state.value}
            onChange={(event) => field.handleChange(event.target.value)}
          />
        );
        if (!password) return control;

        return (
          // Edge draws its own reveal button in a password field; one is enough.
          <div className="relative [&>input]:pr-9 [&>input::-ms-reveal]:hidden">
            {control}
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              aria-label={m.form_show_password()}
              aria-pressed={shown}
              disabled={input.disabled}
              className="absolute top-1/2 right-0.5 -translate-y-1/2 text-muted-foreground after:absolute after:-inset-2 hover:text-foreground aria-pressed:text-foreground [&_svg]:size-3.5"
              onClick={() => setShown(!shown)}
            >
              {shown ? <EyeOff /> : <Eye />}
            </Button>
          </div>
        );
      }}
    </LabelledField>
  );
}

export function NumberField({
  label,
  description,
  ...input
}: Labelled & Omit<ComponentProps<typeof Input>, SetByField | "type">) {
  const field = useFieldContext<number>();

  return (
    <LabelledField label={label} description={description}>
      {(inputProps) => (
        <Input
          {...input}
          {...inputProps}
          type="number"
          // An empty input is NaN in the form, and React rejects NaN as a value.
          value={Number.isNaN(field.state.value) ? "" : field.state.value}
          onChange={(event) => field.handleChange(event.target.valueAsNumber)}
        />
      )}
    </LabelledField>
  );
}

export function TextareaField({
  label,
  description,
  ...textarea
}: Labelled & Omit<ComponentProps<typeof Textarea>, SetByField>) {
  const field = useFieldContext<string>();

  return (
    <LabelledField label={label} description={description}>
      {(inputProps) => (
        <Textarea
          {...textarea}
          className={cn("wrap-anywhere", textarea.className)}
          {...inputProps}
          value={field.state.value}
          onChange={(event) => field.handleChange(event.target.value)}
        />
      )}
    </LabelledField>
  );
}

/**
 * One box for each character, in `groups` of boxes. A paste drops the dashes and spaces.
 */
export function CodeField({
  label,
  description,
  groups,
  pattern,
  inputMode,
  autoComplete,
  autoFocus,
  centered,
}: Labelled & {
  groups: number[];
  pattern: string;
  inputMode: "numeric" | "text";
  autoComplete: string;
  autoFocus?: boolean;
  centered?: boolean;
}) {
  const field = useFieldContext<string>();
  const slotGroups = groups.map((size, group) => {
    const start = groups.slice(0, group).reduce((sum, each) => sum + each, 0);
    return Array.from({ length: size }, (_, offset) => start + offset);
  });
  const length = slotGroups.flat().length;

  return (
    <LabelledField
      label={label}
      description={description}
      className={cn(centered && "items-center text-center *:w-auto")}
    >
      {(inputProps) => (
        <InputOTP
          {...inputProps}
          maxLength={length}
          pattern={pattern}
          inputMode={inputMode}
          autoComplete={autoComplete}
          autoFocus={autoFocus}
          pasteTransformer={(pasted) => pasted.replaceAll(/[\s-]/g, "")}
          value={field.state.value}
          onChange={field.handleChange}
        >
          {slotGroups.map((slots, group) => (
            <Fragment key={group}>
              {group > 0 && <InputOTPSeparator />}
              <InputOTPGroup>
                {slots.map((index) => (
                  <InputOTPSlot
                    key={index}
                    index={index}
                    // Ten boxes must fit beside each other on a phone.
                    className={cn(length > 6 && "max-sm:size-7")}
                  />
                ))}
              </InputOTPGroup>
            </Fragment>
          ))}
        </InputOTP>
      )}
    </LabelledField>
  );
}

function LabelledField({
  label,
  description,
  labelHidden,
  className,
  children,
}: Labelled & {
  className?: string;
  children: (inputProps: InputProps) => ReactNode;
}) {
  const field = useFieldContext<unknown>();
  const id = useId();
  const hintId = `${id}-hint`;
  const { isBlurred, isDirty, errors } = field.state.meta;
  const submitted = useSelector(
    field.form.store,
    (state) => state.submissionAttempts > 0,
  );
  // A form-wide schema checks every field at once, so a field keeps its error
  // to itself until the user has edited it and left it, or has submitted.
  const showsError = submitted || (isBlurred && isDirty);
  const invalid = showsError && errors.length > 0;

  return (
    <Field data-invalid={invalid} className={className}>
      <FieldLabel htmlFor={id} className={cn(labelHidden && "sr-only")}>
        {label}
      </FieldLabel>
      {children({
        id,
        name: field.name,
        onBlur: field.handleBlur,
        "aria-invalid": invalid,
        "aria-describedby": invalid || description ? hintId : undefined,
      })}
      {invalid ? (
        <FieldError id={hintId} errors={errors} />
      ) : (
        description && (
          <FieldDescription id={hintId}>{description}</FieldDescription>
        )
      )}
    </Field>
  );
}
