import { describe, expect, it } from "vitest";
import { changelogBlocks, changelogSection, codeSpans, inlineSpans } from "./changelog";

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

  it("parses inline code, bold and links", () => {
    expect(inlineSpans("用 `git **x**` 和 **加粗**，见 [README](https://x.y/z#a)。")).toEqual([
      { kind: "text", text: "用 " },
      { kind: "code", text: "git **x**" },
      { kind: "text", text: " 和 " },
      { kind: "strong", text: "加粗" },
      { kind: "text", text: "，见 README。" },
    ]);
    expect(inlineSpans("plain")).toEqual([{ kind: "text", text: "plain" }]);
  });
});
