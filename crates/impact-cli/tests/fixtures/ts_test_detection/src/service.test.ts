import { process } from "./service";

describe("service", () => {
  it("processes", () => {
    expect(process()).toBe(true);
  });
});
