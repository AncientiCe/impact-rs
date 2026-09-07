// Calls a `prune` it never imported — a global, or something a bundler injects. The
// adapter can't tie it to a module, so it stays reported but never as `Exact`.
export function airPrint(document) {
  return prune(document);
}
