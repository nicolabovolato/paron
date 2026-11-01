import {
  createContext,
  use,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react";

type Clock = {
  subscribe: (notify: () => void) => () => void;
  now: () => number;
};

const ClockContext = createContext<Clock | undefined>(undefined);

/** One shared tick for every `useNow()` below it. */
export function ClockProvider({ children }: { children: ReactNode }) {
  const [clock] = useState(() => {
    const subscribers = new Set<() => void>();
    let now: number | undefined;
    let timer: ReturnType<typeof setInterval> | undefined;

    return {
      subscribe: (notify) => {
        subscribers.add(notify);
        if (timer === undefined) {
          now = Date.now();
          timer = setInterval(() => {
            now = Date.now();
            for (const notify of subscribers) notify();
          }, 1_000);
        }

        return () => {
          subscribers.delete(notify);
          if (subscribers.size > 0) return;

          clearInterval(timer);
          timer = undefined;
          now = undefined;
        };
      },
      now: () => (now ??= Date.now()),
    } satisfies Clock;
  });

  return <ClockContext value={clock}>{children}</ClockContext>;
}

// eslint-disable-next-line react-refresh/only-export-components
export function useNow<T>(select: (now: number) => T) {
  const clock = use(ClockContext);
  if (clock === undefined)
    throw new Error("useNow must be used within a ClockProvider");

  return useSyncExternalStore(clock.subscribe, () => select(clock.now()));
}
