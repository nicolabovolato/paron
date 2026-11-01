import { Search, X } from "lucide-react";
import { useId, useRef } from "react";
import type { z } from "zod";

import { FieldError } from "@/components/shadcn/field";
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
} from "@/components/shadcn/input-group";
import { Spinner } from "@/components/shadcn/spinner";
import { m } from "@/generated/paraglide/messages";
import { useSearchInput } from "@/hooks/search-input";

export function ListSearch({
  search,
  searchSchema,
  onSearchChange,
  placeholder,
  loading,
}: {
  search: string | undefined;
  searchSchema: z.ZodType<string | undefined>;
  onSearchChange: (search: string | undefined) => void;
  placeholder: string;
  loading: boolean;
}) {
  const input = useRef<HTMLInputElement>(null);
  const id = useId();
  const { inputText, changeInputText, clear, pending, errors } = useSearchInput(
    search,
    onSearchChange,
    searchSchema,
  );

  const removeSearch = () => {
    clear();
    input.current?.focus();
  };

  return (
    <div className="flex flex-col gap-2 sm:max-w-xs">
      <InputGroup>
        <InputGroupInput
          ref={input}
          type="search"
          aria-label={placeholder}
          aria-invalid={errors.length > 0}
          aria-describedby={`${id}-hint`}
          placeholder={placeholder}
          value={inputText}
          onChange={(event) => changeInputText(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Escape" && inputText !== "") {
              event.preventDefault();
              removeSearch();
            }
          }}
          className="[&::-webkit-search-cancel-button]:appearance-none"
        />
        <InputGroupAddon>
          {loading || pending ? <Spinner /> : <Search />}
        </InputGroupAddon>
        {inputText !== "" && (
          <InputGroupAddon align="inline-end">
            <InputGroupButton
              size="icon-xs"
              aria-label={m.list_search_clear()}
              onClick={removeSearch}
            >
              <X />
            </InputGroupButton>
          </InputGroupAddon>
        )}
      </InputGroup>
      <FieldError id={`${id}-hint`} errors={errors.slice(0, 1)} />
    </div>
  );
}
