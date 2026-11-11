import { bindAll } from "./bindAll";

export function renderPanel(state: { title(): string }): string {
  const bound = bindAll(state);
  return bound.title();
}
