// Compile-time checks of the generated types: wrong inputs and wrong use of outputs must not
// compile. Type-checked by `npm run lint` and `npm test` (tsconfig.test.json); never run.
import { Client, type Output } from '../src/index.js';

declare const client: Client;

export async function uses(): Promise<void> {
  const added = await client.run('decoder.add', {
    capture: 'c1',
    protocol: 'i2c',
    roles: { scl: 5, sda: 4 },
    settings: { speed: 400000 },
  });
  const id: string = added.decoder.id;
  void id;

  const session: Output<'session.get'> = await client.run('session.get', {});
  const version: string = session.apiVersion;
  void version;

  // @ts-expect-error: a command that does not exist
  await client.run('decoder.explode', {});
  // @ts-expect-error: decoder.add needs a protocol
  await client.run('decoder.add', { capture: 'c1', roles: {} });
  // @ts-expect-error: unknown fields are refused, as by the engine
  await client.run('session.get', { verbose: true });
  // @ts-expect-error: a channel is a number
  await client.run('decoder.add', { capture: 'c1', protocol: 'uart', roles: { rx: 'CH1' } });
}
