/**
 * Pictures in Delta, and the HTML a doc is kept as. A picture is shrunk to fit MAX_IMAGE_CHARS and stored as its own
 * record (model.ts), and text points at it by id. A doc's HTML is cleaned to the editor's own tags both when it's saved
 * and before it's shown, since a cloud workspace's docs are written by other people.
 */
import { MAX_IMAGE_CHARS, newId, type Doc } from './model';
import type { Session } from './sync';

const LONGEST = 2000;

function loadImage(src: string): Promise<HTMLImageElement> {
  return new Promise((ok, fail) => {
    const img = new Image();
    img.onload = () => ok(img);
    img.onerror = () => fail(new Error("That picture couldn't be opened."));
    img.src = src;
  });
}

function readAsDataUrl(file: Blob): Promise<string> {
  return new Promise((ok, fail) => {
    const r = new FileReader();
    r.onload = () => ok(String(r.result));
    r.onerror = () => fail(new Error("That file couldn't be read."));
    r.readAsDataURL(file);
  });
}

/** A file as a picture Delta can keep: GIFs as they are when they fit (so they still move), the rest redrawn smaller. */
export async function shrink(file: File): Promise<{ data: string; width: number; height: number }> {
  if (!file.type.startsWith('image/')) throw new Error(`${file.name || 'That file'} isn't a picture.`);
  const url = URL.createObjectURL(file);
  try {
    const img = await loadImage(url);
    const w0 = img.naturalWidth || 800;
    const h0 = img.naturalHeight || 600;
    if (file.type === 'image/gif' || (/^image\/(png|jpeg|webp)$/.test(file.type) && Math.max(w0, h0) <= LONGEST)) {
      const raw = await readAsDataUrl(file);
      if (raw.length <= MAX_IMAGE_CHARS && (file.type !== 'image/png' || raw.length < 400_000)) return { data: raw, width: w0, height: h0 };
    }
    let scale = Math.min(1, LONGEST / Math.max(w0, h0));
    for (let n = 0; n < 8; n++, scale *= 0.75) {
      const w = Math.max(1, Math.round(w0 * scale));
      const h = Math.max(1, Math.round(h0 * scale));
      const canvas = document.createElement('canvas');
      canvas.width = w;
      canvas.height = h;
      canvas.getContext('2d')!.drawImage(img, 0, 0, w, h);
      let data = canvas.toDataURL('image/webp', 0.86);
      // Browsers that can't write WebP hand back a PNG.
      if (!data.startsWith('data:image/webp')) data = canvas.toDataURL('image/jpeg', 0.86);
      if (data.length <= MAX_IMAGE_CHARS) return { data, width: w, height: h };
    }
    throw new Error('That picture is too big.');
  } finally {
    URL.revokeObjectURL(url);
  }
}

/** Stores pictures in the workspace; resolves to their new ids, in order. Files that aren't pictures are skipped. */
export async function addImages(s: Session, files: Iterable<File>): Promise<string[]> {
  const ids: string[] = [];
  for (const f of files) {
    if (!f.type.startsWith('image/')) continue;
    try {
      const { data, width, height } = await shrink(f);
      const id = newId();
      if (s.dispatch({ type: 'create', kind: 'image', id, data: { name: f.name || 'Pasted image', data, width, height } })) ids.push(id);
    } catch (e) {
      s.fail(e instanceof Error ? e.message : String(e));
    }
  }
  return ids;
}

type SetText = (f: (v: string) => string) => void;

/**
 * Pictures for a plain text box (an issue's description, a comment): pasted, dropped or picked, they're stored and a
 * ![name](img:id) line goes in where the caret is.
 */
export function imageHandlers(s: Session, box: { current: HTMLTextAreaElement | null }, set: SetText) {
  const add = async (files: File[]) => {
    const ids = await addImages(s, files);
    if (!ids.length) return;
    const text = ids.map((id) => imageToken(s.doc, id)).join('\n');
    set((v) => {
      const at = Math.min(box.current?.selectionEnd ?? v.length, v.length);
      const head = v.slice(0, at);
      return `${head}${head && !head.endsWith('\n') ? '\n' : ''}${text}\n${v.slice(at)}`;
    });
  };
  return {
    onPaste: (e: { clipboardData: DataTransfer | null; preventDefault(): void }) => {
      const f = imageFiles(e.clipboardData);
      if (f.length) {
        e.preventDefault();
        add(f);
      }
    },
    onDrop: (e: { dataTransfer: DataTransfer | null; preventDefault(): void }) => {
      const f = imageFiles(e.dataTransfer);
      if (f.length) {
        e.preventDefault();
        add(f);
      }
    },
    onDragOver: (e: { dataTransfer: DataTransfer | null; preventDefault(): void }) => {
      if (e.dataTransfer?.types.includes('Files')) e.preventDefault();
    },
    pick: async () => add(await pickImages()),
  };
}

/** Picture files from a paste or a drop, if there are any. */
export function imageFiles(dt: DataTransfer | null): File[] {
  if (!dt) return [];
  const out: File[] = [];
  for (const f of Array.from(dt.files ?? [])) if (f.type.startsWith('image/')) out.push(f);
  if (!out.length) for (const it of Array.from(dt.items ?? [])) if (it.kind === 'file' && it.type.startsWith('image/')) { const f = it.getAsFile(); if (f) out.push(f); }
  return out;
}

/** Opens the file picker for pictures. */
export function pickImages(multiple = true): Promise<File[]> {
  return new Promise((ok) => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = 'image/png,image/jpeg,image/webp,image/gif,image/svg+xml,image/bmp';
    input.multiple = multiple;
    input.onchange = () => ok(Array.from(input.files ?? []));
    input.click();
  });
}

export const imageSrc = (doc: Doc, id: string) => doc.image[id]?.data ?? '';

/** The text an issue or comment holds for a picture. */
export const imageToken = (doc: Doc, id: string) => `![${(doc.image[id]?.name ?? 'image').replace(/[[\]]/g, '')}](img:${id})`;

// ---------- a doc's HTML ----------

const KEEP = new Set(['P', 'H1', 'H2', 'H3', 'UL', 'OL', 'LI', 'BLOCKQUOTE', 'PRE', 'HR', 'B', 'I', 'U', 'S', 'CODE', 'A', 'BR', 'IMG', 'ASIDE']);
const RENAME: Record<string, string> = { STRONG: 'B', EM: 'I', STRIKE: 'S', DEL: 'S', H4: 'H3', H5: 'H3', H6: 'H3', DIV: 'P', FIGURE: 'P' };
const DROP = new Set(['SCRIPT', 'STYLE', 'IFRAME', 'OBJECT', 'EMBED', 'TEMPLATE', 'NOSCRIPT', 'SVG', 'MATH', 'META', 'LINK', 'TITLE', 'HEAD', 'BUTTON', 'INPUT', 'SELECT', 'TEXTAREA', 'FORM', 'VIDEO', 'AUDIO', 'CANVAS']);
const BLOCKS = new Set(['P', 'H1', 'H2', 'H3', 'UL', 'OL', 'BLOCKQUOTE', 'PRE', 'HR', 'ASIDE']);

function cleanInto(from: Node, to: Node, inPre: boolean) {
  for (const n of Array.from(from.childNodes)) {
    if (n.nodeType === 3) {
      to.appendChild(document.createTextNode(n.textContent ?? ''));
      continue;
    }
    if (n.nodeType !== 1) continue;
    const el = n as HTMLElement;
    let tag = el.tagName.toUpperCase();
    if (DROP.has(tag)) continue;
    tag = RENAME[tag] ?? tag;
    // A styled span from a paste: keep bold, italic and code where the style says so.
    if (!KEEP.has(tag) || (inPre && tag !== 'BR')) {
      cleanInto(el, to, inPre);
      continue;
    }
    // A div or figure that holds blocks is unwrapped rather than turned into a paragraph around them.
    if (tag === 'P' && Array.from(el.children).some((c) => BLOCKS.has(RENAME[c.tagName] ?? c.tagName) || c.tagName === 'DIV')) {
      cleanInto(el, to, inPre);
      continue;
    }
    const out = document.createElement(tag);
    if (tag === 'A') {
      const href = el.getAttribute('href') ?? '';
      // #page:<id> is a link to another doc in the workspace.
      if (!/^(https?:|mailto:)/i.test(href) && !/^#page:[a-z0-9]{6,40}$/i.test(href)) {
        cleanInto(el, to, inPre);
        continue;
      }
      out.setAttribute('href', href);
    }
    if (tag === 'IMG') {
      const id = el.getAttribute('data-img');
      const src = el.getAttribute('src') ?? '';
      if (id && /^[a-z0-9]{6,40}$/i.test(id)) out.setAttribute('data-img', id);
      else if (/^https:\/\//i.test(src)) out.setAttribute('src', src);
      else continue;
      const w = Number(el.getAttribute('data-w'));
      if (w > 0 && w <= 100) out.setAttribute('data-w', String(Math.round(w)));
      to.appendChild(out);
      continue;
    }
    if (tag === 'UL' && el.getAttribute('data-type') === 'todo') out.setAttribute('data-type', 'todo');
    if (tag === 'LI' && el.hasAttribute('data-checked')) out.setAttribute('data-checked', el.getAttribute('data-checked') === 'true' ? 'true' : 'false');
    if (tag === 'ASIDE' && el.getAttribute('data-icon')) out.setAttribute('data-icon', el.getAttribute('data-icon')!.slice(0, 8));
    if (tag === 'PRE') {
      out.textContent = preText(el);
      to.appendChild(out);
      continue;
    }
    cleanInto(el, out, inPre);
    to.appendChild(out);
  }
}

/** A code block's text, with <br>s and line divs as newlines. */
function preText(el: HTMLElement): string {
  let t = '';
  const walk = (n: Node) => {
    for (const c of Array.from(n.childNodes)) {
      if (c.nodeType === 3) t += c.textContent;
      else if ((c as HTMLElement).tagName === 'BR') t += '\n';
      else {
        if (/^(DIV|P)$/.test((c as HTMLElement).tagName) && t && !t.endsWith('\n')) t += '\n';
        walk(c);
      }
    }
  };
  walk(el);
  return t;
}

/** Keeps only the editor's own tags and attributes; loose text at the top goes into paragraphs. */
export function cleanHtml(html: string): string {
  const tpl = document.createElement('template');
  tpl.innerHTML = html;
  const out = document.createElement('div');
  cleanInto(tpl.content, out, false);
  // Inline runs at the top level go into paragraphs, so every line is a block the editor can turn into another.
  let run: HTMLElement | null = null;
  for (const n of Array.from(out.childNodes)) {
    const block = n.nodeType === 1 && (BLOCKS.has((n as HTMLElement).tagName) || (n as HTMLElement).tagName === 'IMG');
    if (block) {
      run = null;
      continue;
    }
    if (n.nodeType === 3 && !n.textContent?.trim() && !run) {
      n.remove();
      continue;
    }
    if (!run) {
      run = document.createElement('p');
      out.insertBefore(run, n);
    }
    run.appendChild(n);
  }
  return out.innerHTML;
}

/** Plain text as paragraphs, for a paste without HTML. */
export function textToHtml(text: string): string {
  const esc = (t: string) => t.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  return text.split(/\r?\n/).map((l) => `<p>${esc(l) || '<br>'}</p>`).join('');
}
