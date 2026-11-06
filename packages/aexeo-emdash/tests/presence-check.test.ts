import { describe, expect, it } from "vitest";

import { checkAllSources, type SourceResult } from "../src/presence-check.js";

const COLLINFO_URL = "https://index.commoncrawl.org/collinfo.json";

async function commonCrawlResult(
  collinfo: () => Response,
  seen: string[] = [],
): Promise<SourceResult> {
  const fetcher = (async (input: RequestInfo | URL) => {
    const url = String(input);
    seen.push(url);
    if (url === COLLINFO_URL) return collinfo();
    if (url.includes("CC-MAIN-2099-01-index")) {
      return new Response('{"timestamp":"20990101000000"}');
    }
    return new Response("", { status: 404 });
  }) as typeof fetch;
  const results = await checkAllSources(
    { name: "Aeptus", website: "https://www.aeptus.com" },
    fetcher,
  );
  return results.find((result) => result.source === "common_crawl")!;
}

describe("Common Crawl presence check", () => {
  it("searches the newest listed index, wherever the listing puts it", async () => {
    const seen: string[] = [];
    const result = await commonCrawlResult(
      () =>
        Response.json([
          { id: "CC-MAIN-2026-15" },
          { id: "CC-MAIN-2099-01" },
          { id: "../evil" },
        ]),
      seen,
    );
    expect(result.status).toBe("found");
    expect(result.extra).toContain("CC-MAIN-2099-01");
    expect(seen.some((url) => url.includes("CC-MAIN-2026-15"))).toBe(false);
  });

  it.each([
    ["a server error", () => new Response("busy", { status: 503 })],
    ["invalid JSON", () => new Response("not json")],
    ["an empty listing", () => Response.json([])],
    ["no usable id", () => Response.json([{ id: "../evil" }])],
  ])("is unreachable, never not_found, on %s", async (_name, collinfo) => {
    const result = await commonCrawlResult(collinfo);
    expect(result.status).toBe("unreachable");
    expect(result.error).toBeTruthy();
  });
});
