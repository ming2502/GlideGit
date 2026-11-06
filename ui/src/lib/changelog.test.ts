import { describe, expect, it } from "vitest";
import { changelogBlocks, changelogSection, codeSpans } from "./changelog";

const SAMPLE = `# 更新日志

## [Unreleased]

- 还没发布

## [1.1.0] - 2026-12-01

新版本。

### 新增

- 一项功能，带 \`代码\`
  和换行的续行。

## [1.0.0] - 2026-11-06

- 第一版
`;

describe("changelog", () => {
  it("finds a version's section", () => {
    expect(changelogSection("1.0.0", SAMPLE)).toBe("- 第一版");
    expect(changelogSection("9.9.9", SAMPLE)).toBeNull();
    expect(changelogSection("1.0.0")).not.toBeNull(); // the bundled CHANGELOG.md
  });

  it("turns a section into blocks", () => {
    expect(changelogBlocks(changelogSection("1.1.0", SAMPLE)!)).toEqual([
      { kind: "text", text: "新版本。" },
      { kind: "heading", text: "新增" },
      { kind: "item", text: "一项功能，带 `代码` 和换行的续行。" },
    ]);
    expect(codeSpans("带 `代码` 的")).toEqual(["带 ", "代码", " 的"]);
  });
});
