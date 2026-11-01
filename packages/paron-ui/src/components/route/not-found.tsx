import {
  isMatch,
  linkOptions,
  useMatch,
  useMatches,
} from "@tanstack/react-router";
import { SearchX } from "lucide-react";

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

export function RouteNotFound() {
  const failedMatch = useMatch({ strict: false });
  const parentPage = useMatches({
    select: (matches) =>
      matches
        .slice(
          0,
          matches.findIndex(({ id }) => id === failedMatch.id),
        )
        .filter((match) => isMatch(match, "loaderData.breadcrumb"))
        .at(-1),
  });
  const { notFound } = failedMatch.staticData;
  const parentBreadcrumb = parentPage?.loaderData?.breadcrumb;
  const fallback =
    parentPage && parentBreadcrumb !== undefined
      ? {
          link: { to: parentPage.fullPath, params: parentPage.params },
          label: m.route_go_to({ page: parentBreadcrumb }),
        }
      : {
          link: linkOptions({ to: "/" }),
          label: m.route_go_to({ page: m.overview_title() }),
        };

  return (
    <Empty className="border">
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <SearchX />
        </EmptyMedia>
        <EmptyTitle>
          <h1>{notFound?.title() ?? m.route_not_found_title()}</h1>
        </EmptyTitle>
        {notFound && (
          <EmptyDescription>{notFound.description()}</EmptyDescription>
        )}
      </EmptyHeader>
      <EmptyContent>
        <BackButton
          label={m.route_back()}
          fallback={fallback.link}
          fallbackLabel={fallback.label}
        />
      </EmptyContent>
    </Empty>
  );
}
