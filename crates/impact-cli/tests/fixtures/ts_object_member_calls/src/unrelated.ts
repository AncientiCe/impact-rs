const cache = new Map<string, string>();

export function lookup(key: string) {
  return cache.get(key);
}
