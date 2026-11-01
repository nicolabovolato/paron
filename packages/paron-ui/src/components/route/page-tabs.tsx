import { Link, useMatchRoute, type LinkOptions } from "@tanstack/react-router";

import { Tabs, TabsList, TabsTrigger } from "@/components/shadcn/tabs";

export function PageTabs({
  tabs,
}: {
  tabs: { label: string; link: LinkOptions }[];
}) {
  const matchRoute = useMatchRoute();

  return (
    <Tabs value={tabs.findIndex((tab) => matchRoute(tab.link))}>
      <TabsList variant="line">
        {tabs.map((tab, index) => (
          <TabsTrigger
            key={tab.label}
            value={index}
            nativeButton={false}
            render={<Link {...tab.link} />}
          >
            {tab.label}
          </TabsTrigger>
        ))}
      </TabsList>
    </Tabs>
  );
}
