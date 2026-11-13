export function deadline(ms) {
  const now = Date.now();
  return new Map([["at", now + ms]]).get("at");
}
