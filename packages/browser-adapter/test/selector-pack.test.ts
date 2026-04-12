import path from "node:path";
import {
  loadSelectorPack,
  normalizeSelectorPack,
  parseSelectorPack,
} from "../src/selectorPack";

const selectorPackPath = path.resolve(
  __dirname,
  "../../../selector-packs/v1/chatgpt-default.json",
);

describe("selector pack parsing", () => {
  it("parses the default selector pack", () => {
    const pack = loadSelectorPack(selectorPackPath);
    expect(normalizeSelectorPack(pack)).toMatchSnapshot();
  });

  it("rejects a pack without capabilities", () => {
    expect(() =>
      parseSelectorPack({
        format_version: 1,
        pack_id: "broken",
        description: "broken",
        updated_at: "2026-04-12T00:00:00Z",
        capabilities: {},
      }),
    ).toThrow("selector pack capability composer_ready must define strategies");
  });
});
