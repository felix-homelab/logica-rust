// The typed client of Logica's command API (doc 03). The types come from the contract; this file
// only binds them to a transport (the window's MessagePort, tests, …).
import type { Contract } from './contract.gen.js';

export * from './contract.gen.js';
export { API_VERSION, COMMANDS, type CommandSpec } from './commands.gen.js';

export type CommandName = keyof Contract['inputs'];
export type Input<C extends CommandName> = Contract['inputs'][C];
export type Output<C extends CommandName> = Contract['outputs'][C];
export type CommandEvent = Contract['event'];
export type CommandFailure = Contract['error'];

/** Carries one command to the engine and its result back; rejects with a CommandFailure. */
export interface Transport {
  call(command: string, input: unknown): Promise<unknown>;
}

/** An error from the engine, worded for the person: what is wrong and what to do. */
export class CommandError extends Error {
  readonly code: CommandFailure['code'];
  constructor(failure: CommandFailure) {
    super(failure.message);
    this.name = 'CommandError';
    this.code = failure.code;
  }
}

const isFailure = (x: unknown): x is CommandFailure =>
  typeof x === 'object' &&
  x !== null &&
  typeof (x as { code?: unknown }).code === 'string' &&
  typeof (x as { message?: unknown }).message === 'string';

export class Client {
  constructor(private readonly transport: Transport) {}

  /** Runs a command; its input and output types follow from its name. */
  async run<C extends CommandName>(command: C, input: Input<C>): Promise<Output<C>> {
    try {
      return (await this.transport.call(command, input)) as Output<C>;
    } catch (e) {
      throw isFailure(e) ? new CommandError(e) : e;
    }
  }
}
