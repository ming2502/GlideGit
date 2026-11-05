// Fuzzy matching for the command palette: the query's characters must
// appear in order. Scores favour word starts, runs of consecutive
// characters and an early first match, like IntelliJ's "Find Action".

export interface Match {
  score: number;
  /** Indexes in the text of the matched characters. */
  positions: number[];
}

const isWordStart = (text: string, i: number) => {
  if (i === 0) return true;
  const prev = text[i - 1];
  const cur = text[i];
  if (/[\s\-_/.:（(「]/.test(prev)) return true;
  // camelCase hump, or the boundary between Latin and CJK text.
  if (/[a-z]/.test(prev) && /[A-Z]/.test(cur)) return true;
  return /[一-鿿]/.test(cur) !== /[一-鿿]/.test(prev) && /\S/.test(prev);
};

/**
 * Match `query` against `text`. Spaces in the query separate words that may
 * match anywhere, in order. Returns null when it does not match.
 */
export function fuzzyMatch(query: string, text: string): Match | null {
  const q = query.trim().toLowerCase().replace(/\s+/g, " ");
  if (!q) return { score: 0, positions: [] };
  const lower = text.toLowerCase();

  // A plain substring is the best kind of match.
  const sub = lower.indexOf(q);
  if (sub >= 0) {
    const positions = Array.from({ length: q.length }, (_, k) => sub + k);
    // In the middle of a word ("cap" in "Escape") it ranks below an acronym.
    const score = isWordStart(text, sub) ? 1000 - sub + (sub === 0 ? 300 : 0) : 400 - sub;
    return { score, positions };
  }

  // Greedy in-order match, preferring word starts: for each query character
  // look ahead for a word start before settling for the next occurrence.
  const positions: number[] = [];
  let score = 0;
  let from = 0;
  let prev = -2;
  for (const ch of q) {
    if (ch === " ") continue;
    let at = -1;
    for (let i = from; i < lower.length; i++) {
      if (lower[i] !== ch) continue;
      if (i === prev + 1) {
        at = i;
        break;
      }
      if (isWordStart(text, i)) {
        at = i;
        break;
      }
      if (at < 0) at = i;
    }
    if (at < 0) return null;
    // Like IntelliJ: a scattered match must begin at the start of a word
    // ("main" should not match the "m" inside "compare").
    if (positions.length === 0 && !isWordStart(text, at) && !/[\u4e00-\u9fff]/.test(text[at])) return null;
    if (at === prev + 1) score += 15;
    if (isWordStart(text, at)) score += 30;
    score -= Math.min(at - from, 20);
    positions.push(at);
    prev = at;
    from = at + 1;
  }
  // Every character on a word start: an acronym ("cap" → Commit And Push).
  const acronym = positions.length > 1 && positions.every((i) => isWordStart(text, i));
  return { score: score - positions[0] + (acronym ? 500 : 0), positions };
}

/** Best match of a query against several texts (title first, then aliases). */
export function bestMatch(query: string, title: string, aliases: string[] = []): { title: Match | null; score: number } | null {
  const t = fuzzyMatch(query, title);
  let best = t ? t.score : -Infinity;
  for (const a of aliases) {
    const m = fuzzyMatch(query, a);
    // Matching only an alias ranks a little lower than matching the title.
    if (m && m.score - 50 > best) best = m.score - 50;
  }
  return best === -Infinity ? null : { title: t, score: best };
}
