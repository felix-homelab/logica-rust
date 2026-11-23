// The command palette's matching: every word must match; prefixes and title starts rank first.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { filterItems } from '../src/components/filter.ts';

const item = (title, keywords) => ({ id: title, title, group: 'Commands', keywords, run: () => {} });
const items = [
  item('Open file…', ['import']),
  item('Add decoder'),
  item('Undo', ['history']),
  item('Toggle sidebar'),
  item('Decoded table: show'),
];
const titles = (q) => filterItems(items, q).map((i) => i.title);

test('an empty query keeps every item in its order', () => {
  assert.deepEqual(titles('  '), items.map((i) => i.title));
});

test('every word must match, title starts first', () => {
  assert.deepEqual(titles('dec'), ['Decoded table: show', 'Add decoder']);
  assert.deepEqual(titles('add dec'), ['Add decoder']);
  assert.deepEqual(titles('add zzz'), []);
});

test('keywords and mid-word matches count, case does not', () => {
  assert.deepEqual(titles('IMPORT'), ['Open file…']);
  assert.deepEqual(titles('history'), ['Undo']);
  assert.deepEqual(titles('bar'), ['Toggle sidebar']);
});
