# Security Policy

## Reporting

Do not open a public issue for a suspected vulnerability.

Use GitHub private vulnerability reporting for this repository if it is enabled. If private reporting is unavailable, contact the maintainers directly before publishing any details.

## Scope

Security reports are especially useful for:

- credential or token exposure
- unsafe file writes or code execution paths
- SSRF, auth, or access-control issues in the runtime and worker packages
- supply-chain or release-integrity issues affecting published artifacts

## Runtime crawl trust boundary

A crawl fetches URLs chosen by the site it audits, so `aexeo crawl` applies these rules to every hop, including redirects:

- Configured credentials (`[runtime] headers`, `cookies`, `basic_auth`) go only to the crawl origin: same scheme, host and port, with `www.` equivalence only when scheme and port match. An `https` to `http` downgrade or a port change drops them, and neither moves the crawl to the new origin. A `cookies` entry with an explicit `domain` or `url` is scoped by the browser engine to what it names.
- Sitemaps on another site are refused and reported as `CRW001` warnings. Nesting is capped at depth 5 and 1000 sitemaps.
- Loopback, private, link-local (including `169.254.169.254`) and unique-local addresses are refused after DNS resolution, unless the crawl target itself resolves only to such addresses (local preview) or `[runtime] allow_private_network = true`.
- These rules cover robots.txt too. A refused robots.txt means everything is disallowed, as RFC 9309 requires for unknown rules.
- HTTP response bodies are capped by `[runtime] max_response_bytes` (20 MiB by default). A larger response is a recorded fetch failure.
- Secret-bearing response headers such as `set-cookie` are redacted before capture. New checkpoints and audit artifacts are created with mode 0600 on Unix; an existing file keeps its mode. URL userinfo, queries and fragments are left out of the messages this boundary produces. Playwright traces, when enabled, record request headers, and their directory is created with mode 0700.

Known limits:

- The HTTP engine pins connections to checked addresses in its resolver, so DNS rebinding does not defeat it. A configured HTTP proxy resolves names itself, so behind a proxy only IP-literal URLs are checked.
- The Playwright engine checks each request's host with its own DNS lookup, cached per crawl, while Chromium resolves separately. A host that rebinds between the two is not caught. WebSocket connections and requests made by popups or workers are not intercepted, and they carry no configured headers.
- In the Playwright engine, cookies are held by the browser, which scopes them by host and, for `https` targets, the `Secure` flag, not by port. A cookie for `http://example.com` also reaches `http://example.com:81`.
- A target whose DNS answers are non-public when the crawl starts is treated as local preview for that crawl. Its operator can use this to turn private-network blocking off.

## Response

Reports will be triaged privately. When a fix ships, the public advisory should include affected versions, mitigation steps, and upgrade guidance.
