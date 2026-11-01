import { useNow } from "@/hooks/now";
import { formatDateTime, formatRelative } from "@/lib/format";

export function RelativeTime({ iso }: { iso: string }) {
  const text = useNow((now) => formatRelative(iso, now));

  return (
    <time dateTime={iso} title={formatDateTime(iso)}>
      {text}
    </time>
  );
}
