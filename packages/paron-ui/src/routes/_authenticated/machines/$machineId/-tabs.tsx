import { linkOptions, useParams } from "@tanstack/react-router";

import { PageTabs } from "@/components/route/page-tabs";
import { m } from "@/generated/paraglide/messages";

export function MachineTabs() {
  const { machineId } = useParams({
    from: "/_authenticated/machines/$machineId",
  });

  return (
    <PageTabs
      tabs={[
        {
          label: m.audit_tab_overview(),
          link: linkOptions({
            to: "/machines/$machineId",
            params: { machineId },
          }),
        },
        {
          label: m.audit_tab_activity(),
          link: linkOptions({
            to: "/machines/$machineId/activity",
            params: { machineId },
          }),
        },
      ]}
    />
  );
}
