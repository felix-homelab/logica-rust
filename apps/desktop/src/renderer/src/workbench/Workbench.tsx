// The main window (doc 08 layout): tabs, toolbar, channel sidebar, waveform, inspector, bottom
// panel and status bar; a start screen while nothing is open; the command palette.
import type { DragEvent } from 'react';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { COMMANDS } from '@logica/api';
import {
  Badge,
  Button,
  CommandPalette,
  Dialog,
  IconButton,
  Kbd,
  Splitter,
  TabsContent,
  TabsList,
  TabsRoot,
  TabsTrigger,
  useStored,
  type PaletteItem,
} from '@logica/ui';
import {
  Activity,
  Bot,
  Command,
  FlaskConical,
  FolderOpen,
  PanelBottom,
  PanelLeft,
  PanelRight,
  Plus,
  Redo2,
  Save,
  Settings as SettingsIcon,
  Undo2,
  X,
  ZoomIn,
  ZoomOut,
} from 'lucide-react';
import { bridge, client, mod } from '../bridge';
import { at, hz, int, time } from '../format';
import { isAgent, useActive, useStore, who, type CaptureInfo } from '../store';
import { centreOn, zoomAt } from '../viewmath';
import { ActivityFeed, DecodedTable, Measurements } from './bottom';
import { AnalysisPanel, ChannelList, DecodersPanel, DevicePanel, SearchPanel } from './panels';
import { Waveform, type ViewState } from './Waveform';

const AGENT_SETUP = 'claude mcp add logica -- logica mcp';

/** The engine's view of a capture as the window's view state. */
function fromEngine(c: CaptureInfo): ViewState {
  const v = c.view;
  const whole = v.to <= v.from;
  return {
    from: whole ? 0 : v.from,
    to: whole ? c.sampleCount : v.to,
    cursorA: v.cursorA ?? null,
    cursorB: v.cursorB ?? null,
    selection: (v.selection as [number, number] | null | undefined) ?? null,
  };
}

const key = (v: ViewState) => JSON.stringify([v.from, v.to, v.cursorA, v.cursorB, v.selection]);

export function Workbench() {
  const store = useStore();
  const { session, settings, run, notify } = store;
  const active = useActive();
  const [views, setViews] = useState<Record<string, ViewState>>({});
  const sent = useRef<Record<string, string>>({});
  const timers = useRef<Record<string, number>>({});
  const [palette, setPalette] = useState(false);
  const [help, setHelp] = useState<'keys' | 'agent' | null>(null);
  const [runCommand, setRunCommand] = useState<string | null>(null);
  const [inspector, setInspector] = useState('analysis');
  const [bottomTab, setBottomTab] = useState('decoded');
  const [showSidebar, setShowSidebar] = useStored('logica.sidebar', true);
  const [showInspector, setShowInspector] = useStored('logica.inspector', true);
  const [showBottom, setShowBottom] = useStored('logica.bottom', true);
  const [sidebarW, setSidebarW] = useStored('logica.sidebarW', 240);
  const [inspectorW, setInspectorW] = useStored('logica.inspectorW', 340);
  const [bottomH, setBottomH] = useStored('logica.bottomH', 240);
  const [narrow, setNarrow] = useState(window.innerWidth < 1280);
  const expert = settings?.expert ?? false;

  useEffect(() => {
    const f = () => setNarrow(window.innerWidth < 1280);
    window.addEventListener('resize', f);
    return () => window.removeEventListener('resize', f);
  }, []);

  // adopt the engine's view when someone else (an agent) changed it; keep our own otherwise
  useEffect(() => {
    if (!session) return;
    setViews((old) => {
      const next: Record<string, ViewState> = {};
      for (const c of session.captures) {
        const engine = fromEngine(c);
        const mine = old[c.id];
        next[c.id] = !mine || (sent.current[c.id] !== key(engine) && key(mine) !== key(engine)) ? engine : mine;
      }
      return next;
    });
  }, [session]);

  /** Shows a view at once and tells the engine shortly after (agents see the person's view). */
  const setView = useCallback(
    (capture: string, v: ViewState) => {
      setViews((old) => ({ ...old, [capture]: v }));
      window.clearTimeout(timers.current[capture]);
      timers.current[capture] = window.setTimeout(() => {
        sent.current[capture] = key(v);
        void client
          .run('view.set', {
            capture,
            from: v.from,
            to: v.to,
            ...(v.cursorA === null && v.cursorB === null ? { clearCursors: true } : {}),
            ...(v.cursorA !== null ? { cursorA: v.cursorA } : {}),
            ...(v.cursorB !== null ? { cursorB: v.cursorB } : {}),
            ...(v.selection ? { selection: v.selection } : { clearSelection: true }),
          })
          .catch(() => {});
      }, 200);
    },
    [],
  );

  const view = active ? views[active.id] : undefined;

  const jump = useCallback(
    (from: number, to: number) => {
      if (!active) return;
      const cur = views[active.id] ?? fromEngine(active);
      // the item fills about a fifth of the view when it is out of view or tiny in it
      const len = Math.max(1, to - from);
      const desired = Math.max(64, len * 5);
      const span = cur.to - cur.from;
      const inView = from >= cur.from && to <= cur.to;
      const next = span < len || span > desired * 50 ? desired : span;
      const r = inView && next === span ? { from: cur.from, to: cur.to } : centreOn((from + to) / 2, next, active.sampleCount);
      setView(active.id, { ...cur, ...r, selection: to > from ? [from, to] : null });
    },
    [active, views, setView],
  );

  const openFiles = useCallback(
    async (files: string[]) => {
      for (const path of files) {
        const r = await run('file.open', { path });
        if (r) {
          bridge().opened(path);
          void client.run('view.set', { capture: r.capture.id, activate: true }).catch(() => {});
        }
      }
    },
    [run],
  );

  const openDialog = useCallback(async () => openFiles(await bridge().chooseFiles()), [openFiles]);

  const save = useCallback(
    async (as: boolean) => {
      if (!active) return;
      const isLacap = active.path?.toLowerCase().endsWith('.lacap');
      let path: string | null | undefined;
      if (as || !isLacap) {
        path = await bridge().chooseSave(`${active.title.replace(/\.[^.]+$/, '')}.lacap`);
        if (!path) return;
      }
      const r = await run('file.save', { capture: active.id, ...(path ? { path } : {}) });
      if (r) notify('success', `Saved to ${r.path}.`);
    },
    [active, run, notify],
  );

  const demo = useCallback(async () => {
    const device = 'synthetic/mix';
    const dev = session?.devices.find((d) => d.info.id === device);
    if (!dev?.connected && !(await run('device.connect', { device }))) return;
    if (!(await run('device.configure', { device, config: { channels: [0, 1, 2, 6], sampleRateHz: 1e8, samples: 3_000_000, mode: 'stream' } }))) return;
    const r = await run('capture.start', { device, title: 'Demo capture', wait: true });
    if (r) void client.run('view.set', { capture: r.id, activate: true }).catch(() => {});
  }, [run, session]);

  const zoom = useCallback(
    (factor: number | 'all') => {
      if (!active || !view) return;
      setView(active.id, factor === 'all' ? { ...view, from: 0, to: active.sampleCount } : { ...view, ...zoomAt(view, factor, 0.5, active.sampleCount) });
    },
    [active, view, setView],
  );

  const actions = useMemo<Record<string, () => void>>(
    () => ({
      'file.open': () => void openDialog(),
      'file.save': () => void save(false),
      'file.saveAs': () => void save(true),
      'capture.close': () => active && void run('capture.close', { capture: active.id }),
      'history.undo': () => void run('history.undo', {}),
      'history.redo': () => void run('history.redo', {}),
      palette: () => setPalette(true),
      search: () => {
        setShowInspector(true);
        setInspector('search');
      },
      'view.sidebar': () => setShowSidebar(!showSidebar),
      'view.inspector': () => setShowInspector(!showInspector),
      'view.bottom': () => setShowBottom(!showBottom),
      'zoom.in': () => zoom(0.5),
      'zoom.out': () => zoom(2),
      'zoom.all': () => zoom('all'),
      'help.keys': () => setHelp('keys'),
      'help.agent': () => setHelp('agent'),
      gallery: () => {
        window.location.hash = 'gallery';
      },
      demo: () => void demo(),
      settings: () => bridge().openSettings(),
    }),
    [openDialog, save, active, run, showSidebar, showInspector, showBottom, setShowSidebar, setShowInspector, setShowBottom, zoom, demo],
  );

  useEffect(() => bridge().onMenu((id) => actions[id]?.()), [actions]);
  useEffect(() => bridge().onOpenFiles((files) => void openFiles(files)), [openFiles]);

  // every shortcut is handled here (doc 08: everything has a keyboard shortcut); the menu only
  // shows them, so they work the same with and without a menu bar, and not while typing
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const typing = (e.target as HTMLElement | null)?.closest('input, textarea, select, [role="combobox"]');
      const m = e.ctrlKey || e.metaKey;
      const k = e.key.toLowerCase();
      const map: Record<string, string> = {
        k: 'palette',
        o: 'file.open',
        s: e.shiftKey ? 'file.saveAs' : 'file.save',
        w: 'capture.close',
        z: e.shiftKey ? 'history.redo' : 'history.undo',
        y: 'history.redo',
        f: 'search',
        b: 'view.sidebar',
        i: 'view.inspector',
        j: 'view.bottom',
        '=': 'zoom.in',
        '+': 'zoom.in',
        '-': 'zoom.out',
        '0': 'zoom.all',
        ',': 'settings',
      };
      const id = m ? map[k] : !typing && (e.key === '?' || e.key === 'F1') ? 'help.keys' : e.key === 'F1' ? 'help.keys' : undefined;
      if (!id || (typing && id !== 'palette' && id !== 'help.keys')) return;
      e.preventDefault();
      actions[id]?.();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [actions]);

  const onDrop = (e: DragEvent) => {
    e.preventDefault();
    const files = [...e.dataTransfer.files].map((f) => bridge().pathOf(f)).filter(Boolean);
    void openFiles(files);
  };

  const items = useMemo<PaletteItem[]>(() => {
    const k = mod();
    const app: PaletteItem[] = [
      { id: 'a-open', group: 'Actions', title: 'Open file…', shortcut: [k, 'O'], icon: <FolderOpen />, run: actions['file.open']! },
      { id: 'a-demo', group: 'Actions', title: 'Try a demo capture', hint: 'synthetic device', icon: <FlaskConical />, run: actions.demo! },
      { id: 'a-save', group: 'Actions', title: 'Save', shortcut: [k, 'S'], icon: <Save />, disabled: !active, disabledReason: 'No capture is open', run: actions['file.save']! },
      { id: 'a-undo', group: 'Actions', title: 'Undo', shortcut: [k, 'Z'], icon: <Undo2 />, run: actions['history.undo']! },
      { id: 'a-redo', group: 'Actions', title: 'Redo', icon: <Redo2 />, run: actions['history.redo']! },
      { id: 'a-sidebar', group: 'Actions', title: 'Show or hide the sidebar', shortcut: [k, 'B'], icon: <PanelLeft />, run: actions['view.sidebar']! },
      { id: 'a-inspector', group: 'Actions', title: 'Show or hide the inspector', shortcut: [k, 'I'], icon: <PanelRight />, run: actions['view.inspector']! },
      { id: 'a-bottom', group: 'Actions', title: 'Show or hide the bottom panel', shortcut: [k, 'J'], icon: <PanelBottom />, run: actions['view.bottom']! },
      { id: 'a-settings', group: 'Actions', title: 'Settings', shortcut: [k, ','], icon: <SettingsIcon />, run: actions.settings! },
      { id: 'a-agent', group: 'Actions', title: 'Connect an agent', icon: <Bot />, run: actions['help.agent']! },
      {
        id: 'a-pause',
        group: 'Actions',
        title: session?.agentsPaused ? 'Let agents act again' : 'Stop agent actions',
        icon: <Activity />,
        run: () => void run('session.pauseAgents', { paused: !session?.agentsPaused }),
      },
    ];
    const captures: PaletteItem[] = (session?.captures ?? []).map((c) => ({
      id: `c-${c.id}`,
      group: 'Captures',
      title: c.title,
      hint: `${int(c.sampleCount)} samples at ${hz(c.sampleRateHz)}`,
      run: () => void client.run('view.set', { capture: c.id, activate: true }).catch(() => {}),
    }));
    const marks: PaletteItem[] = (active?.markers ?? []).map((m) => ({
      id: `m-${m.id}`,
      group: 'Markers',
      title: m.label || m.id,
      hint: `${at(m.at, active?.sampleRateHz ?? 0)} · ${who(m.by)}`,
      run: () => jump(m.at, m.end ?? m.at),
    }));
    const decoders: PaletteItem[] = (active?.decoders ?? []).map((d) => ({
      id: `d-${d.id}`,
      group: 'Decoders',
      title: d.config.name,
      hint: `${int(d.packets)} packets`,
      run: () => {
        setShowInspector(true);
        setInspector('decoders');
      },
    }));
    const commands: PaletteItem[] = COMMANDS.map((c) => ({
      id: `x-${c.name}`,
      group: 'Commands of the engine',
      title: c.name,
      hint: c.summary,
      keywords: c.summary.split(/\W+/),
      run: () => setRunCommand(c.name),
    }));
    return [...app, ...captures, ...marks, ...decoders, ...commands];
  }, [actions, active, session, run, jump, setShowInspector]);

  if (!session) {
    return (
      <main className="boot" aria-busy="true">
        <p>{store.failure ? `The window could not reach the engine: ${store.failure}` : 'Starting the engine…'}</p>
      </main>
    );
  }

  return (
    <div className="app" onDragOver={(e) => e.preventDefault()} onDrop={onDrop}>
      <header className="head">
        <h1 className="lg-visually-hidden">{active ? `Logica: ${active.title}` : 'Logica'}</h1>
        <div className="tabs">
          <nav className="tabs__list" aria-label="Open captures">
            <ul className="tabs__ul">
              {session.captures.map((c) => (
                <li key={c.id} className="tabs__tab" data-active={c.id === active?.id || undefined}>
                  <button
                    type="button"
                    aria-current={c.id === active?.id ? 'page' : undefined}
                    className="tabs__title"
                    onClick={() => void client.run('view.set', { capture: c.id, activate: true }).catch(() => {})}
                  >
                    {c.title}
                    {c.state === 'capturing' && <Badge tone="accent">capturing</Badge>}
                  </button>
                  <IconButton label={`Close ${c.title}`} icon={<X />} onClick={() => void run('capture.close', { capture: c.id })} />
                </li>
              ))}
            </ul>
            <IconButton label="Open a capture" shortcut={[mod(), 'O']} icon={<Plus />} onClick={actions['file.open']} />
          </nav>
          <button type="button" className="tabs__palette" onClick={() => setPalette(true)}>
            <Command aria-hidden="true" /> Search commands… <Kbd keys={[mod(), 'K']} />
          </button>
          <AgentIndicator />
        </div>
        {active && view && (
          <div className="toolbar" role="toolbar" aria-label="Capture">
            <IconButton label="Sidebar" shortcut={[mod(), 'B']} pressed={showSidebar} icon={<PanelLeft />} onClick={actions['view.sidebar']} />
            <span className="toolbar__sep" />
            <IconButton label="Zoom in" shortcut={[mod(), '=']} icon={<ZoomIn />} onClick={actions['zoom.in']} />
            <IconButton label="Zoom out" shortcut={[mod(), '−']} icon={<ZoomOut />} onClick={actions['zoom.out']} />
            <Button size="sm" variant="ghost" onClick={actions['zoom.all']}>
              Whole capture
            </Button>
            <span className="toolbar__sep" />
            <IconButton label="Undo" shortcut={[mod(), 'Z']} icon={<Undo2 />} onClick={actions['history.undo']} />
            <IconButton label="Redo" icon={<Redo2 />} onClick={actions['history.redo']} />
            <span className="toolbar__grow" />
            <Button size="sm" variant="ghost" icon={<Save />} onClick={actions['file.save']}>
              Save
            </Button>
            <IconButton label="Bottom panel" shortcut={[mod(), 'J']} pressed={showBottom} icon={<PanelBottom />} onClick={actions['view.bottom']} />
            <IconButton label="Inspector" shortcut={[mod(), 'I']} pressed={showInspector} icon={<PanelRight />} onClick={actions['view.inspector']} />
          </div>
        )}
      </header>

      {active && view ? (
        <>
          <div className="workspace">
            {showSidebar && (
              <aside className="sidebar" aria-label="Channels">
                <div className="sidebar__content" style={{ width: sidebarW }}>
                  <h2 className="section-title pad-x">Channels</h2>
                  <ChannelList capture={active} />
                </div>
                <Splitter orientation="vertical" label="Sidebar width" value={sidebarW} min={180} max={420} onChange={setSidebarW} />
              </aside>
            )}
            <main className="center">
              <section className="wave-area" aria-label="Waveform">
                <Waveform capture={active} view={view} onView={(v) => setView(active.id, v)} expert={expert} />
              </section>
              {showBottom && (
                <>
                  <Splitter orientation="horizontal" label="Bottom panel height" value={bottomH} min={120} max={600} onChange={setBottomH} invert />
                  <section className="bottom" style={{ height: bottomH }} aria-label="Bottom panel">
                    <TabsRoot value={bottomTab} onValueChange={setBottomTab} className="tabs-fill">
                      <TabsList label="Bottom panel">
                        <TabsTrigger value="decoded">Decoded</TabsTrigger>
                        <TabsTrigger value="measure">Measurements</TabsTrigger>
                        <TabsTrigger value="activity">Activity</TabsTrigger>
                      </TabsList>
                      <TabsContent value="decoded" className="fill">
                        <DecodedTable capture={active} onJump={jump} />
                      </TabsContent>
                      <TabsContent value="measure" className="fill">
                        <Measurements capture={active} view={view} />
                      </TabsContent>
                      <TabsContent value="activity" className="fill">
                        <ActivityFeed />
                      </TabsContent>
                    </TabsRoot>
                  </section>
                </>
              )}
            </main>
            {showInspector && (
              <aside className="inspector" data-drawer={narrow || undefined} aria-label="Inspector">
                {!narrow && <Splitter orientation="vertical" label="Inspector width" value={inspectorW} min={260} max={560} onChange={setInspectorW} invert />}
                <div className="inspector__content" style={{ width: inspectorW }}>
                  <TabsRoot value={inspector} onValueChange={setInspector} className="tabs-fill">
                    <TabsList label="Inspector">
                      <TabsTrigger value="analysis">Analysis</TabsTrigger>
                      <TabsTrigger value="decoders">Decoders</TabsTrigger>
                      <TabsTrigger value="search">Search</TabsTrigger>
                      <TabsTrigger value="device">Device</TabsTrigger>
                    </TabsList>
                    <TabsContent value="analysis" className="fill">
                      <AnalysisPanel capture={active} expert={expert} view={view} />
                    </TabsContent>
                    <TabsContent value="decoders" className="fill">
                      <DecodersPanel capture={active} />
                    </TabsContent>
                    <TabsContent value="search" className="fill">
                      <SearchPanel key={active.id} capture={active} onJump={jump} expert={expert} />
                    </TabsContent>
                    <TabsContent value="device" className="fill">
                      <DevicePanel />
                    </TabsContent>
                  </TabsRoot>
                </div>
              </aside>
            )}
          </div>
          <StatusBar capture={active} view={view} />
        </>
      ) : (
        <StartScreen onOpen={actions['file.open']!} onDemo={actions.demo!} onOpenRecent={(f) => void openFiles([f])} onAgent={actions['help.agent']!} />
      )}

      <CommandPalette open={palette} onOpenChange={setPalette} items={items} />
      <ShortcutsDialog open={help === 'keys'} onClose={() => setHelp(null)} />
      <AgentDialog open={help === 'agent'} onClose={() => setHelp(null)} />
      <RunCommandDialog command={runCommand} onClose={() => setRunCommand(null)} />
      <Notices />
    </div>
  );
}

function AgentIndicator() {
  const { session, mcp, startedByAgent } = useStore();
  const agents = new Set((session?.recent ?? []).map((h) => h.actor).filter(isAgent));
  const label = mcp.port ? `${agents.size ? `${agents.size} agent${agents.size === 1 ? '' : 's'} active` : 'Agents can connect'}` : 'MCP is off';
  return (
    <div className="agent-indicator" title={mcp.error ?? (mcp.port ? `MCP at http://127.0.0.1:${mcp.port}/mcp` : 'Switch MCP on in the settings')}>
      {startedByAgent && <Badge tone="agent">started by an agent</Badge>}
      {session?.agentsPaused && <Badge tone="warning">agent actions stopped</Badge>}
      <Badge tone={agents.size ? 'agent' : 'neutral'}>
        <Bot aria-hidden="true" size={12} /> {label}
      </Badge>
    </div>
  );
}

function StatusBar({ capture, view }: { capture: CaptureInfo; view: ViewState }) {
  const { session, mcp } = useStore();
  const rate = capture.sampleRateHz;
  const running = session?.running.length ?? 0;
  const device = session?.devices.find((d) => d.connected);
  const loss = capture.notes.find((n) => n.level === 'warning' && /lost|overwrit|dropped/i.test(n.message));
  return (
    <footer className="status" aria-label="Status">
      <span>{running ? `Capturing (${running})` : device ? `${device.info.model}: connected` : 'No device connected'}</span>
      <span className="lg-num">{hz(rate)}</span>
      <span className="lg-num">
        {time(view.from / rate)} – {time(view.to / rate)} ({int(view.to - view.from)} samples)
      </span>
      {view.cursorA !== null && view.cursorB !== null && <span className="lg-num">Δt {time((view.cursorB - view.cursorA) / rate)}</span>}
      {loss && <span className="status__warn">{loss.message}</span>}
      <span className="status__grow" />
      <span>{mcp.port ? `MCP on 127.0.0.1:${mcp.port}` : mcp.error ? 'MCP not started' : 'MCP off'}</span>
    </footer>
  );
}

function StartScreen({ onOpen, onDemo, onOpenRecent, onAgent }: { onOpen: () => void; onDemo: () => void; onOpenRecent: (f: string) => void; onAgent: () => void }) {
  const { settings, session } = useStore();
  const recent = settings?.recent ?? [];
  return (
    <main className="start">
      <div className="start__inner">
        <h2 className="start__title">Start</h2>
        <p className="muted">Open a recording, or capture from a device. Drop a file anywhere on this window to open it.</p>
        <div className="row">
          <Button variant="primary" icon={<FolderOpen />} onClick={onOpen}>
            Open file…
          </Button>
          <Button icon={<FlaskConical />} onClick={onDemo}>
            Try a demo capture
          </Button>
        </div>
        <div className="start__grid">
          <section className="card">
            <h2 className="section-title">Recent captures</h2>
            {recent.length === 0 ? (
              <p className="muted">Files you open appear here.</p>
            ) : (
              <ul className="plain">
                {recent.map((f) => (
                  <li key={f}>
                    <button type="button" className="link" onClick={() => onOpenRecent(f)} title={f}>
                      {f.split(/[\\/]/).pop()}
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </section>
          <section className="card">
            <h2 className="section-title">Devices</h2>
            <ul className="plain">
              {(session?.devices ?? []).slice(0, 6).map((d) => (
                <li key={d.info.id}>
                  {d.info.model} <span className="muted">{d.connected ? '· connected' : ''}</span>
                </li>
              ))}
            </ul>
            <p className="muted">Real analysers come with milestone M3; these synthetic devices produce known signals.</p>
          </section>
          <section className="card">
            <h2 className="section-title">Work with an agent</h2>
            <p className="muted">Claude Code and other MCP clients can open, decode and mark captures with you, in this window.</p>
            <Button size="sm" icon={<Bot />} onClick={onAgent}>
              How to connect
            </Button>
          </section>
        </div>
      </div>
    </main>
  );
}

function Notices() {
  const { notices, dismiss } = useStore();
  return (
    <div className="notices" role="status" aria-live="polite">
      {notices.map((n) => (
        <div key={n.id} className="notice" data-tone={n.tone}>
          <span>{n.text}</span>
          <IconButton label="Dismiss" icon={<X />} onClick={() => dismiss(n.id)} />
        </div>
      ))}
    </div>
  );
}

const SHORTCUTS: [string, string][] = [
  ['Command palette', 'Ctrl+K'],
  ['Open a file', 'Ctrl+O'],
  ['Save', 'Ctrl+S'],
  ['Undo your last action', 'Ctrl+Z'],
  ['Redo', 'Ctrl+Y / Cmd+Shift+Z'],
  ['Sidebar, inspector, bottom panel', 'Ctrl+B, Ctrl+I, Ctrl+J'],
  ['Find in the capture', 'Ctrl+F'],
  ['Zoom in, out, whole capture', 'Ctrl+=, Ctrl+−, Ctrl+0'],
  ['Pan the waveform (focused)', '← → (Shift: half a view)'],
  ['Zoom the waveform (focused)', '+ and −, or the wheel'],
  ['Start, end of the capture', 'Home, End'],
  ['Cursor A, cursor B', 'Click, Alt+click'],
  ['Select a range', 'Shift+drag'],
  ['Clear cursors and selection', 'Esc'],
  ['These shortcuts', '? or F1'],
];

function ShortcutsDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()} title="Keyboard shortcuts" wide footer={<Button onClick={onClose}>Close</Button>}>
      <dl className="facts">
        {SHORTCUTS.map(([what, keys]) => (
          <div key={what} className="facts__row">
            <dt>{what}</dt>
            <dd>
              <code>{keys}</code>
            </dd>
          </div>
        ))}
      </dl>
    </Dialog>
  );
}

function AgentDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { mcp } = useStore();
  return (
    <Dialog
      open={open}
      onOpenChange={(o) => !o && onClose()}
      title="Connect an agent"
      description="Agents use Logica over MCP and work in this window with you. What they do appears in the activity feed, where you can undo it or stop them."
      footer={
        <>
          <Button onClick={() => void navigator.clipboard.writeText(AGENT_SETUP)}>Copy the command</Button>
          <Button variant="primary" onClick={onClose}>
            Done
          </Button>
        </>
      }
    >
      <p>For Claude Code, run once:</p>
      <pre className="query">{AGENT_SETUP}</pre>
      <p className="muted">
        {mcp.port ? `MCP is on (127.0.0.1:${mcp.port}); logica mcp finds it by itself.` : mcp.error ? mcp.error : 'MCP is off; switch it on in the settings.'}
      </p>
    </Dialog>
  );
}

function RunCommandDialog({ command, onClose }: { command: string | null; onClose: () => void }) {
  const { run } = useStore();
  const active = useActive();
  const [input, setInput] = useState('{}');
  const [output, setOutput] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    setInput(active && command && command !== 'session.get' ? JSON.stringify({ capture: active.id }, null, 2) : '{}');
    setOutput(null);
    setError(null);
  }, [command, active]);
  const spec = COMMANDS.find((c) => c.name === command);
  const go = async () => {
    let parsed: unknown;
    try {
      parsed = JSON.parse(input);
    } catch (e) {
      setError(`The input is not JSON: ${String(e)}`);
      return;
    }
    setError(null);
    const r = await run(command as never, parsed as never);
    if (r !== undefined) setOutput(JSON.stringify(r, null, 2).slice(0, 20000));
  };
  return (
    <Dialog
      open={command !== null}
      onOpenChange={(o) => !o && onClose()}
      title={command ?? ''}
      description={spec?.summary ?? ''}
      wide
      footer={
        <>
          <Button onClick={onClose}>Close</Button>
          <Button variant="primary" onClick={() => void go()}>
            Run
          </Button>
        </>
      }
    >
      <label className="lg-field">
        <span className="lg-field__label">Input (JSON; its schema is in the command API contract)</span>
        <textarea className="lg-input code-input" rows={6} value={input} onChange={(e) => setInput(e.target.value)} aria-invalid={error ? true : undefined} />
      </label>
      {error && <p className="lg-field__error" role="alert">{error}</p>}
      {output && <pre className="query output">{output}</pre>}
      {!output && spec && spec.undo.kind === 'never' && <p className="muted">This cannot be undone: {spec.undo.reason}</p>}
    </Dialog>
  );
}
