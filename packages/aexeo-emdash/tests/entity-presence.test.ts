import { describe, expect, it } from "vitest";

import {
  checkAllSources,
  entityFromManifest,
  type EntityInput,
  type SourceResult,
} from "../src/presence-check.js";
import type { TruthManifest } from "../src/types.js";

type Responder = (url: URL) => Response | Promise<Response>;

const json = (body: unknown, status = 200): Response =>
  new Response(JSON.stringify(body), { status });

// Answers every source with "nothing here" unless a test overrides its host.
function fetcher(overrides: Record<string, Responder> = {}): {
  fetch: typeof fetch;
  requested: URL[];
} {
  const requested: URL[] = [];
  const defaults: Record<string, Responder> = {
    "en.wikipedia.org": () => json(["q", [], [], []]),
    "www.wikidata.org": () => json({ search: [] }),
    "api.github.com": () => new Response("", { status: 404 }),
    "rdap.org": () => new Response("", { status: 404 }),
    "index.commoncrawl.org": (url) =>
      url.pathname === "/collinfo.json"
        ? json([{ id: "CC-MAIN-2030-01" }, { id: "CC-MAIN-2029-51" }])
        : new Response("", { status: 404 }),
  };
  const routes = { ...defaults, ...overrides };
  return {
    requested,
    fetch: (async (input: string | URL | Request) => {
      const url = new URL(String(input));
      requested.push(url);
      const respond = routes[url.hostname];
      if (respond === undefined) throw new Error(`unexpected host ${url.host}`);
      return respond(url);
    }) as typeof fetch,
  };
}

async function check(
  source: string,
  input: EntityInput,
  overrides?: Record<string, Responder>,
): Promise<SourceResult> {
  const results = await checkAllSources(input, fetcher(overrides).fetch);
  const result = results.find((candidate) => candidate.source === source);
  if (result === undefined) throw new Error(`no ${source} result`);
  return result;
}

const wikipedia = (title: string): Record<string, Responder> => ({
  "en.wikipedia.org": () =>
    json(["q", [title], ["A description"], [`https://en.wikipedia.org/${title}`]]),
});

const wikidata = (label: string): Record<string, Responder> => ({
  "www.wikidata.org": () =>
    json({ search: [{ id: "Q1", label, concepturi: "https://wikidata.org/Q1" }] }),
});

describe("entity name matching", () => {
  it.each([
    ["Apple", "Pineapple"],
    ["Acme", "Acme Rockets"],
    ["Acme Rockets", "Acme"],
    ["AI", "Mail"],
  ])("does not treat %s as found by the record %s", async (name, record) => {
    expect((await check("wikipedia", { name }, wikipedia(record))).status).toBe(
      "not_found",
    );
    expect((await check("wikidata", { name }, wikidata(record))).status).toBe(
      "not_found",
    );
  });

  it.each([
    ["Acme", "acme"],
    ["Acme", "Acme, Inc."],
    ["Acme Corp", "Acme Corporation"],
    ["Acme", "Acme (company)"],
    ["The Acme Company", "Acme"],
    ["Société Générale", "Société Générale SA"],
  ])("treats %s as found by the record %s", async (name, record) => {
    expect((await check("wikipedia", { name }, wikipedia(record))).status).toBe(
      "found",
    );
  });

  it("accepts a record that names a declared alias", async () => {
    const input = { name: "Acme Robotics", aliases: ["ACME"] };
    expect((await check("wikipedia", input, wikipedia("Acme"))).status).toBe(
      "found",
    );
  });

  it("does not reduce a name made only of legal words to nothing", async () => {
    const input = { name: "The Company" };
    expect((await check("wikipedia", input, wikipedia("Inc"))).status).toBe(
      "not_found",
    );
    expect(
      (await check("wikipedia", input, wikipedia("The Company"))).status,
    ).toBe("found");
  });
});

describe("Common Crawl", () => {
  const input = { name: "Acme", website: "https://acme.example/" };

  it("queries the whole domain, not only the home page URL", async () => {
    const { fetch: fake, requested } = fetcher();
    await checkAllSources(input, fake);
    const query = requested.find((url) => url.pathname.endsWith("-index"));
    expect(query?.searchParams.get("url")).toBe("acme.example");
    expect(query?.searchParams.get("matchType")).toBe("domain");
  });

  it("reports a host with captures as found", async () => {
    const result = await check("common_crawl", input, {
      "index.commoncrawl.org": (url) =>
        url.pathname === "/collinfo.json"
          ? json([{ id: "CC-MAIN-2030-01" }])
          : new Response('{"timestamp":"20300105120000"}\n'),
    });
    expect(result.status).toBe("found");
    expect(result.extra).toContain("2030-01-05");
  });

  it("is skipped without a website", async () => {
    expect((await check("common_crawl", { name: "Acme" })).status).toBe("skipped");
  });
});

describe("failure states", () => {
  it("keeps a network failure distinct from an absent entity", async () => {
    const down: Responder = () => {
      throw new Error("connection refused");
    };
    const results = await checkAllSources(
      { name: "Acme", website: "https://acme.example" },
      fetcher({
        "en.wikipedia.org": down,
        "www.wikidata.org": down,
        "api.github.com": down,
        "rdap.org": down,
        "index.commoncrawl.org": down,
      }).fetch,
    );
    expect(results.map((result) => result.status)).toEqual(
      Array(5).fill("unreachable"),
    );
  });

  it("reports a GitHub rate limit as unreachable", async () => {
    const result = await check("github", { name: "acme" }, {
      "api.github.com": () => new Response("", { status: 403 }),
    });
    expect(result.status).toBe("unreachable");
  });

  it("skips GitHub for a name that cannot be a handle", async () => {
    expect((await check("github", { name: "Acme & Sons" })).status).toBe("skipped");
  });
});

describe("entityFromManifest", () => {
  it("needs an organization name", () => {
    expect(entityFromManifest(null)).toBeNull();
    expect(entityFromManifest({} as TruthManifest)).toBeNull();
  });

  it("carries the website and aliases", () => {
    expect(
      entityFromManifest({
        organization: { name: "Acme", website: "https://a.example", aliases: ["A"] },
      } as unknown as TruthManifest),
    ).toEqual({ name: "Acme", website: "https://a.example", aliases: ["A"] });
  });
});
