// Motion for a list of chat rows: the selection highlight glides to the chat that's open, rows slide to where they
// moved when the list changes (a new chat on top, one marked done), and a row that leaves fades out where it was
// instead of vanishing. Rows are found by `data-chat-id` among the list's own children.
import { useLayoutEffect, useRef, type RefObject } from 'react';

type Box = { top: number; left: number; width: number; height: number };

const reduced = () => typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
const EASE = 'cubic-bezier(0.2, 0.8, 0.2, 1)';

const boxOf = (el: HTMLElement): Box => ({ top: el.offsetTop, left: el.offsetLeft, width: el.offsetWidth, height: el.offsetHeight });

/** `frozen` while something else moves the rows itself (a drag): positions are only recorded, nothing animates. */
export function useListMotion(list: RefObject<HTMLElement | null>, frozen: boolean) {
  const highlight = useRef<HTMLDivElement>(null);
  const ghosts = useRef<HTMLDivElement>(null);
  const boxes = useRef(new Map<string, Box>());
  const nodes = useRef(new Map<string, HTMLElement>());
  const lit = useRef(false);

  // Every render: cheap, a read of each row's offset, and it has to see every change to the list.
  useLayoutEffect(() => {
    const root = list.current;
    if (!root) return;
    const now = new Map<string, HTMLElement>();
    root.querySelectorAll<HTMLElement>(':scope > [data-chat-id]').forEach((el) => now.set(el.dataset.chatId!, el));
    const animate = !frozen && !reduced();

    if (animate) {
      for (const [id, el] of nodes.current) {
        const box = boxes.current.get(id);
        if (now.has(id) || el.isConnected || !box || !ghosts.current) continue;
        const ghost = el.cloneNode(true) as HTMLElement;
        ghost.removeAttribute('data-chat-id');
        ghost.classList.add('row-ghost');
        Object.assign(ghost.style, { position: 'absolute', top: `${box.top}px`, left: `${box.left}px`, width: `${box.width}px`, height: `${box.height}px`, transform: '', transition: 'none', margin: '0' });
        ghosts.current.appendChild(ghost);
        ghost.animate(
          [{ opacity: 1, transform: 'none' }, { opacity: 0, transform: 'translateX(-14px) scale(0.97)' }],
          { duration: 240, easing: EASE, fill: 'forwards' },
        ).onfinish = () => ghost.remove();
      }
      for (const [id, el] of now) {
        const was = boxes.current.get(id);
        if (!was) continue;
        const dy = was.top - el.offsetTop;
        if (Math.abs(dy) > 0.5) el.animate([{ transform: `translateY(${dy}px)` }, { transform: 'none' }], { duration: 320, easing: EASE });
      }
    }
    boxes.current = new Map([...now].map(([id, el]) => [id, boxOf(el)]));
    nodes.current = now;

    // The highlight sits under the open chat's row; it fades in where it first lands, then glides.
    const hl = highlight.current;
    if (!hl) return;
    const selected = root.querySelector<HTMLElement>(':scope > .session-row.selected');
    if (!selected) {
      hl.style.opacity = '0';
      lit.current = false;
      return;
    }
    const b = boxOf(selected);
    const place = () => Object.assign(hl.style, { top: `${b.top}px`, left: `${b.left}px`, width: `${b.width}px`, height: `${b.height}px` });
    if (!lit.current || !animate) {
      hl.style.transition = 'none';
      place();
      void hl.offsetWidth;
      hl.style.transition = '';
      hl.style.opacity = '1';
      lit.current = true;
    } else {
      place();
    }
  });

  return { highlight, ghosts };
}
