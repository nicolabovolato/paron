import { useId } from "react";

import {
  Field,
  FieldContent,
  FieldDescription,
  FieldLabel,
  FieldTitle,
} from "@/components/shadcn/field";
import { RadioGroupItem } from "@/components/shadcn/radio-group";

export function ChoiceCard({
  value,
  title,
  description,
}: {
  value: string;
  title: string;
  description: string;
}) {
  const id = useId();

  return (
    <FieldLabel htmlFor={id}>
      <Field orientation="horizontal">
        <FieldContent>
          <FieldTitle>{title}</FieldTitle>
          <FieldDescription>{description}</FieldDescription>
        </FieldContent>
        <RadioGroupItem value={value} id={id} />
      </Field>
    </FieldLabel>
  );
}
