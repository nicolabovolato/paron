import { Skeleton } from "@/components/shadcn/skeleton";

export function RoutePending() {
  return (
    <div className="flex flex-col gap-3 p-6">
      <Skeleton className="h-8 w-48" />
      <Skeleton className="h-40 w-full" />
    </div>
  );
}
