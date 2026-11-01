import { CodeBlock } from "@/components/code-block";
import { DialogContent } from "@/components/dialog";
import { Button } from "@/components/shadcn/button";
import {
  DialogClose,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/shadcn/dialog";
import { m } from "@/generated/paraglide/messages";

export function RecoveryCodesDialog({ codes }: { codes: string[] }) {
  return (
    <DialogContent className="sm:max-w-md" showCloseButton={false}>
      <DialogHeader>
        <DialogTitle>{m.recovery_codes_title()}</DialogTitle>
      </DialogHeader>

      <CodeBlock
        value={codes
          .map((code) => `${code.slice(0, 5)}-${code.slice(5)}`)
          .join("\n")}
        copyable
      />

      <DialogFooter>
        <DialogClose render={<Button />}>{m.recovery_codes_done()}</DialogClose>
      </DialogFooter>
    </DialogContent>
  );
}
