import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Client, CommandError, COMMANDS, API_VERSION } from '../dist/index.js';

test('the command list carries every command of the contract with its rules', () => {
  const names = COMMANDS.map((c) => c.name);
  assert.equal(new Set(names).size, names.length);
  assert.ok(names.includes('file.open') && names.includes('history.undo'));
  assert.equal(API_VERSION, '1.0');
  const start = COMMANDS.find((c) => c.name === 'capture.start');
  assert.equal(start.lease, true);
  assert.equal(start.safety, 'capture');
  assert.equal(start.longRunning, true);
  assert.equal(COMMANDS.find((c) => c.name === 'file.save').undo.kind, 'never');
});

test('the client passes commands to its transport and returns the result', async () => {
  const calls = [];
  const client = new Client({
    call: async (command, input) => {
      calls.push([command, input]);
      return { entries: [] };
    },
  });
  assert.deepEqual(await client.run('history.list', { limit: 5 }), { entries: [] });
  assert.deepEqual(calls, [['history.list', { limit: 5 }]]);
});

test('engine errors become a CommandError with the code', async () => {
  const client = new Client({
    call: async () => {
      throw { code: 'busy', message: 'The device is in use by the person.' };
    },
  });
  await assert.rejects(
    client.run('session.get', {}),
    (e) => e instanceof CommandError && e.code === 'busy' && /in use/.test(e.message),
  );
});

test('other failures (a broken transport) pass through unchanged', async () => {
  const broken = new Error('port closed');
  const client = new Client({
    call: async () => {
      throw broken;
    },
  });
  await assert.rejects(client.run('session.get', {}), (e) => e === broken);
});
