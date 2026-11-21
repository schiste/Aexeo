/** Explicit host attestations are scoped by collection and, for fragments, field.
 * Treat malformed external configuration as unavailable rather than as consent. */
export function collectionListStructureAttested(value: unknown, collection: string): boolean {
  return Array.isArray(value) && value.includes(collection);
}

export function fragmentListStructureAttested(value: unknown, collection: string, field: string): boolean {
  if (value === null || typeof value !== "object" || Array.isArray(value) || !Object.hasOwn(value, collection)) return false;
  const fields: unknown = (value as Record<string, unknown>)[collection];
  return Array.isArray(fields) && fields.includes(field);
}
