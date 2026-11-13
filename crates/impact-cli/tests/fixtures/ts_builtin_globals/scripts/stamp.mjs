// Never imports Clock; only shares its use of the Date builtin.
export const startedAt = Date.now();

export function elapsed() {
  const now = Date.now();
  return now - startedAt;
}
