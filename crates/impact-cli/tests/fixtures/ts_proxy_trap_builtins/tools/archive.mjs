// Reads entries out of a binary archive; never touches bindAll.
export function readEntry(index, name) {
  const offset = index.get(name);
  return offset;
}
