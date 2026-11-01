import { type ClassValue, cn } from "cn";
import {
  createContext,
  type CSSProperties,
  type ReactNode,
  use,
  useEffect,
  useState,
} from "react";

export type StackedDialog = {
  root: {
    open: boolean;
    onOpenChange: (open: boolean) => void;
    onOpenChangeComplete: (open: boolean) => void;
  };
  above: ReactNode;
};

type DialogSize = { width: number; height: number };

export const StackedDialogContext = createContext<StackedDialog | undefined>(
  undefined,
);

const FrontDialogSizeContext = createContext<
  ((size: DialogSize | undefined) => void) | undefined
>(undefined);

// The dialogs behind the front one go behind it the same way for every size: each one a tenth
// narrower than the one in front of it, with only its title above it.
export function useStackedDialog(className: ClassValue) {
  const stacked = use(StackedDialogContext);
  if (stacked === undefined)
    throw new Error(
      "DialogContent and AlertDialogContent must be opened with useDialog().open()",
    );

  const reportToParent = use(FrontDialogSizeContext);
  // State, not a ref: the popup mounts in a portal after the first effects.
  const [popup, setPopup] = useState<HTMLDivElement | null>(null);
  const [size, setSize] = useState<DialogSize>();
  const [frontSize, setFrontSize] = useState<DialogSize>();

  useEffect(() => {
    if (!popup) return;
    const observer = new ResizeObserver(() =>
      setSize({ width: popup.offsetWidth, height: popup.offsetHeight }),
    );
    observer.observe(popup);
    return () => observer.disconnect();
  }, [popup]);

  useEffect(
    () => reportToParent?.(frontSize ?? size),
    [reportToParent, frontSize, size],
  );
  useEffect(() => () => reportToParent?.(undefined), [reportToParent]);

  return {
    root: stacked.root,
    popupProps: {
      ref: setPopup,
      className: cn(
        "transition-[translate,scale,clip-path] ease-out data-nested:shadow-lg motion-reduce:transition-none",
        // A dialog taller than the window scrolls inside it.
        "max-h-[calc(100dvh-2rem)] overflow-y-auto overscroll-contain",
        // The veil also keeps the pointer off this dialog.
        "data-nested-dialog-open:after:absolute data-nested-dialog-open:after:inset-0 data-nested-dialog-open:after:rounded-[inherit] data-nested-dialog-open:after:bg-background/60",
        // --nested-dialogs is the number of dialogs in front of this one.
        frontSize &&
          "origin-top translate-y-[calc(var(--front-height)*-0.5px-2.75rem*var(--nested-dialogs))] scale-(--nested-scale) [--nested-scale:calc(pow(0.9,var(--nested-dialogs))*var(--front-width)/var(--popup-width))] [clip-path:inset(0_0_calc(100%-(2.75rem+var(--front-height)*0.5px)/var(--nested-scale))_0)]",
        className,
      ),
      // Plain numbers, because CSS cannot divide one length by another.
      style: (frontSize &&
        size && {
          "--front-width": frontSize.width,
          "--front-height": frontSize.height,
          "--popup-width": size.width,
        }) as CSSProperties | undefined,
    },
    above: (
      <FrontDialogSizeContext value={setFrontSize}>
        {stacked.above}
      </FrontDialogSizeContext>
    ),
  };
}
