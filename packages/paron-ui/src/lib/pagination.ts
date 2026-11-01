import { z } from "zod";

export const PAGE_SIZE = 20;

export const pageSearch = z.object({
  page: z.number().int().min(1).catch(1).default(1),
});

export function pageQuery(page: number, limit = PAGE_SIZE) {
  return { limit, offset: (page - 1) * limit };
}

export function pageOfQuery({
  limit,
  offset,
}: {
  limit: number;
  offset: number;
}) {
  return offset / limit + 1;
}
