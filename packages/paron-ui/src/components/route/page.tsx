import type { ReactNode } from "react";

export function Page({
  title,
  actions,
  children,
}: {
  title: ReactNode;
  actions?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <section className="flex flex-col gap-6">
      <header className="flex flex-wrap items-center justify-between gap-4">
        {typeof title === "string" ? (
          <h1 className="text-xl font-semibold tracking-tight">{title}</h1>
        ) : (
          title
        )}
        {actions}
      </header>
      {children}
    </section>
  );
}
