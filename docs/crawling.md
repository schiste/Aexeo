# Live crawl behaviour

How `crawl`, `verify`, `profile runtime`, `perf baseline` and `content --crawl-url`
decide what to request and how they report the result.

## Link discovery

Links are read from the parsed DOM and resolved against the observed page URL, as
a browser would: relative and parent paths, unquoted attributes and `<base href>`
all work. Only `<a href>` is followed; `data-href` and markup inside scripts are not.

Every root-relative link the broken-link rule checks is also requested, including
anchors in `<template>` and `<noscript>` that the DOM keeps inert. A complete crawl
therefore never reports `LNK001` for a link it did not ask the server about. These
anchors are followed for discovery only; they are not link-graph evidence.

Query strings are not part of a route. `?page=2` resolves to the route of its path,
so query variants, including query pagination, are never requested separately. This
prevents faceted URLs from becoming a crawl trap. A page linked only through a query
variant is not reached; seed it or list it in the sitemap under its own path.

Routes do not keep a trailing slash. `/docs/` is requested as `/docs`, and relative
links on it resolve against the URL the server finally answers from.

## robots.txt and identification

robots.txt is fetched before any other request and honoured by default, following
RFC 9309, for page, seed and sitemap URLs and for optional artifact probes.

| robots.txt response | Effect |
| --- | --- |
| 2xx | The rules apply: the group naming the crawler's product token, otherwise `*` |
| 4xx other than 429 | No restrictions |
| 5xx | Everything is disallowed, with a `CRW002` warning naming `--ignore-robots` |
| No response (network failure) | Everything is disallowed, with a `CRW002` warning naming `--ignore-robots`. If no page responds either, the target is unreachable: the command fails with exit 2 instead of reporting an audit |
| Refused by the [network boundary](#network-boundary), such as a redirect to a private address | Everything is disallowed, with a `CRW002` warning naming the refusal. The target answered, so this is not an unreachable target and does not exit 2 |
| 429 | Treated like a 5xx. RFC 9309 groups it with 4xx; this is deliberately stricter, because 429 asks the client to back off |

Requests send `User-Agent: aexeo/<version>` unless `headers` sets another one. The
rules are matched against the first token of the effective value, so an overridden
User-Agent is held to the rules for that agent.

Skipped URLs are recorded in `site.crawl_meta.robots_skipped_urls` and counted in
`crawl.skipped_by_robots`. A skip is not a failure and leaves the audit `complete`,
and a link to a skipped route is not reported as broken. If robots.txt blocks every
route tried, nothing was audited and the status is `partial`.

To audit a site you own, such as staging, pass `--ignore-robots` or set
`ignore_robots = true` under `[runtime]`. robots.txt is still fetched and linted.
`content --crawl-url` has no flag; use `--crawl-config`.

`crawl_delay_ms` under `[runtime]` waits between batches. A batch holds up to
`max_workers` requests. The `Crawl-delay` directive is not read.

## Status

`complete` means the audit finished, not that the site is healthy. Exit codes keep
the two apart: exit 1 reports defects found, and exit 2 from `content --crawl-url`
reports an audit that did not finish. Otherwise the status is `partial` and
`truncation_reason` says why.

A non-2xx response is never a page. It is not audited and is recorded in
`site.crawl_meta.failed_responses`, with a `CRW001` finding. What else happens depends
on the response:

| Response | Status | `CRW001` | Counted in | Links to it |
| --- | --- | --- | --- | --- |
| 404, 410, other 4xx, 3xx | Unaffected | error | `crawl.observed_error_responses` | `LNK001` |
| 401, 403 | `partial` | warning | `crawl.access_denied` | Not broken: the page may exist |
| 429, 5xx (after retries) | `partial` | error | `crawl.fetch_failures` | `LNK001` |
| Transport error, unreadable body | `partial` | error | `crawl.fetch_failures` | `LNK001` |
| Refused by the [network boundary](#network-boundary) | `partial` | error | `crawl.fetch_failures` | `LNK001` |

A site with one broken link therefore ends `complete` and exits 1. If every requested
URL returned an error status, nothing was audited and the status is `partial`.

A link to a URL that redirected to an audited page, through a recorded chain, is
not broken (see [legacy findings](link-graph.md#legacy-inbound-link-findings)).

`--seed-file inventory.json` also requests every inventory page and records a
crawl attestation (see
[inventory-seeded crawls](link-graph.md#inventory-seeded-crawl-attestation)).
With `--link-graph-output`, the crawl's own link report then uses that
inventory as its context, or the file `--link-graph-context` names, so a
single command can confirm an unlinked published page and remove the orphan
findings of verified aliases. Before this, that report had no context: adding
`--seed-file` now changes the report and can change the run's findings.

| Command | Effect of `partial` |
| --- | --- |
| `crawl` | Exit code still follows findings: exit 1 on an error finding, exit 0 if only warnings |
| `content --crawl-url` | Exits 2, whatever the gate results |
| `verify` | Refuses a partial baseline artifact unless `--allow-partial-baseline` is passed |

Only `max_pages` truncation emits `CRW003` and suppresses graph rules, because only
then are unvisited routes indistinguishable from missing ones.

An optional artifact such as `llms.txt` that cannot be fetched produces a `CRW002`
warning and does not affect the status.

Transport errors, HTTP 429 and 5xx are retried up to `--retry-budget` times with
exponential backoff from 250 ms to at most 5 s. `Retry-After` is not read. Other
statuses are not retried, and neither are refused fetches, whose outcome would not
change.

## Network boundary

A crawl fetches URLs the audited site chooses, through redirects, sitemaps and
manifests. These rules apply per hop, to robots.txt as much as to pages. `SECURITY.md`
lists their known limits.

- `headers`, `cookies` and `basic_auth` go only to the crawl origin: the same scheme,
  host and port, with `www.` equivalence only when scheme and port match. Other hosts
  get only the crawl's User-Agent. With the Playwright engine, the browser scopes
  cookies by host, not port.
- A redirect still gets followed off the origin, without credentials. Only `www.`
  canonicalization moves the crawl. A redirect to another scheme or port does not, so
  an `http://` target that redirects to `https://` requests each page through that
  redirect and loses credentials on it. Use the `https://` URL for an authenticated
  crawl.
- A sitemap on another site, or one that redirects there, gives a `CRW001` warning and
  is not read. Nesting stops at depth 5 and 1000 sitemaps.
- Loopback, private, link-local and unique-local addresses are refused, after DNS
  resolution. This is off when the target itself resolves only to such addresses, so
  local preview keeps working, or with `allow_private_network = true` under `[runtime]`.
- A response body over `max_response_bytes` under `[runtime]` (20 MiB by default) is
  a refused fetch. The Playwright engine does not apply this cap.
- A fetch of an optional artifact that the boundary refuses gives a warning: `CRW002`
  for `llms.txt` and `sitemap.xml`, `CRW001` for artifact probes. It does not affect
  the status.

## URLs in reports

Findings, response provenance, skipped URLs, timings, progress events and `site_url`
hold URLs reduced to scheme, host, port and path. Userinfo, query and fragment are
removed because they can carry credentials. Fetch failures are described by a fixed
summary rather than the underlying error text, which embeds the URL. A refusal by the
network boundary states its reason and names the URL in the same reduced form.

Checkpoints store URLs the same way. They still hold the crawled page bodies, so a
new checkpoint file is created readable only by its owner (mode 0600 on Unix); an
existing one keeps its mode.

## Checkpoints

`--resume` refuses a checkpoint whose version, base URL, host or engine differs from
the current run, before sending any request. Where a resumed crawl sends requests
comes only from the URL given on the command line, never from the file: it starts at
that URL's origin and follows redirects again. Credentials embedded in the URL apply
only until the crawl adopts the origin the server answered from, and are not used
after a resume. Checkpoints are written atomically and triggered once `--checkpoint-every` pages have been visited since the last one. A
route that did not fit in `--max-pages` stays queued, so a resume crawls it. robots.txt
is fetched again on resume.
