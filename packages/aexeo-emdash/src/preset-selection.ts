/** Explicit, exact matching: combined override, locale, collection, fallback. */
export interface PresetSelection {
  name?: string;
  byCollection?: Readonly<Record<string, string>>;
  byLocale?: Readonly<Record<string, string>>;
  byCollectionLocale?: Readonly<Record<string, Readonly<Record<string, string>>>>;
}

function ownValue<T>(map: Readonly<Record<string, T>> | undefined, key: string | undefined): T | undefined {
  return map !== undefined && key !== undefined && Object.hasOwn(map, key) ? map[key] : undefined;
}

export type PresetSelectionSource = "collection_locale" | "locale" | "collection" | "fallback" | "none";
export type PresetSelectionCounts = Record<PresetSelectionSource, number>;

export function emptyPresetSelectionCounts(): PresetSelectionCounts {
  return { collection_locale: 0, locale: 0, collection: 0, fallback: 0, none: 0 };
}

export function selectContentPreset(selection: PresetSelection, collection: string | undefined, locale: string | undefined): { name: string | undefined; source: PresetSelectionSource } {
  const collectionLocales = ownValue(selection.byCollectionLocale, collection);
  const combined = ownValue(collectionLocales, locale);
  if (combined !== undefined) return { name: combined, source: "collection_locale" };
  const localized = ownValue(selection.byLocale, locale);
  if (localized !== undefined) return { name: localized, source: "locale" };
  const collected = ownValue(selection.byCollection, collection);
  if (collected !== undefined) return { name: collected, source: "collection" };
  return selection.name === undefined ? { name: undefined, source: "none" } : { name: selection.name, source: "fallback" };
}
