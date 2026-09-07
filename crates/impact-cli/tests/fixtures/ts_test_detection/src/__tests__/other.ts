import { process } from "../service";

describe("directory suite", () => {
  beforeEach(() => {
    process();
  });

  it("works", () => {
    expect(process()).toBe(true);
  });
});
