// IndexNow submission for the publish hook. Workers have native fetch
// so we do not pull in any HTTP client; the aexeo-cli ledger model is
// reused by storing one entry per submission attempt in KV.

const INDEXNOW_ENDPOINT = "https://api.indexnow.org/indexnow";

export interface IndexNowConfig {
  // Public-facing site URL, used as the "host" field and to validate
  // that submitted URLs belong to this property.
  siteUrl: string;
  // The IndexNow key. Must already be served at keyLocation.
  key: string;
  // Optional override for the location of the key file. Defaults to
  // <siteUrl>/<key>.txt, matching aexeo-cli's default contract.
  keyLocation?: string;
}

export interface IndexNowSubmission {
  ok: boolean;
  status: number;
  endpoint: string;
  submitted: string[];
  rejected: string[];
  reason?: string;
}

export async function submitIndexNow(
  config: IndexNowConfig,
  urls: readonly string[],
): Promise<IndexNowSubmission> {
  const host = hostOf(config.siteUrl);
  if (host === null) {
    return failure("siteUrl is not a valid URL", []);
  }
  const keyLocation = config.keyLocation ?? defaultKeyLocation(config);
  const { submitted, rejected } = partitionByHost(urls, host);
  if (submitted.length === 0) {
    return failure("no submittable URLs after host check", rejected);
  }
  const payload = {
    host,
    key: config.key,
    keyLocation,
    urlList: submitted,
  };
  const response = await fetch(INDEXNOW_ENDPOINT, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(payload),
  });
  return {
    ok: response.ok,
    status: response.status,
    endpoint: INDEXNOW_ENDPOINT,
    submitted,
    rejected,
  };
}

function hostOf(rawUrl: string): string | null {
  try {
    return new URL(rawUrl).host;
  } catch {
    return null;
  }
}

function defaultKeyLocation(config: IndexNowConfig): string {
  const siteUrl = config.siteUrl.endsWith("/")
    ? config.siteUrl.slice(0, -1)
    : config.siteUrl;
  return `${siteUrl}/${config.key}.txt`;
}

function partitionByHost(
  urls: readonly string[],
  expectedHost: string,
): { submitted: string[]; rejected: string[] } {
  const submitted: string[] = [];
  const rejected: string[] = [];
  for (const candidate of urls) {
    const host = hostOf(candidate);
    if (host === expectedHost) {
      submitted.push(candidate);
    } else {
      rejected.push(candidate);
    }
  }
  return { submitted, rejected };
}

function failure(reason: string, rejected: string[]): IndexNowSubmission {
  return {
    ok: false,
    status: 0,
    endpoint: INDEXNOW_ENDPOINT,
    submitted: [],
    rejected,
    reason,
  };
}
