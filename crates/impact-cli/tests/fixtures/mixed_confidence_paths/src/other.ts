// A second `ambiguous`, so a call to it through an untyped receiver can't tell the two
// apart.
export function ambiguous() {
  return 3;
}
