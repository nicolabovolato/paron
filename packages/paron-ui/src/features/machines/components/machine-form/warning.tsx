import { TriangleAlert } from "lucide-react";
import type { ReactNode } from "react";

import { Alert, AlertDescription, AlertTitle } from "@/components/shadcn/alert";

export function Warning({
  title,
  description,
  action,
}: {
  title: string;
  description: string;
  action?: ReactNode;
}) {
  return (
    <Alert className="text-warning sm:has-[>svg]:grid-cols-[auto_1fr_auto]">
      <TriangleAlert />
      <AlertTitle>{title}</AlertTitle>
      <AlertDescription>{description}</AlertDescription>
      {action && (
        <div className="col-start-2 mt-2 sm:col-start-3 sm:row-span-2 sm:row-start-1 sm:mt-0 sm:ml-4 sm:self-center">
          {action}
        </div>
      )}
    </Alert>
  );
}
