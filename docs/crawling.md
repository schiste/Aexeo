# Live crawl behaviour

How `crawl`, `verify`, `profile runtime`, `perf baseline` and `content --crawl-url`
decide what to request and how they report the result.

## Link discovery

Links are read from the parsed DOM and resolved against the observed page URL, as
a browser would: relative and parent paths, unquoted attributes and `<base href>`
all work. Only `<a href>` is followed; `data-href` and markup inside scripts are not.

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

A site with one broken link therefore ends `complete` and exits 1. If every requested
URL returned an error status, nothing was audited and the status is `partial`.

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
statuses are not retried.

## URLs in reports

Findings, response provenance, skipped URLs, timings, progress events and `site_url`
hold URLs reduced to scheme, host, port and path. Userinfo, query and fragment are
removed because they can carry credentials. Fetch failures are described by a fixed
summary rather than the underlying error text, which embeds the URL.

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
