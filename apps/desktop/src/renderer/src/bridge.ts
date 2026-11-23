// What the preload exposes (src/preload/index.ts), typed, and the command API client over it.
import { Client } from '@logica/api';

export interface Settings {
  theme: 'system' | 'light' | 'dark';
  mcp: boolean;
  expert: boolean;
  memoryMb: number;
  updates: boolean;
  recent: string[];
}

export interface McpStatus {
  port: number | null;
  error: string | null;
}

export interface LogicaBridge {
  call(command: string, input: unknown): Promise<unknown>;
  onEvent(listener: (event: unknown) => void): () => void;
  launchedAt(): number;
  ready(coldStartMs: number): void;
  failed(message: string): void;
  chooseFiles(): Promise<string[]>;
  chooseSave(suggested: string): Promise<string | null>;
  pathOf(file: File): string;
  opened(file: string): void;
  getSettings(): Promise<Settings>;
  setSettings(s: Settings): Promise<Settings>;
  openSettings(): void;
  onSettings(l: (s: Settings) => void): () => void;
  onMcp(l: (m: McpStatus) => void): () => void;
  onOpenFiles(l: (files: string[]) => void): () => void;
  onMenu(l: (id: string) => void): () => void;
  onStartedByAgent(l: () => void): () => void;
  onEnv(l: (e: { packaged: boolean }) => void): () => void;
  onUpdate(l: (u: { version: string }) => void): () => void;
  platform: string;
}

declare global {
  interface Window {
    logica: LogicaBridge;
  }
}

export const bridge = (): LogicaBridge => window.logica;

export const client = new Client({ call: (command, input) => window.logica.call(command, input) });

/** "Ctrl" or "⌘" for shortcuts shown to the person. */
export const mod = (): string => (window.logica?.platform === 'darwin' ? '⌘' : 'Ctrl');
