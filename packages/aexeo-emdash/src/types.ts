// These types mirror the Rust serde wire format emitted by the
// aexeo-emdash-bridge crate. Field names follow what serde actually
// serializes, not what the Rust struct fields look like: _key, listItem,
// markDefs are the JSON shape, rule_id and snake_case scope values come
// from aexeo-contracts. Keep them in sync when the bridge's types move.

export type BlockStyle =
  | "normal"
  | "h1"
  | "h2"
  | "h3"
  | "h4"
  | "h5"
  | "h6"
  | "blockquote";

export type ListItem = "bullet" | "number";

export interface PortableTextSpan {
  _type: "span";
  _key?: string;
  text: string;
  marks: string[];
}

export interface MarkDef {
  _key: string;
  _type: string;
  [key: string]: unknown;
}

export type PortableTextChild = PortableTextSpan;

export interface PortableTextBlock {
  _type?: "block";
  _key?: string;
  style?: BlockStyle;
  listItem?: ListItem;
  level?: number;
  children: PortableTextChild[];
  markDefs?: MarkDef[];
}

export interface HreflangAlternate {
  lang: string;
  href: string;
}

export interface EmdashDocument {
  route: string;
  title: string;
  description?: string;
  canonical?: string;
  lang?: string;
  alternates?: HreflangAlternate[];
  meta?: Record<string, string>;
  schema?: unknown[];
  body?: PortableTextBlock[];
}

export type FindingSeverity = "error" | "warning";

export type FindingScope = "page" | "template" | "sitewide";

/// The four-layer GEO model. Mirror of aexeo-contracts::Layer.
/// The bridge enriches each Finding with its rule's primary + secondary
/// layers; the plugin admin groups findings by primary layer for the
/// four-pillar UI restructure.
export type Layer =
  | "retrievability"
  | "citability"
  | "absorbability"
  | "entity_legitimacy";

export interface RuleLayers {
  primary: Layer;
  secondaries: Layer[];
}

export interface Finding {
  rule_id: string;
  message: string;
  path: string;
  line: number;
  column: number;
  severity: FindingSeverity;
  suggestion: string | null;
  scope: FindingScope;
  /// Populated by the bridge when emitting findings via
  /// evaluateDocuments. Optional in the type so legacy KV entries
  /// (written before the bridge enrichment landed) still parse.
  layers?: RuleLayers;
}

/// Stable display order for the four pillars. Used by the plugin's
/// admin pages and dashboard widget. Mirrors Layer::ordered() in Rust.
export const LAYERS_ORDERED: readonly Layer[] = [
  "retrievability",
  "citability",
  "absorbability",
  "entity_legitimacy",
] as const;

export function layerHumanLabel(layer: Layer): string {
  switch (layer) {
    case "retrievability":
      return "Retrievability";
    case "citability":
      return "Citability";
    case "absorbability":
      return "Absorbability";
    case "entity_legitimacy":
      return "Entity legitimacy";
  }
}

export function layerOneLineDescription(layer: Layer): string {
  switch (layer) {
    case "retrievability":
      return "Can the engine find the page at all? Robots, sitemap, internal links, machine-readable surfaces.";
    case "citability":
      return "Once retrieved, does it look worth citing? Structure, schema, evidence density, scannability.";
    case "absorbability":
      return "Does the answer actually use this content? Cite-ready evidence, mirrors, llms.txt.";
    case "entity_legitimacy":
      return "Does the entity exist strongly enough to be selected at all? Aexeo surfaces this layer; it does not fix it.";
  }
}

// Mirror of aexeo-core's SiteIntelligenceScore. Exact subset the
// dashboard widget consumes; less-used fields like elapsed_us live in
// the JSON the bridge emits but are not typed here.
// Mirror of aexeo-core's TruthStructuredSource enum (snake_case as
// serialized by serde). The bridge splices this onto SiteIntelligenceScore
// JSON so the dashboard widget can badge the truth score with the actual
// signal source — telling editors "you're seeing schema-only" vs "you're
// seeing manifest+schema" without needing a separate WASM call.
export type StructuredTruthSource =
  | "manifest"
  | "schema"
  | "schema_and_manifest"
  | "none";

export interface SiteIntelligenceScore {
  citation_readiness_score: number;
  truth_consistency_score: number;
  answer_pack_score: number;
  external_trust_alignment_score: number | null;
  overall_score: number;
  route_scores: unknown[];
  blockers: IntelligenceBlocker[];
  structured_truth_source?: StructuredTruthSource;
}

export interface IntelligenceBlocker {
  rule_id?: string;
  message: string;
  route?: string;
  severity?: string;
}

// Minimal projection of aexeo-core's TruthManifest — only the fields
// the plugin reads on the TS side. The Rust validator is the
// authority on full shape; here we just need what the
// entity-presence diagnostic and admin UI consume. Everything is
// optional because manifests can be partial during authoring.
export interface TruthEntity {
  name: string;
  aliases?: string[];
  website?: string;
  category?: string;
  descriptors?: string[];
  features?: string[];
}

export interface TruthManifest {
  version?: number;
  organization?: TruthEntity;
  products?: TruthEntity[];
  terminology?: {
    preferred?: Record<string, string>;
    forbidden?: Record<string, string>;
  };
}
