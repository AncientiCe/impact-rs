import { process } from "./service";

// A top-level `it` with no enclosing `describe` — also a shape real suites use.
it("processes in a spec file", () => {
  expect(process()).toBe(true);
});
