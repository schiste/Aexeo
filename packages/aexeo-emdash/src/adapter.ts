import type { EmdashDocument, PortableTextBlock } from "./types.js";

export interface EmdashContentItem {
  id: string;
  type: string;
  slug: string | null;
  status: string;
  locale: string | null;
  translationGroup?: string | null;
  data: Record<string, unknown>;
  seo?: {
    title?: string | null;
    description?: string | null;
    canonical?: string | null;
  };
}

export interface EmdashContentMeta {
  id: string;
  collection: string;
  slug: string | null;
  status: string;
  title: string;
  /** Exact URL returned by EmDash, or null when this entry has no public route. */
  publicUrl: string | null;
}

export interface ContentUrlApi {
  getPublicUrl(collection: string, id: string): Promise<string | null>;
  getTranslations(
    collection: string,
    id: string,
  ): Promise<{
    translationGroup: string;
    translations: Array<{
      id: string;
      locale: string | null;
      slug: string | null;
      status: string;
      updatedAt: string;
    }>;
  }>;
}

export interface AdaptedContent {
  document: EmdashDocument;
  meta: EmdashContentMeta;
}

/**
 * Adapt a stored content row using EmDash's route resolver. The host API
 * accounts for collection URL patterns, locale prefixes, and slash policy;
 * slug-derived paths are not equivalent to the site's actual public URLs.
 */
export async function adaptContentItemWithContext(
  content: EmdashContentItem,
  contentApi: ContentUrlApi,
  defaultLocale?: string,
): Promise<AdaptedContent> {
  const publicUrl =
    content.status === "published"
      ? await contentApi.getPublicUrl(content.type, content.id)
      : null;
  const route =
    publicUrl === null ? deriveRoute(content) : routeFromPublicUrl(publicUrl);
  const adapted = adaptContentItem(content, {
    route,
    publicUrl,
    ...(defaultLocale === undefined ? {} : { defaultLocale }),
  });

  const translationSet = await contentApi.getTranslations(
    content.type,
    content.id,
  );
  const alternates = await Promise.all(
    translationSet.translations
      .filter(
        (translation) =>
          translation.status === "published" &&
          typeof translation.locale === "string" &&
          translation.locale.length > 0,
      )
      .map(async (translation) => {
        const href =
          translation.id === content.id
            ? publicUrl
            : await contentApi.getPublicUrl(content.type, translation.id);
        if (href === null) return null;
        return { lang: translation.locale!, href };
      }),
  );
  const uniqueAlternates = new Map<string, { lang: string; href: string }>();
  for (const alternate of alternates) {
    if (alternate !== null) uniqueAlternates.set(alternate.lang, alternate);
  }
  if (uniqueAlternates.size > 0) {
    adapted.document.alternates = [...uniqueAlternates.values()];
  }

  return adapted;
}

/**
 * Synchronous adapter for callers that do not have a host content API.
 * It deliberately uses a stable identity route; callers should use
 * adaptContentItemWithContext whenever EmDash can resolve the public URL.
 */
export function contentItemToEmdashDocument(
  content: EmdashContentItem,
): EmdashDocument {
  return adaptContentItem(content).document;
}

export function adaptContentItem(
  content: EmdashContentItem,
  options: {
    route?: string;
    publicUrl?: string | null;
    defaultLocale?: string;
  } = {},
): AdaptedContent {
  const route = options.route ?? deriveRoute(content);
  const title = stringOrEmpty(
    content.seo?.title ?? content.data["title"] ?? content.slug ?? content.id,
  );
  const document: EmdashDocument = { route, title };
  const description = content.seo?.description ?? content.data["description"];
  if (typeof description === "string" && description.length > 0) {
    document.description = description;
  }
  const canonical = content.seo?.canonical;
  if (typeof canonical === "string" && canonical.length > 0) {
    document.canonical = canonical;
  }
  const locale = content.locale || options.defaultLocale;
  if (typeof locale === "string" && locale.length > 0) {
    document.lang = locale;
  }
  const body = extractPortableText(content.data);
  if (body !== null) document.body = body;

  return {
    document,
    meta: {
      id: content.id,
      collection: content.type,
      slug: content.slug,
      status: content.status,
      title,
      publicUrl: options.publicUrl ?? null,
    },
  };
}

function deriveRoute(content: EmdashContentItem): string {
  // Drafts and published entries without a routable URL still need distinct
  // storage keys. IDs avoid collisions between locales and custom URL patterns.
  return `/${encodeURIComponent(content.type)}/${encodeURIComponent(content.id)}`;
}

function routeFromPublicUrl(publicUrl: string): string {
  try {
    return new URL(publicUrl).pathname || "/";
  } catch (cause) {
    throw new Error(`EmDash returned an invalid public URL: ${publicUrl}`, {
      cause,
    });
  }
}

function stringOrEmpty(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function extractPortableText(
  data: Record<string, unknown>,
): PortableTextBlock[] | null {
  // Prefer established rich-text fields. EmDash's typed `blocks` field is a
  // separate composition format: its _type/_version/_key envelope does not
  // contain Portable Text's `children`, so passing it through loses the text.
  for (const slug of ["body", "content"]) {
    const value = data[slug];
    if (isPortableTextArray(value)) return value;
    if (isEmdashBlocksArray(value, slug)) return blocksToPortableText(value);
    if (typeof value === "string" && value.trim().length > 0) {
      return [portableTextBlock(value.trim(), `${slug}-text`)];
    }
  }

  const blocks = data["blocks"];
  if (isPortableTextArray(blocks)) return blocks;
  if (isEmdashBlocksArray(blocks, "blocks")) return blocksToPortableText(blocks);

  for (const [field, value] of Object.entries(data)) {
    if (field === "body" || field === "content" || field === "blocks") continue;
    if (isPortableTextArray(value)) return value;
    if (isEmdashBlocksArray(value, field)) return blocksToPortableText(value);
  }
  return null;
}

function isPortableTextArray(value: unknown): value is PortableTextBlock[] {
  return (
    Array.isArray(value) &&
    value.length > 0 &&
    value.every(
      (item) =>
        isRecord(item) &&
        item["_type"] === "block" &&
        Array.isArray(item["children"]) &&
        item["children"].every(
          (child) => isRecord(child) && typeof child["text"] === "string",
        ),
    )
  );
}

function isEmdashBlocksArray(
  value: unknown,
  field: string,
): value is Array<Record<string, unknown>> {
  if (!Array.isArray(value) || value.length === 0) return false;
  return value.every(
    (item) =>
      isRecord(item) &&
      typeof item["_type"] === "string" &&
      typeof item["_key"] === "string" &&
      (typeof item["_version"] === "number" || field === "blocks"),
  );
}

function blocksToPortableText(
  blocks: Array<Record<string, unknown>>,
): PortableTextBlock[] {
  const paragraphs: PortableTextBlock[] = [];
  for (const block of blocks) {
    const segments: string[] = [];
    collectText(block, "", segments);
    const text = segments.map((segment) => segment.trim()).filter(Boolean).join(" ");
    if (text.length > 0) {
      paragraphs.push(portableTextBlock(text, stringOrEmpty(block["_key"])));
    }
  }
  return paragraphs;
}

function collectText(value: unknown, key: string, out: string[]): void {
  if (typeof value === "string") {
    if (!isStructuralField(key) && value.trim().length > 0) out.push(value);
    return;
  }
  if (Array.isArray(value)) {
    for (const item of value) collectText(item, key, out);
    return;
  }
  if (!isRecord(value)) return;
  for (const [childKey, childValue] of Object.entries(value)) {
    if (isStructuralField(childKey)) continue;
    collectText(childValue, childKey, out);
  }
}

const STRUCTURAL_FIELDS = new Set([
  "_key",
  "_ref",
  "_type",
  "_version",
  "asset",
  "align",
  "alignment",
  "autoplay",
  "color",
  "controls",
  "focalpoint",
  "height",
  "href",
  "id",
  "layout",
  "level",
  "loop",
  "markdefs",
  "marks",
  "mime_type",
  "mimetype",
  "position",
  "rel",
  "size",
  "src",
  "style",
  "target",
  "theme",
  "tone",
  "url",
  "variant",
  "width",
]);

function isStructuralField(key: string): boolean {
  const normalized = key.toLowerCase();
  return normalized.startsWith("_") || STRUCTURAL_FIELDS.has(normalized);
}

function portableTextBlock(text: string, key: string): PortableTextBlock {
  return {
    _type: "block",
    ...(key.length === 0 ? {} : { _key: key }),
    children: [
      {
        _type: "span",
        ...(key.length === 0 ? {} : { _key: `${key}-text` }),
        text,
        marks: [],
      },
    ],
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
