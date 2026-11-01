const locale = document.documentElement.lang || "en";

const dateTime = new Intl.DateTimeFormat(locale, {
  year: "numeric",
  month: "short",
  day: "numeric",
  hour: "2-digit",
  minute: "2-digit",
  hourCycle: "h23",
  timeZoneName: "short",
});

export function formatDateTime(iso: string) {
  return dateTime.format(new Date(iso));
}

export function formatDateTimeRange(from: string, to: string) {
  return dateTime.formatRange(new Date(from), new Date(to));
}

const time = new Intl.DateTimeFormat(locale, {
  hour: "2-digit",
  minute: "2-digit",
  hourCycle: "h23",
});

export function formatTime(iso: string) {
  return time.format(new Date(iso));
}

const dayMonthYear = new Intl.DateTimeFormat(locale, {
  day: "numeric",
  month: "short",
  year: "numeric",
});

export function formatDate(date: Date) {
  return dayMonthYear.format(date);
}

export function formatDateRange(from: Date, to: Date) {
  return dayMonthYear.formatRange(from, to);
}

const day = new Intl.DateTimeFormat(locale, {
  weekday: "long",
  day: "numeric",
  month: "long",
  year: "numeric",
});

export function formatDay(instant: number) {
  return day.format(instant);
}

const relative = new Intl.RelativeTimeFormat(locale, { numeric: "auto" });

const UNITS = [
  ["year", 31_536_000],
  ["month", 2_628_000],
  ["week", 604_800],
  ["day", 86_400],
  ["hour", 3_600],
  ["minute", 60],
  ["second", 1],
] as const;

export function formatRelative(iso: string, now: number) {
  const seconds = (new Date(iso).getTime() - now) / 1000;
  const [unit, size] =
    UNITS.find(([, size]) => Math.abs(seconds) >= size) ??
    (["second", 1] as const);

  return relative.format(Math.trunc(seconds / size) || 0, unit);
}
