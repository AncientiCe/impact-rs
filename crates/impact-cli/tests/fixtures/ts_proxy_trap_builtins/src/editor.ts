import { apply, ownKeys } from "./patch";

export function save(doc: string): string {
  ownKeys({ doc });
  return apply(doc, "!");
}
