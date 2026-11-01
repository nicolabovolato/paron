import {
  Link,
  useCanGoBack,
  useRouter,
  type LinkOptions,
} from "@tanstack/react-router";

import { Button, buttonVariants } from "@/components/shadcn/button";

export function BackButton({
  label,
  fallback,
  fallbackLabel = label,
  variant = "outline",
}: {
  label: string;
  fallback: LinkOptions;
  fallbackLabel?: string;
  variant?: "outline" | "ghost";
}) {
  const router = useRouter();
  const canGoBack = useCanGoBack();

  return canGoBack ? (
    <Button variant={variant} onClick={() => router.history.back()}>
      {label}
    </Button>
  ) : (
    <Link {...fallback} className={buttonVariants({ variant })}>
      {fallbackLabel}
    </Link>
  );
}
