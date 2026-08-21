/** Prototype-safe string-keyed records for data derived from untrusted JSON. */
export function createSafeRecord<T>(): Record<string, T> {
  return Object.create(null) as Record<string, T>;
}

export function ownValue<T>(
  source: Record<string, T> | undefined,
  key: string
): T | undefined {
  return source && Object.prototype.hasOwnProperty.call(source, key)
    ? source[key]
    : undefined;
}
