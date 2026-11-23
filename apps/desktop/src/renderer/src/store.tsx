// The window's view of the engine session (doc 03: shared session). The session is read again
// after every recorded change, undo, redo or lease change (state.changed) and on view and decoder
// events, so what agents do appears here live. Nothing is mirrored by hand.
import type { ReactNode } from 'react';
import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react';
import { CommandError, type CommandName, type Input, type Output } from '@logica/api';
import { bridge, client, type McpStatus, type Settings } from './bridge';

export type Session = Output<'session.get'>;
export type CaptureInfo = Session['captures'][number];
export type HistoryItem = Session['recent'][number];

export interface Notice {
  id: number;
  tone: 'danger' | 'agent' | 'neutral' | 'success';
  text: string;
}

interface Store {
  session: Session | null;
  /** The engine could not be reached. */
  failure: string | null;
  settings: Settings | null;
  mcp: McpStatus;
  startedByAgent: boolean;
  notices: Notice[];
  /** Runs a command; shows the engine's message when it fails and returns undefined then. */
  run<C extends CommandName>(command: C, input: Input<C>): Promise<Output<C> | undefined>;
  refresh(): void;
  notify(tone: Notice['tone'], text: string): void;
  dismiss(id: number): void;
  /** Bumped by every engine event, for views that cache engine data (waveform, tables). */
  revision: number;
}

const Ctx = createContext<Store | null>(null);

export function useStore(): Store {
  const s = useContext(Ctx);
  if (!s) throw new Error('useStore outside StoreProvider');
  return s;
}

/** The capture in front, if any. */
export function useActive(): CaptureInfo | undefined {
  const { session } = useStore();
  return session?.captures.find((c) => c.id === session.active) ?? session?.captures[0];
}

let noticeId = 1;

export function StoreProvider({ children }: { children: ReactNode }) {
  const [session, setSession] = useState<Session | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [mcp, setMcp] = useState<McpStatus>({ port: null, error: null });
  const [startedByAgent, setStartedByAgent] = useState(false);
  const [notices, setNotices] = useState<Notice[]>([]);
  const [revision, setRevision] = useState(0);
  const pending = useRef<number | null>(null);
  const first = useRef(true);

  const notify = useCallback((tone: Notice['tone'], text: string) => {
    const id = noticeId++;
    setNotices((n) => [...n.slice(-3), { id, tone, text }]);
    window.setTimeout(() => setNotices((n) => n.filter((x) => x.id !== id)), tone === 'danger' ? 8000 : 5000);
  }, []);

  const refresh = useCallback(() => {
    client
      .run('session.get', {})
      .then((s) => {
        setSession(s);
        setFailure(null);
        if (first.current) {
          first.current = false;
          bridge().ready(Date.now() - bridge().launchedAt());
        }
      })
      .catch((e: unknown) => {
        const message = e instanceof CommandError ? e.message : String(e);
        setFailure(message);
        if (first.current) bridge().failed(message);
      });
  }, []);

  // several events in a row cause one read
  const refreshSoon = useCallback(() => {
    if (pending.current !== null) return;
    pending.current = window.setTimeout(() => {
      pending.current = null;
      refresh();
    }, 30);
  }, [refresh]);

  const run = useCallback(
    async <C extends CommandName>(command: C, input: Input<C>): Promise<Output<C> | undefined> => {
      try {
        return await client.run(command, input);
      } catch (e) {
        notify('danger', e instanceof CommandError ? e.message : String(e));
        return undefined;
      }
    },
    [notify],
  );

  useEffect(() => {
    refresh();
    const offEvents = bridge().onEvent((raw) => {
      const e = raw as { event: string; actor?: string; capture?: string };
      setRevision((r) => r + 1);
      if (['state.changed', 'view.changed', 'decoder.updated', 'capture.done', 'lease.changed', 'device.disconnected'].includes(e.event)) {
        refreshSoon();
      }
      if (e.event === 'view.changed' && e.actor?.startsWith('agent:')) {
        notify('agent', `${e.actor.slice(6)} moved your view.`);
      }
    });
    const offSettings = bridge().onSettings((s) => {
      setSettings(s);
      const root = document.documentElement;
      if (s.theme === 'system') delete root.dataset.theme;
      else root.dataset.theme = s.theme;
    });
    const offMcp = bridge().onMcp(setMcp);
    const offAgent = bridge().onStartedByAgent(() => setStartedByAgent(true));
    return () => {
      offEvents();
      offSettings();
      offMcp();
      offAgent();
    };
  }, [refresh, refreshSoon, notify]);

  const dismiss = useCallback((id: number) => setNotices((n) => n.filter((x) => x.id !== id)), []);

  const value = useMemo<Store>(
    () => ({ session, failure, settings, mcp, startedByAgent, notices, run, refresh, notify, dismiss, revision }),
    [session, failure, settings, mcp, startedByAgent, notices, run, refresh, notify, dismiss, revision],
  );
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

/** "agent:claude-code" → "claude-code"; the person → "you". */
export function who(actor: string): string {
  if (actor === 'user') return 'you';
  if (actor === 'cli') return 'the command line';
  return actor.startsWith('agent:') ? actor.slice(6) : actor;
}

export const isAgent = (actor: string | null | undefined): boolean => !!actor && actor.startsWith('agent:');
