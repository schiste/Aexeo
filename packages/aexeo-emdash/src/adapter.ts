import type { EmdashDocument, PortableTextBlock } from "./types.js";

/** Ordered fields that the host declares constitute a custom block's content. */
export type CustomBlockField =
  | { kind: "paragraph" | "richText"; field: string; optional?: boolean }
  | { kind: "heading"; field: string; level?: 1 | 2 | 3 | 4 | 5 | 6; optional?: boolean }
  | { kind: "link"; field: string; hrefField: string; optional?: boolean }
  | { kind: "repeat"; field: string; fields: readonly CustomBlockField[]; optional?: boolean };
export type CustomBlockMappings = Record<string, readonly CustomBlockField[]>;

export interface EmdashContentItem {
  id: string;
  type: string;
  slug: string | null;
  status: string;
  locale: string | null;
  /** Host write timestamp. Orders concurrent saves and Refresh sweeps. */
  updatedAt?: string;
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
  /** The host's `updatedAt` for the row this entry was adapted from. */
  updatedAt?: string;
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
  blockMappings?: CustomBlockMappings,
  contentFields?: readonly string[],
  descriptionFields?: readonly string[],
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
    ...(blockMappings === undefined ? {} : { blockMappings }),
    ...(contentFields === undefined ? {} : { contentFields }),
    ...(descriptionFields === undefined ? {} : { descriptionFields }),
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
  blockMappings?: CustomBlockMappings,
): EmdashDocument {
  return adaptContentItem(content, { blockMappings }).document;
}

export function adaptContentItem(
  content: EmdashContentItem,
  options: {
    route?: string;
    publicUrl?: string | null;
    defaultLocale?: string;
    blockMappings?: CustomBlockMappings | undefined;
    contentFields?: readonly string[];
    descriptionFields?: readonly string[];
  } = {},
): AdaptedContent {
  const route = options.route ?? deriveRoute(content);
  const title = stringOrEmpty(
    content.seo?.title ?? content.data["title"] ?? content.slug ?? content.id,
  );
  const document: EmdashDocument = { route, title };
  const fields = options.descriptionFields;
  const mapped = fields !== undefined && fields.length > 0 &&
    fields.every((field) => typeof field === "string" && field.length > 0 &&
      (content.data[field] == null || typeof content.data[field] === "string"));
  if (mapped) document.description_source_observed = true;
  const description = fields === undefined
    ? content.seo?.description ?? content.data["description"]
    : mapped ? fields.map((field) => content.data[field]).find((value) => typeof value === "string" && value.length > 0) : undefined;
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
  const body = options.contentFields === undefined
    ? extractPortableText(content.data, options.blockMappings)
    : extractSelectedContent(content.data, options.contentFields, options.blockMappings);
  if (body !== null) {
    document.body = body.blocks;
    if (body.capabilities !== undefined) document.body_capabilities = body.capabilities;
  }

  return {
    document,
    meta: {
      id: content.id,
      collection: content.type,
      slug: content.slug,
      status: content.status,
      title,
      publicUrl: options.publicUrl ?? null,
      ...(content.updatedAt === undefined ? {} : { updatedAt: content.updatedAt }),
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

type ExtractedBody = { blocks: PortableTextBlock[]; capabilities?: ("text" | "headings" | "paragraphs" | "links")[] };

/** A selection asserts that the listed fields constitute the rendered body, in order. */
function extractSelectedContent(data: Record<string, unknown>, fields: readonly string[], mappings?: CustomBlockMappings): ExtractedBody | null {
  if (fields.length === 0 || new Set(fields).size !== fields.length || fields.some((field) => typeof field !== "string" || field.length === 0)) return null;
  const all = ["text", "headings", "paragraphs", "links"] as const;
  let capabilities: readonly (typeof all)[number][] = all;
  const blocks: PortableTextBlock[] = [];
  let observed = false;
  for (const field of fields) {
    const value = data[field];
    if (value === undefined || value === null) continue;
    observed = true;
    const part = extractFirstPortableText({ content: value }, mappings);
    if (part === null) {
      // An unrecognized field cannot establish coverage, but later selected
      // fields may still contain useful prose and must not be discarded.
      capabilities = [];
      continue;
    }
    blocks.push(...part.blocks);
    capabilities = capabilities.filter((capability) => part.capabilities === undefined || part.capabilities.includes(capability));
  }
  if (!observed) return null;
  return capabilities.length === all.length ? { blocks } : { blocks, capabilities: [...capabilities] };
}

function extractPortableText(
  data: Record<string, unknown>,
  mappings?: CustomBlockMappings,
): ExtractedBody | null {
  const result = extractFirstPortableText(data, mappings);
  if (result === null) return null;
  const candidates = Object.entries(data).filter(([field, value]) =>
    isPortableTextArray(value) || isMixedPortableTextArray(value) || isEmdashBlocksArray(value, field) ||
    ((field === "body" || field === "content") && typeof value === "string" && value.trim().length > 0),
  );
  // Automatically selecting one field does not prove the others are rendered
  // or irrelevant. Never present that projection as the complete document.
  return candidates.length > 1 ? { blocks: result.blocks, capabilities: [] } : result;
}

function extractFirstPortableText(
  data: Record<string, unknown>,
  mappings?: CustomBlockMappings,
): ExtractedBody | null {
  // Prefer established rich-text fields. EmDash's typed `blocks` field is a
  // separate composition format: its _type/_version/_key envelope does not
  // contain Portable Text's `children`, so passing it through loses the text.
  let foundEmptyBody = false;
  for (const slug of ["body", "content"]) {
    const value = data[slug];
    if ((Array.isArray(value) && value.length === 0) || (typeof value === "string" && value.trim().length === 0)) foundEmptyBody = true;
    if (isPortableTextArray(value)) return { blocks: value };
    if (isMixedPortableTextArray(value)) return mapMixedPortableText(value, mappings);
    if (isEmdashBlocksArray(value, slug)) return mapMixedPortableText(value, mappings);
    if (typeof value === "string" && value.trim().length > 0) {
      return { blocks: [portableTextBlock(value.trim(), `${slug}-text`)], capabilities: ["text"] };
    }
  }

  const blocks = data["blocks"];
  if (Array.isArray(blocks) && blocks.length === 0) foundEmptyBody = true;
  if (isPortableTextArray(blocks)) return { blocks };
  if (isMixedPortableTextArray(blocks)) return mapMixedPortableText(blocks, mappings);
  if (isEmdashBlocksArray(blocks, "blocks")) return mapMixedPortableText(blocks, mappings);

  for (const [field, value] of Object.entries(data)) {
    if (field === "body" || field === "content" || field === "blocks") continue;
    if (isPortableTextArray(value)) return { blocks: value };
    if (isMixedPortableTextArray(value)) return mapMixedPortableText(value, mappings);
    if (isEmdashBlocksArray(value, field)) return mapMixedPortableText(value, mappings);
  }
  return foundEmptyBody ? { blocks: [] } : null;
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

/** Portable Text can interleave ordinary text blocks with custom components. */
function isMixedPortableTextArray(value: unknown): value is unknown[] {
  // One valid block identifies the format. Other items can be malformed or
  // unknown; preserve the valid neighbors without asserting full coverage.
  return Array.isArray(value) && value.some((item) => isPortableTextArray([item]));
}

function mapMixedPortableText(blocks: unknown[], mappings?: CustomBlockMappings, depth = 0): ExtractedBody {
  if (depth > 8) return { blocks: [], capabilities: [] };
  const result: PortableTextBlock[] = [];
  let complete = true;
  for (const block of blocks) {
    if (isPortableTextArray([block])) {
      result.push(block as unknown as PortableTextBlock);
      continue;
    }
    if (!isRecord(block) || typeof block["_type"] !== "string" || block["_type"] === "block") {
      complete = false;
      continue;
    }
    const mapped = mapCustomBlocks([block], mappings, depth + 1);
    result.push(...mapped.blocks);
    if (mapped.capabilities !== undefined) complete = false;
  }
  return complete ? { blocks: result } : { blocks: result, capabilities: [] };
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
      (typeof item["_version"] === "number" || field === "blocks" || field === "body" || field === "content"),
  );
}

/** A mapping is a host assertion that its ordered fields cover the rendered body.
 * Unknown block types or malformed values fall back to unverified content. */
function mapCustomBlocks(
  blocks: Array<Record<string, unknown>>,
  mappings: CustomBlockMappings | undefined,
  depth = 0,
): { blocks: PortableTextBlock[]; capabilities?: [] } {
  if (depth > 8) return { blocks: [], capabilities: [] };
  const unavailable = () => ({ blocks: blocksToPortableText(blocks), capabilities: [] as [] });
  const result: PortableTextBlock[] = [];
  let complete = true;
  const mapFields = (record: Record<string, unknown>, fields: readonly CustomBlockField[], depth: number): boolean => {
    if (depth > 8 || fields.length === 0) return false;
    for (const entry of fields) {
      if (!entry || typeof entry.field !== "string" || !entry.field ||
        !["heading", "paragraph", "richText", "link", "repeat"].includes(entry.kind)) return false;
      if (entry.kind === "heading" && entry.level !== undefined && ![1, 2, 3, 4, 5, 6].includes(entry.level)) return false;
      if (entry.kind === "link" && (typeof entry.hrefField !== "string" || !entry.hrefField)) return false;
      if (entry.kind === "repeat" && (!Array.isArray(entry.fields) || entry.fields.length === 0)) return false;
      const value = record[entry.field];
      if ((value === undefined || value === null || value === "") && entry.optional) {
        // Optional links must not silently discard a destination without its label.
        if (entry.kind === "link" && record[entry.hrefField] != null && record[entry.hrefField] !== "") return false;
        continue;
      }
      if (entry.kind === "richText") {
        if (!Array.isArray(value)) return false;
        if (value.length === 0 || isPortableTextArray(value)) {
          result.push(...value);
        } else if (isMixedPortableTextArray(value) || isEmdashBlocksArray(value, "blocks")) {
          const nested = mapMixedPortableText(value, mappings, depth + 1);
          result.push(...nested.blocks);
          if (nested.capabilities !== undefined) complete = false;
        } else {
          return false;
        }
      } else if (entry.kind === "heading" || entry.kind === "paragraph") {
        if (typeof value !== "string") return false;
        result.push({ ...portableTextBlock(value, ""), style: entry.kind === "heading" ? `h${entry.level ?? 2}` : "normal" });
      } else if (entry.kind === "link") {
        const href = record[entry.hrefField];
        if (typeof value !== "string" || typeof href !== "string" || !href) return false;
        result.push({
          _type: "block",
          children: [{ _type: "span", text: value, marks: ["link"] }],
          markDefs: [{ _key: "link", _type: "link", href }],
        });
      } else if (entry.kind === "repeat") {
        if (!Array.isArray(value) || !Array.isArray(entry.fields)) return false;
        for (const item of value) {
          if (!isRecord(item) || !mapFields(item, entry.fields, depth + 1)) return false;
        }
      } else {
        return false;
      }
    }
    return true;
  };
  for (const block of blocks) {
    const fields = mappings?.[String(block["_type"])];
    if (!Array.isArray(fields)) return unavailable();
    // An empty mapping explicitly declares a component irrelevant to the body.
    if (fields.length === 0) continue;
    if (!mapFields(block, fields, depth)) return unavailable();
  }
  return complete ? { blocks: result } : { blocks: result, capabilities: [] };
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

// Fallback text is only a partial, unverified projection. Bound each unknown
// component so malformed or deeply nested host data cannot monopolize a save.
const FALLBACK_MAX_DEPTH = 64;
const FALLBACK_MAX_NODES = 50_000;
const FALLBACK_MAX_CHARS = 1_048_576;

function collectText(value: unknown, key: string, out: string[]): void {
  const active = new WeakSet<object>();
  let nodes = 0;
  let remaining = FALLBACK_MAX_CHARS;
  const visit = (current: unknown, field: string, depth: number): void => {
    if (depth > FALLBACK_MAX_DEPTH || nodes >= FALLBACK_MAX_NODES || remaining <= 0) return;
    nodes++;
    if (typeof current === "string") {
      if (!isStructuralField(field)) {
        const separator = out.length > 0 ? 1 : 0;
        const limit = Math.max(0, remaining - separator);
        let text = current.slice(0, limit);
        // Do not create an invalid lone surrogate when the budget cuts an emoji.
        if (limit < current.length && /[\uD800-\uDBFF]/.test(text.at(-1) ?? "") &&
            /[\uDC00-\uDFFF]/.test(current[limit] ?? "")) text = text.slice(0, -1);
        text = text.trim();
        if (text.length > 0) {
          out.push(text);
          remaining -= text.length + separator;
        }
      }
      return;
    }
    if (!Array.isArray(current) && !isRecord(current)) return;
    if (active.has(current)) return;
    active.add(current);
    if (Array.isArray(current)) {
      for (const item of current) {
        if (nodes >= FALLBACK_MAX_NODES || remaining <= 0) break;
        visit(item, field, depth + 1);
      }
    } else {
      for (const childKey in current) {
        if (nodes >= FALLBACK_MAX_NODES || remaining <= 0) break;
        if (Object.hasOwn(current, childKey) && !isStructuralField(childKey)) {
          visit(current[childKey], childKey, depth + 1);
        }
      }
    }
    active.delete(current);
  };
  visit(value, key, 0);
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
