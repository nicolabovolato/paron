import { linkOptions, useParams } from "@tanstack/react-router";

import { PageTabs } from "@/components/route/page-tabs";
import { m } from "@/generated/paraglide/messages";

export function UserTabs() {
  const { userId } = useParams({ from: "/_authenticated/users/$userId" });

  return (
    <PageTabs
      tabs={[
        {
          label: m.audit_tab_overview(),
          link: linkOptions({ to: "/users/$userId", params: { userId } }),
        },
        {
          label: m.audit_tab_activity(),
          link: linkOptions({
            to: "/users/$userId/activity",
            params: { userId },
          }),
        },
      ]}
    />
  );
}
