// The palette's matching, kept free of JSX so it can be tested in plain Node.
import type { ReactNode } from 'react';

export interface PaletteItem {
  id: string;
  title: string;
  /** Shown as a heading over its items: "Commands", "Captures", … */
  group: string;
  hint?: string;
  shortcut?: string[];
  keywords?: string[];
  icon?: ReactNode;
  disabled?: boolean;
  /** Why it cannot run now (shown instead of running it). */
  disabledReason?: string;
  run: () => void;
}

/** Items whose words all match the query (by prefix of any word, or anywhere in the title), best first. */
export function filterItems(items: PaletteItem[], query: string): PaletteItem[] {
  const terms = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (terms.length === 0) return items;
  const scored: { item: PaletteItem; score: number }[] = [];
  for (const item of items) {
    const title = item.title.toLowerCase();
    const words = [...title.split(/[\s./:-]+/), ...(item.keywords ?? []).map((k) => k.toLowerCase())];
    let score = 0;
    let all = true;
    for (const t of terms) {
      if (title.startsWith(t)) score += 3;
      else if (words.some((w) => w.startsWith(t))) score += 2;
      else if (title.includes(t)) score += 1;
      else {
        all = false;
        break;
      }
    }
    if (all) scored.push({ item, score });
  }
  return scored.sort((a, b) => b.score - a.score).map((s) => s.item);
}

