import { ChevronDown, ChevronUp, X } from "lucide-react";
import type { RefObject } from "react";

import {
  InputGroup,
  InputGroupAddon,
  InputGroupInput,
} from "@/components/shadcn/input-group";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/shadcn/popover";
import { m } from "@/generated/paraglide/messages";

import { isShortcut } from "./device/shortcuts";
import { ToolbarButton } from "./toolbar-button";

type SearchProps = {
  query: string;
  matches:
    | { position: number | undefined; total: number; limited: boolean }
    | undefined;
  onQueryChange: (query: string) => void;
  onFind: (direction: "next" | "previous") => void;
  onClose: () => void;
  field: RefObject<HTMLInputElement | null>;
};

export function SearchBar({
  open,
  query,
  matches,
  onQueryChange,
  onFind,
  onClose,
  field,
}: { open: boolean } & SearchProps) {
  return (
    <Popover
      open={open}
      onOpenChange={(nextOpen, details) => {
        if (!nextOpen && details.reason === "escape-key") onClose();
      }}
    >
      <PopoverTrigger
        nativeButton={false}
        render={
          <span
            aria-hidden
            tabIndex={-1}
            className="pointer-events-none absolute top-2 right-2"
          />
        }
      />
      <PopoverContent
        side="bottom"
        align="end"
        sideOffset={0}
        initialFocus={() => {
          field.current?.select();
          return field.current;
        }}
        finalFocus={false}
        className="w-auto flex-row items-center gap-1 p-1"
      >
        <SearchFields
          query={query}
          matches={matches}
          onQueryChange={onQueryChange}
          onFind={onFind}
          onClose={onClose}
          field={field}
        />
      </PopoverContent>
    </Popover>
  );
}

function SearchFields({
  query,
  matches,
  onQueryChange,
  onFind,
  onClose,
  field,
}: SearchProps) {
  const noMatches = !query || !matches || matches.total === 0;
  const count = matches?.limited ? `${matches.total}+` : (matches?.total ?? 0);

  return (
    <>
      <InputGroup className="h-7 w-40 sm:w-56">
        <InputGroupInput
          ref={field}
          value={query}
          placeholder={m.terminal_search_placeholder()}
          aria-label={m.terminal_search()}
          className="text-sm text-ellipsis"
          onChange={(event) => onQueryChange(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              onFind(event.shiftKey ? "previous" : "next");
            }
            if (isShortcut(event, "search")) {
              event.preventDefault();
              event.currentTarget.select();
            }
          }}
        />
        {query && matches && (
          <InputGroupAddon
            align="inline-end"
            className="text-xs font-normal whitespace-nowrap tabular-nums"
          >
            {matches.total === 0
              ? m.terminal_search_none()
              : matches.position === undefined
                ? count
                : m.terminal_search_results({ index: matches.position, count })}
          </InputGroupAddon>
        )}
      </InputGroup>
      <ToolbarButton
        label={m.terminal_search_previous()}
        disabled={noMatches}
        onClick={() => onFind("previous")}
      >
        <ChevronUp />
      </ToolbarButton>
      <ToolbarButton
        label={m.terminal_search_next()}
        disabled={noMatches}
        onClick={() => onFind("next")}
      >
        <ChevronDown />
      </ToolbarButton>
      <ToolbarButton label={m.terminal_search_close()} onClick={onClose}>
        <X />
      </ToolbarButton>
    </>
  );
}
