/** Display only: never use transformed text as an API path or identifier. */
export function visibleText(text: string): string {
  return text.replace(/[\p{Cc}\p{Cf}\p{Zl}\p{Zp}\p{Default_Ignorable_Code_Point}]/gu, (char) =>
    char === "\n" || char === "\t"
      ? char
      : `⟦U+${(char.codePointAt(0) ?? 0).toString(16).toUpperCase().padStart(4, "0")}⟧`,
  );
}
