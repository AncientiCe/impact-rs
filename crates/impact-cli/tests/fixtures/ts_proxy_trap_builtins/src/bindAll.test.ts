import { bindAll } from "./bindAll";

describe("bindAll", () => {
  it("keeps methods bound", () => {
    const bound = bindAll({ name: "x" });
    expect(bound.name).toBe("x");
  });
});
