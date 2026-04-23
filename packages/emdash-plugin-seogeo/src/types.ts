// These types mirror the Rust serde wire format emitted by the
// aexeo-emdash-bridge crate. Field names follow what serde actually
// serializes, not what the Rust struct fields look like: _key, listItem,
// markDefs are the JSON shape, rule_id and snake_case scope values come
// from seogeo-contracts. Keep them in sync when the bridge's types move.

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

export interface Finding {
  rule_id: string;
  message: string;
  path: string;
  line: number;
  column: number;
  severity: FindingSeverity;
  suggestion: string | null;
  scope: FindingScope;
}
