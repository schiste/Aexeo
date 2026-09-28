// Validation for the sidecar URL pasted on the plugin's Setup page.
//
// This form is the only place a user-supplied string becomes the target
// of an outbound fetch: `handleSetupSubmit` stores it, `sidecar.ts` later
// appends `/evaluate` and sends `Authorization: Bearer <token>` to it.
// Two things follow from that and are enforced here:
//
//   1. The scheme must be https. sidecar.ts puts the bearer token in a
//      request header, so an http: URL puts a live credential on the wire
//      in cleartext for anyone on the path.
//
//   2. The host must not be a loopback, private, or link-local address.
//      emdash's sandbox bridge has its own SSRF blocklist, but relying on
//      it means the form happily stores a URL that then fails on every
//      Refresh — and if the bridge's list ever moves, a private-range URL
//      here is a credentialed SSRF straight into the host's network.
//
// The third check is the plugin's own outbound allowlist. `buildAllowedHosts`
// (see plugin.ts) is baked into the sandbox descriptor at build time and is
// the only host emdash will let the plugin fetch. Validating the submitted
// hostname against it here means the form cannot store a URL the plugin is
// guaranteed to reject later.
//
// Everything is a pure function of its arguments so the rules are unit
// testable without a workerd host.

export type SidecarUrlValidation =
  | { ok: true; url: string }
  | { ok: false; error: string };

/**
 * Validate a pasted sidecar URL.
 *
 * `allowedHosts` is the descriptor's outbound allowlist. An empty list means
 * the allowlist could not be resolved at runtime (see `runtimeAllowedHosts`
 * in sandbox-entry.ts); the host check is then skipped rather than locking
 * the operator out of a sidecar they configured through the factory option.
 * Every other check always applies.
 */
export function validateSidecarUrl(
  raw: string,
  allowedHosts: readonly string[] = [],
): SidecarUrlValidation {
  const url = raw.trim();
  if (url.length === 0) {
    return { ok: false, error: "Sidecar URL is required." };
  }

  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return { ok: false, error: `Sidecar URL is not a valid URL: ${url}` };
  }

  if (parsed.protocol !== "https:") {
    return {
      ok: false,
      error:
        `Sidecar URL must be https: — the EVAL_TOKEN is sent as a bearer ` +
        `header and must never travel in cleartext. Got ${parsed.protocol}`,
    };
  }

  const host = parsed.hostname.toLowerCase();
  if (isBlockedHost(host)) {
    return {
      ok: false,
      error:
        "Sidecar URL must not point at a loopback, private, or link-local " +
        "address. Use the deployed *.workers.dev URL.",
    };
  }

  if (allowedHosts.length > 0 && !hostMatchesAllowlist(host, allowedHosts)) {
    return {
      ok: false,
      error:
        `Sidecar host "${host}" is not in this plugin's outbound allowlist ` +
        `(${allowedHosts.join(", ")}). The emdash sandbox would reject the ` +
        `fetch with "Host not allowed" — update evaluatorHost in ` +
        `astro.config and rebuild, or use that host.`,
    };
  }

  return { ok: true, url };
}

// Hostnames that name the local machine rather than a routable host.
// `0.0.0.0` is caught by the IPv4 rule below, not here.
function isBlockedHost(host: string): boolean {
  if (host === "localhost" || host.endsWith(".localhost")) {
    return true;
  }
  if (host.startsWith("[")) {
    return isBlockedIpv6(host);
  }
  return isBlockedIpv4(host);
}

// `new URL()` normalizes IPv4 literals before we ever see them, so
// `http://2130706433/` and `http://0x7f.1/` both arrive as `127.0.0.1`
// and a plain dotted-quad match is sufficient.
function isBlockedIpv4(host: string): boolean {
  const match = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/.exec(host);
  if (match === null) {
    // Not an IP literal. Real DNS names other than localhost/* are not
    // classified here: a name that resolves to a private address is the
    // host bridge's problem (it resolves and checks), and blocking names
    // by pattern would only produce false positives.
    return false;
  }
  const first = Number(match[1]);
  const second = Number(match[2]);
  if (first === 0) return true; // 0.0.0.0/8, "this host"
  if (first === 127) return true; // 127.0.0.0/8 loopback
  if (first === 10) return true; // RFC1918
  if (first === 172 && second >= 16 && second <= 31) return true; // RFC1918
  if (first === 192 && second === 168) return true; // RFC1918
  if (first === 169 && second === 254) return true; // link-local
  // 169.254.169.254 is the cloud instance metadata endpoint and is the
  // single most valuable SSRF target on a deployed Worker, which is why
  // the whole /16 is refused rather than just that one address.
  if (first === 100 && second >= 64 && second <= 127) return true; // RFC6598 CGNAT
  if (first >= 224) return true; // multicast, reserved, broadcast
  return false;
}

// URL serialization brackets IPv6 hosts, so `https://[::1]/` arrives with
// `hostname === "[::1]"`. IPv4-mapped addresses are normalized to their
// canonical hex form (`[::ffff:127.0.0.1]` becomes `[::ffff:7f00:1]`), so the
// `::ffff:` prefix covers every spelling of a mapped address; the
// `::7f00:1` family is the deprecated IPv4-compatible form of the same idea.
function isBlockedIpv6(host: string): boolean {
  const inner = host.slice(1, -1).toLowerCase();
  if (inner === "::" || inner === "::1") {
    return true; // unspecified address and loopback
  }
  if (inner.startsWith("::ffff:") || inner.startsWith("::7f")) {
    return true; // IPv4-mapped / IPv4-compatible, incl. 127.0.0.1
  }
  const firstHextet = Number.parseInt(inner.split(":")[0] ?? "", 16);
  if (Number.isNaN(firstHextet)) {
    return false;
  }
  if ((firstHextet & 0xfe00) === 0xfc00) {
    return true; // fc00::/7 unique local
  }
  if ((firstHextet & 0xffc0) === 0xfe80) {
    return true; // fe80::/10 link-local
  }
  return false;
}

// emdash documents `allowedHosts` as supporting `*.example.com` wildcards, so
// honor the same shape here. `buildAllowedHosts` only ever emits exact hosts
// today, but the descriptor field is user-authored and can carry a wildcard.
function hostMatchesAllowlist(
  host: string,
  allowedHosts: readonly string[],
): boolean {
  return allowedHosts.some((allowed) => {
    const entry = allowed.trim().toLowerCase();
    if (entry.length === 0) {
      return false;
    }
    if (entry.startsWith("*.")) {
      const suffix = entry.slice(1); // ".example.com"
      return host.endsWith(suffix) && host.length > suffix.length;
    }
    return host === entry;
  });
}
