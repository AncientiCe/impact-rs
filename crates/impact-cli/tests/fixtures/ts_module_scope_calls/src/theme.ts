export function withTheme(name: string, getStyles: () => object) {
  return <C>(component: C): C => component;
}
