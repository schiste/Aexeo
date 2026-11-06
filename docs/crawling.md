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
| 429, 5xx or a network failure | Everything is disallowed, with a `CRW002` warning |

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

`complete` requires that the crawl was not truncated and that no fetch failed.
Otherwise the status is `partial` and `truncation_reason` says why.

A fetch fails on a transport error, an unreadable body or a non-2xx status. Each
failure produces a `CRW001` finding. A non-2xx response is not a page: it is not
audited, it is recorded in `site.crawl_meta.failed_responses`, and links to its
route are reported by `LNK001`. Consequently one broken internal link makes a crawl
`partial`.

| Command | Effect of `partial` |
| --- | --- |
| `crawl` | Exit code still follows findings; `CRW001` is an error, so a failed fetch exits 1 |
| `content --crawl-url` | Exits 2, whatever the gate results |
| `verify` | Refuses a partial baseline artifact unless `--allow-partial-baseline` is passed |

Only `max_pages` truncation emits `CRW003` and suppresses graph rules, because only
then are unvisited routes indistinguishable from missing ones.

An optional artifact such as `llms.txt` that cannot be fetched produces a `CRW002`
warning and does not affect the status.

Transport errors are retried up to `--retry-budget` times with exponential backoff
from 250 ms to at most 5 s. Non-2xx responses are not retried.

## URLs in reports

Findings, response provenance, skipped URLs, timings, progress events and `site_url`
hold URLs reduced to scheme, host, port and path. Userinfo, query and fragment are
removed because they can carry credentials. Fetch failures are described by a fixed
summary rather than the underlying error text, which embeds the URL.

A checkpoint file is not a report: it keeps the base URL as given, so that a resume
can use it. Treat a checkpoint for a URL with embedded credentials as a secret.

## Checkpoints

`--resume` refuses a checkpoint whose version, base URL, host or engine differs from
the current run, before sending any request. Checkpoints are written atomically and
triggered once `--checkpoint-every` pages have been visited since the last one. A
route that did not fit in `--max-pages` stays queued, so a resume crawls it. robots.txt
is fetched again on resume.
