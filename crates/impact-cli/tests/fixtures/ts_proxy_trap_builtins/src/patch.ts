// Project functions that share their names with JavaScript builtins.
export function apply(doc: string, edit: string): string {
  return doc + edit;
}

export function ownKeys(doc: object): string[] {
  return Object.keys(doc);
}
