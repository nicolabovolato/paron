import { z } from "zod";

import { zListAuditEntriesQuery } from "@/generated/api/zod.gen";

const query = zListAuditEntriesQuery.shape;
const { role } = query.entity.unwrap().shape;

export const activitySearch = z.object({
  role: role.catch(role.def.defaultValue),
  from: query.from.catch(undefined),
  to: query.to.catch(undefined),
  order: query.order.catch(query.order.def.defaultValue),
});

export const ACTIVITY_SEARCH_DEFAULTS = activitySearch.parse({});

export type ActivitySearch = z.output<typeof activitySearch>;
