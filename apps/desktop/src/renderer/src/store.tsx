// The window's view of the engine session (doc 03: shared session). The session is read again on
// the events that change it (relevance.ts), so what agents do appears here live; nothing is
// mirrored by hand. The store also follows the engine process (restarts, failures), shows what long
// commands are doing, and logs what goes wrong.
import type { ReactNode } from 'react';
import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react';
import { CommandError, type CommandName, type Input, type Output } from '@logica/api';
import { bridge, client, type EngineState, type McpStatus, type Settings } from './bridge';
import { sessionLiveNeeds, sessionNeeds, type EngineEvent } from './relevance';

export type Session = Output<'session.get'>;
export type CaptureInfo = Session['captures'][number];
export type HistoryItem = Session['recent'][number];

export interface Notice {
  id: number;
  tone: 'danger' | 'agent' | 'neutral' | 'success';
  text: string;
}

/** Files that were open when the engine stopped, to open again. */
export interface Lost {
  files: { path: string; title: string }[];
  /** Captures with no file, or with changes since: their work is gone. */
  unsaved: string[];
}

interface Store {
  session: Session | null;
  /** The engine could not be reached. */
  failure: string | null;
  engine: EngineState;
  lost: Lost | null;
  dismissLost(): void;
  settings: Settings | null;
  mcp: McpStatus;
  startedByAgent: boolean;
  notices: Notice[];
  /** What a long command is doing right now ("Opening mystery_D.demo…"), if anything. */
  busy: string | null;
  /** Runs a command; shows the engine's message when it fails and returns undefined then. */
  run<C extends CommandName>(command: C, input: Input<C>): Promise<Output<C> | undefined>;
  refresh(): void;
  notify(tone: Notice['tone'], text: string): void;
  dismiss(id: number): void;
  /** Subscribes to engine events; returns the unsubscribe function. */
  subscribe(listener: (e: EngineEvent) => void): () => void;
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

/** A number that grows with each event `match` accepts, a little delayed so bursts count once. */
export function useEventCount(match: (e: EngineEvent) => boolean, delayMs = 60): number {
  const { subscribe } = useStore();
  const [count, setCount] = useState(0);
  const matchRef = useRef(match);
  matchRef.current = match;
  useEffect(() => {
    let timer: number | null = null;
    const off = subscribe((e) => {
      if (!matchRef.current(e) || timer !== null) return;
      timer = window.setTimeout(() => {
        timer = null;
        setCount((n) => n + 1);
      }, delayMs);
    });
    return () => {
      off();
      if (timer !== null) window.clearTimeout(timer);
    };
  }, [subscribe, delayMs]);
  return count;
}

/** Long commands and what the status bar says while they run. */
function busyLabel(command: string, input: unknown): string | null {
  const i = (input ?? {}) as { path?: string };
  switch (command) {
    case 'file.open':
      return `Opening ${String(i.path ?? '').split(/[\\/]/).pop()}…`;
    case 'file.save':
      return 'Saving…';
    case 'capture.start':
      return 'Capturing…';
    case 'analysis.detect':
      return 'Looking at the signals…';
    case 'decoder.add':
    case 'decoder.update':
      return 'Decoding…';
    case 'search.run':
      return 'Searching…';
    default:
      return null;
  }
}

let noticeId = 1;

export function StoreProvider({ children }: { children: ReactNode }) {
  const [session, setSession] = useState<Session | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [engine, setEngine] = useState<EngineState>({ state: 'starting' });
  const [lost, setLost] = useState<Lost | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [mcp, setMcp] = useState<McpStatus>({ port: null, error: null });
  const [startedByAgent, setStartedByAgent] = useState(false);
  const [notices, setNotices] = useState<Notice[]>([]);
  const [busy, setBusy] = useState<string[]>([]);
  const listeners = useRef(new Set<(e: EngineEvent) => void>());
  const pending = useRef<number | null>(null);
  const livePending = useRef<number | null>(null);
  const first = useRef(true);
  const sessionRef = useRef<Session | null>(null);
  sessionRef.current = session;

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
        bridge().log('warn', `session.get failed: ${message}`);
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

  // a live capture's progress: at most four reads a second
  const refreshLive = useCallback(() => {
    if (livePending.current !== null) return;
    livePending.current = window.setTimeout(() => {
      livePending.current = null;
      refresh();
    }, 250);
  }, [refresh]);

  const run = useCallback(
    async <C extends CommandName>(command: C, input: Input<C>): Promise<Output<C> | undefined> => {
      const label = busyLabel(command, input);
      if (label) setBusy((b) => [...b, label]);
      try {
        return await client.run(command, input);
      } catch (e) {
        const message = e instanceof CommandError ? e.message : String(e);
        notify('danger', message);
        if (!(e instanceof CommandError)) bridge().log('error', `${command} failed: ${message}`);
        return undefined;
      } finally {
        if (label) {
          setBusy((b) => {
            const i = b.indexOf(label);
            return i < 0 ? b : [...b.slice(0, i), ...b.slice(i + 1)];
          });
        }
      }
    },
    [notify],
  );

  const subscribe = useCallback((listener: (e: EngineEvent) => void) => {
    listeners.current.add(listener);
    return () => {
      listeners.current.delete(listener);
    };
  }, []);

  useEffect(() => {
    // nothing that goes wrong in the window is silent
    const onError = (e: ErrorEvent) => bridge().log('error', `${e.message} (${e.filename}:${e.lineno})`);
    const onRejection = (e: PromiseRejectionEvent) =>
      bridge().log('error', `Unhandled: ${e.reason instanceof Error ? (e.reason.stack ?? e.reason.message) : String(e.reason)}`);
    window.addEventListener('error', onError);
    window.addEventListener('unhandledrejection', onRejection);
    // main asks before closing which captures would lose work
    bridge().provideUnsaved(() => (sessionRef.current?.captures ?? []).filter((c) => c.unsaved).map((c) => c.title));

    refresh();
    const offEvents = bridge().onEvent((raw) => {
      const e = raw as EngineEvent;
      listeners.current.forEach((l) => l(e));
      if (sessionNeeds(e)) refreshSoon();
      else if (sessionLiveNeeds(e)) refreshLive();
      if (e.event === 'view.changed' && e.actor?.startsWith('agent:')) {
        notify('agent', `${e.actor.slice(6)} moved your view.`);
      }
    });
    const offEngine = bridge().onEngine((s) => {
      setEngine((before) => {
        if (s.state === 'restarting' || s.state === 'failed') {
          // remember what was open, to offer it again once the engine is back
          const caps = sessionRef.current?.captures ?? [];
          if (caps.length && before.state === 'running') {
            setLost({
              files: caps.filter((c) => c.path).map((c) => ({ path: c.path!, title: c.title })),
              unsaved: caps.filter((c) => c.unsaved || !c.path).map((c) => c.title),
            });
          }
          bridge().log('error', `Engine: ${s.reason ?? s.state}`);
        }
        if (s.state === 'running' && before.state !== 'running' && before.state !== 'starting') refresh();
        return s;
      });
    });
    const offSettings = bridge().onSettings((s) => {
      setSettings(s);
      const root = document.documentElement;
      if (s.theme === 'system') delete root.dataset.theme;
      else root.dataset.theme = s.theme;
    });
    const offMcp = bridge().onMcp(setMcp);
    const offAgent = bridge().onStartedByAgent(() => setStartedByAgent(true));
    const offUpdate = bridge().onUpdate((u) => notify('success', `Logica ${u.version} is ready; it is installed when you quit.`));
    return () => {
      window.removeEventListener('error', onError);
      window.removeEventListener('unhandledrejection', onRejection);
      offEvents();
      offEngine();
      offSettings();
      offMcp();
      offAgent();
      offUpdate();
    };
  }, [refresh, refreshSoon, refreshLive, notify]);

  const dismiss = useCallback((id: number) => setNotices((n) => n.filter((x) => x.id !== id)), []);
  const dismissLost = useCallback(() => setLost(null), []);

  const value = useMemo<Store>(
    () => ({
      session,
      failure,
      engine,
      lost,
      dismissLost,
      settings,
      mcp,
      startedByAgent,
      notices,
      busy: busy[0] ?? null,
      run,
      refresh,
      notify,
      dismiss,
      subscribe,
    }),
    [session, failure, engine, lost, dismissLost, settings, mcp, startedByAgent, notices, busy, run, refresh, notify, dismiss, subscribe],
  );
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

/** "agent:claude-code" → "claude-code"; the person → "you". */
/** How often the window looks for devices while devices are on screen. */
export const DEVICE_WATCH_MS = 2000;

/**
 * While `on`, looks for devices every two seconds (a virtual analyser or a plugged-in device can
 * appear or go at any time; the engine only looks when asked) and reads the session again when what
 * was found changed. Quiet: no busy label, and a failure is left to the next look.
 */
export function useDeviceWatch(on: boolean): void {
  const { refresh } = useStore();
  useEffect(() => {
    if (!on) return;
    let last = '';
    let alive = true;
    const look = () =>
      client
        .run('device.list', {})
        .then((r) => {
          const now = JSON.stringify(r.devices.map((d) => [d.info.id, d.connected, d.capturing, d.holder]));
          if (alive && now !== last) {
            last = now;
            refresh();
          }
        })
        .catch(() => {});
    void look();
    const t = window.setInterval(() => void look(), DEVICE_WATCH_MS);
    return () => {
      alive = false;
      window.clearInterval(t);
    };
  }, [on, refresh]);
}

export function who(actor: string): string {
  if (actor === 'user') return 'you';
  if (actor === 'cli') return 'the command line';
  return actor.startsWith('agent:') ? actor.slice(6) : actor;
}

export const isAgent = (actor: string | null | undefined): boolean => !!actor && actor.startsWith('agent:');
