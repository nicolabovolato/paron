import { linkOptions } from "@tanstack/react-router";
import { ShieldX } from "lucide-react";

import { BackButton } from "@/components/route/back-button";
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/shadcn/empty";
import { m } from "@/generated/paraglide/messages";

export function RouteForbidden() {
  return (
    <Empty className="border">
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <ShieldX />
        </EmptyMedia>
        <EmptyTitle>
          <h1>{m.route_forbidden_title()}</h1>
        </EmptyTitle>
        <EmptyDescription>{m.route_forbidden_description()}</EmptyDescription>
      </EmptyHeader>
      <EmptyContent>
        <BackButton
          label={m.route_back()}
          fallback={linkOptions({ to: "/" })}
          fallbackLabel={m.route_go_to({ page: m.overview_title() })}
        />
      </EmptyContent>
    </Empty>
  );
}
