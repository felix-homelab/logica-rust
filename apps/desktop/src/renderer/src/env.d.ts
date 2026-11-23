// What the preload exposes to the page (src/preload/index.ts).
interface LogicaBridge {
  call(command: string, input: unknown): Promise<unknown>;
  onEvent(listener: (event: unknown) => void): () => void;
  launchedAt(): number;
  ready(coldStartMs: number): void;
  failed(message: string): void;
}

interface Window {
  logica: LogicaBridge;
}
