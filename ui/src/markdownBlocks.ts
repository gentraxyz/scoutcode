/** A line that could carry on what's above a blank line (a list item, indented text, a table row), so no split before it. */
const CARRIES_ON = /^(\s|[-*+]\s|\d+[.)]\s|\|)/;
/** Link references and footnotes are defined anywhere and used anywhere, so text with them is parsed whole. */
const DEFINITIONS = /^ {0,3}\[[^\]]+\]:|\[\^/m;

/** Text cut into blocks that parse the same apart as together: at blank lines outside code fences, before a line that
 *  starts something new. A streaming reply then re-parses only its last block as words arrive, not the whole reply. */
export function markdownBlocks(text: string): string[] {
  if (DEFINITIONS.test(text)) return [text];
  const lines = text.split('\n');
  const out: string[] = [];
  let start = 0;
  let fence: string | null = null;
  for (let i = 0; i < lines.length; i++) {
    const f = /^ {0,3}(`{3,}|~{3,})(.*)$/.exec(lines[i]);
    if (f) {
      if (!fence) fence = f[1];
      else if (f[1][0] === fence[0] && f[1].length >= fence.length && !f[2].trim()) fence = null;
      continue;
    }
    const next = lines[i + 1];
    if (!fence && i > start && !lines[i].trim() && next?.trim() && !CARRIES_ON.test(next)) {
      out.push(lines.slice(start, i).join('\n'));
      start = i + 1;
    }
  }
  out.push(lines.slice(start).join('\n'));
  return out;
}
