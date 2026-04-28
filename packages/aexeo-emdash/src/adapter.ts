import type { EmdashDocument, PortableTextBlock } from "./types.js";

// Adapts emdash's ContentItem (the shape passed to content:afterSave)
// into the EmdashDocument shape the seogeo WASM bridge expects.
//
// emdash's ContentItem is the storage row:
//   { id, type, slug, status, locale, data: Record<string, unknown>, seo?, ... }
//
// The WASM bridge wants the rendered page's intent:
//   { route, title, description?, lang?, body?, ... }
//
// We map field-by-field with defensive fallbacks — content.data is a
// schema-driven blob with no compile-time guarantee about which fields
// exist, so anything missing falls back to a neutral default rather
// than throwing. The blog template's posts collection happens to use
// `title` and `body` slugs which is what most starter schemas pick;
// extending the mapping for less common slug names is a follow-up
// once we see real-world schemas drift.

export interface EmdashContentItem {
  id: string;
  type: string;
  slug: string | null;
  status: string;
  locale: string | null;
  data: Record<string, unknown>;
  seo?: {
    title?: string | null;
    description?: string | null;
    canonical?: string | null;
  };
}

// Metadata about the source ContentItem that the WASM bridge doesn't
// need but the admin UI does — enough to construct edit URLs into
// emdash, public URLs into the deployed site, and to distinguish
// drafts from published content. Stored alongside the EmdashDocument
// in KV (see StoredDocument in plugin.ts).
export interface EmdashContentMeta {
  id: string;
  collection: string;
  slug: string | null;
  status: string;
  // Hydrated from the same content.data extraction the document uses,
  // so the admin can render a human-readable label even when the
  // route is a slugless fallback like "/posts/<id>".
  title: string;
}

export interface AdaptedContent {
  document: EmdashDocument;
  meta: EmdashContentMeta;
}

export function contentItemToEmdashDocument(
  content: EmdashContentItem,
): EmdashDocument {
  return adaptContentItem(content).document;
}

export function adaptContentItem(content: EmdashContentItem): AdaptedContent {
  const route = deriveRoute(content);
  const title = stringOrEmpty(
    content.seo?.title ?? content.data["title"] ?? content.slug ?? content.id,
  );
  const document: EmdashDocument = {
    route,
    title,
  };
  const description = content.seo?.description ?? content.data["description"];
  if (typeof description === "string" && description.length > 0) {
    document.description = description;
  }
  const canonical = content.seo?.canonical;
  if (typeof canonical === "string" && canonical.length > 0) {
    document.canonical = canonical;
  }
  if (content.locale !== null) {
    document.lang = content.locale;
  }
  const body = extractPortableText(content.data);
  if (body !== null) {
    document.body = body;
  }
  return {
    document,
    meta: {
      id: content.id,
      collection: content.type,
      slug: content.slug,
      status: content.status,
      title,
    },
  };
}

function deriveRoute(content: EmdashContentItem): string {
  // Most emdash blog templates serve at /<slug>; collections without a
  // slug fall back to the row id so we still produce a unique key for
  // KV storage. The WASM bridge treats the route as opaque.
  // Note: the typed shape is `string | null`, but newly-created
  // ContentItems may have slug undefined when the schema's slug field
  // hasn't been populated yet. Truthy check covers both null and
  // undefined; an empty string falls through to the id fallback too.
  if (typeof content.slug === "string" && content.slug.length > 0) {
    return content.slug.startsWith("/") ? content.slug : `/${content.slug}`;
  }
  return `/${content.type}/${content.id}`;
}

function stringOrEmpty(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function extractPortableText(
  data: Record<string, unknown>,
): PortableTextBlock[] | null {
  // Walk well-known field slugs first. Some schemas use `body`, some
  // `content`. Anything else with an array-of-objects-with-_type
  // signature is also acceptable as a Portable Text blob.
  const candidates = ["body", "content"];
  for (const slug of candidates) {
    const value = data[slug];
    if (isPortableTextArray(value)) {
      return value;
    }
  }
  for (const value of Object.values(data)) {
    if (isPortableTextArray(value)) {
      return value;
    }
  }
  return null;
}

function isPortableTextArray(value: unknown): value is PortableTextBlock[] {
  if (!Array.isArray(value) || value.length === 0) {
    return false;
  }
  const first = value[0];
  return (
    typeof first === "object" &&
    first !== null &&
    "_type" in first &&
    typeof (first as { _type: unknown })._type === "string"
  );
}
