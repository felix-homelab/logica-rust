// The command palette (doc 08: command palette first). Ctrl+K / Cmd+K opens it; it searches every
// item it is given (commands, captures, channels, decoders, hits) and runs the chosen one.
import type { KeyboardEvent } from 'react';
import { useEffect, useId, useMemo, useRef, useState } from 'react';
import { Dialog as RDialog } from 'radix-ui';
import { Kbd } from './basic';
import { filterItems, type PaletteItem } from './filter';

export type { PaletteItem };

export function CommandPalette({
  open,
  onOpenChange,
  items,
  placeholder = 'Search commands, captures, channels…',
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  items: PaletteItem[];
  placeholder?: string;
}) {
  const [query, setQuery] = useState('');
  const [index, setIndex] = useState(0);
  const listId = useId();
  const list = useRef<HTMLUListElement>(null);
  const shown = useMemo(() => filterItems(items, query).slice(0, 200), [items, query]);
  useEffect(() => {
    if (open) {
      setQuery('');
      setIndex(0);
    }
  }, [open]);
  useEffect(() => setIndex(0), [query]);
  useEffect(() => {
    list.current?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest' });
  }, [index]);
  const choose = (item: PaletteItem | undefined) => {
    if (!item || item.disabled) return;
    onOpenChange(false);
    item.run();
  };
  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setIndex((i) => Math.min(shown.length - 1, i + 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setIndex((i) => Math.max(0, i - 1));
    } else if (e.key === 'Enter') {
      e.preventDefault();
      choose(shown[index]);
    }
  };
  let lastGroup = '';
  return (
    <RDialog.Root open={open} onOpenChange={onOpenChange}>
      <RDialog.Portal>
        <RDialog.Overlay className="lg-dialog__scrim" />
        <RDialog.Content className="lg-palette" aria-describedby={undefined}>
          <RDialog.Title className="lg-visually-hidden">Command palette</RDialog.Title>
          <input
            className="lg-palette__input"
            role="combobox"
            aria-expanded="true"
            aria-controls={listId}
            aria-activedescendant={shown[index] ? `${listId}-${shown[index].id}` : undefined}
            aria-label="Search commands"
            placeholder={placeholder}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={onKeyDown}
            autoFocus
          />
          <ul id={listId} ref={list} role="listbox" className="lg-palette__list" aria-label="Results">
            {shown.length === 0 && <li className="lg-palette__empty">Nothing matches “{query}”.</li>}
            {shown.map((item, i) => {
              const heading = item.group !== lastGroup ? item.group : null;
              lastGroup = item.group;
              return (
                <li key={item.id} role="presentation">
                  {heading && <div className="lg-palette__group" aria-hidden="true">{heading}</div>}
                  <div
                    id={`${listId}-${item.id}`}
                    role="option"
                    aria-selected={i === index}
                    aria-disabled={item.disabled || undefined}
                    className="lg-palette__item"
                    onMouseMove={() => setIndex(i)}
                    onClick={() => choose(item)}
                  >
                    <span className="lg-palette__icon" aria-hidden="true">
                      {item.icon}
                    </span>
                    <span className="lg-palette__title">{item.title}</span>
                    <span className="lg-palette__hint">{item.disabled ? item.disabledReason : item.hint}</span>
                    {item.shortcut && <Kbd keys={item.shortcut} />}
                  </div>
                </li>
              );
            })}
          </ul>
        </RDialog.Content>
      </RDialog.Portal>
    </RDialog.Root>
  );
}
