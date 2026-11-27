// The main window (doc 08 layout): tabs, toolbar, channel sidebar, waveform, inspector, bottom
// panel and status bar; a start screen while nothing is open; the command palette.
import type { DragEvent } from 'react';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { COMMANDS } from '@logica/api';
import {
  Button,
  CommandPalette,
  Dialog,
  IconButton,
  Kbd,
  Splitter,
  Status,
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
import { at, elapsed, int, rate as formatRate, time } from '../format';
import { isAgent, useActive, useDeviceWatch, useStore, who, type CaptureInfo } from '../store';
import { adoptEngineView, centreOn, zoomAt } from '../viewmath';
import { DEFAULTS, LIMITS, fitPanels } from '../panelsize';
import { ActivityFeed, DecodedTable, Measurements } from './bottom';
import { AnalysisPanel, ChannelList, DecodersPanel, DevicePanel, SearchPanel } from './panels';
import { Waveform, type ViewState } from './Waveform';
import { LiveWaveform } from './Live';
import { CaptureDialog } from './CaptureDialog';
import { Boundary } from '../Boundary';

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
  // the engine's view of each tab as last seen, and the tabs whose view is still being sent
  const seen = useRef<Record<string, string>>({});
  const sending = useRef<Record<string, number>>({});
  const timers = useRef<Record<string, number>>({});
  const [palette, setPalette] = useState(false);
  const [help, setHelp] = useState<'keys' | 'agent' | null>(null);
  const [liveOpen, setLiveOpen] = useState(false);
  const [runCommand, setRunCommand] = useState<string | null>(null);
  const [inspector, setInspector] = useState('analysis');
  const [bottomTab, setBottomTab] = useState('decoded');
  const [showSidebar, setShowSidebar] = useStored('logica.sidebar', true);
  const [showInspector, setShowInspector] = useStored('logica.inspector', true);
  const [showBottom, setShowBottom] = useStored('logica.bottom', true);
  const [sidebarW, setSidebarW] = useStored<unknown>('logica.sidebarW', DEFAULTS.sidebar);
  const [inspectorW, setInspectorW] = useStored<unknown>('logica.inspectorW', DEFAULTS.inspector);
  const [bottomH, setBottomH] = useStored<unknown>('logica.bottomH', DEFAULTS.bottom);
  const [win, setWin] = useState({ width: window.innerWidth, height: window.innerHeight });
  // below 1280 px the inspector is a drawer over the workspace (doc 08): closed until asked for
  const narrow = win.width < 1280;
  const [drawer, setDrawer] = useState(false);
  const inspectorOpen = narrow ? drawer : showInspector;
  const openInspector = useCallback(() => (narrow ? setDrawer(true) : setShowInspector(true)), [narrow, setShowInspector]);
  const expert = settings?.expert ?? false;
  // remembered sizes, fitted to this window: the waveform always keeps room
  const panels = fitPanels({ sidebar: sidebarW, inspector: inspectorW, bottom: bottomH }, win, { sidebar: showSidebar, dockedInspector: !narrow && showInspector });

  // the active tab stays in sight when more tabs are open than fit
  const tabStrip = useRef<HTMLUListElement>(null);
  useEffect(() => {
    tabStrip.current?.querySelector('[data-active]')?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  }, [active?.id, session?.captures.length, win.width]);

  useEffect(() => {
    const f = () => setWin({ width: window.innerWidth, height: window.innerHeight });
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
        // the person steers a live view (it follows the newest data); the engine's grows behind it
        if (mine && c.state === 'capturing') {
          next[c.id] = mine;
          continue;
        }
        const adopt = adoptEngineView({
          mine: mine ? key(mine) : null,
          engine: key(engine),
          seen: seen.current[c.id] ?? null,
          sending: (sending.current[c.id] ?? 0) > 0,
        });
        next[c.id] = adopt || !mine ? engine : mine;
        seen.current[c.id] = key(engine);
      }
      return next;
    });
  }, [session]);

  /** Shows a view at once and tells the engine shortly after (agents see the person's view). */
  const setView = useCallback(
    (capture: string, v: ViewState) => {
      setViews((old) => ({ ...old, [capture]: v }));
      // the newest change counts: an earlier one finishing does not end the wait for it
      const generation = Date.now() + Math.random();
      sending.current[capture] = generation;
      window.clearTimeout(timers.current[capture]);
      timers.current[capture] = window.setTimeout(() => {
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
          .then(() => {
            // the engine has it now; only a later change by someone else replaces it
            seen.current[capture] = key(v);
          })
          .catch(() => {})
          .finally(() => {
            if (sending.current[capture] === generation) sending.current[capture] = 0;
          });
      }, 200);
    },
    [],
  );

  const view = active ? views[active.id] : undefined;
  const activeId = active?.id;
  const onActiveView = useCallback((v: ViewState) => activeId && setView(activeId, v), [activeId, setView]);

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

  // a file being opened is not opened again by a second click or drop
  const opening = useRef(new Set<string>());
  const openFiles = useCallback(
    async (files: string[]) => {
      for (const path of files) {
        if (opening.current.has(path)) continue;
        opening.current.add(path);
        const r = await run('file.open', { path }).finally(() => opening.current.delete(path));
        if (r) {
          bridge().opened(path);
          void client.run('view.set', { capture: r.capture.id, activate: true }).catch(() => {});
        }
      }
    },
    [run],
  );

  const openDialog = useCallback(async () => openFiles(await bridge().chooseFiles()), [openFiles]);

  /** Saves a capture; true when it was saved. */
  const saveCapture = useCallback(
    async (c: CaptureInfo, as: boolean): Promise<boolean> => {
      const isLacap = c.path?.toLowerCase().endsWith('.lacap');
      let path: string | null | undefined;
      if (as || !isLacap) {
        path = await bridge().chooseSave(`${c.title.replace(/\.[^.]+$/, '')}.lacap`);
        if (!path) return false;
      }
      const r = await run('file.save', { capture: c.id, ...(path ? { path } : {}) });
      if (r) notify('success', `Saved to ${r.path}.`);
      return !!r;
    },
    [run, notify],
  );

  const save = useCallback(
    async (as: boolean) => {
      if (active) await saveCapture(active, as);
    },
    [active, saveCapture],
  );

  // closing cannot be undone (the capture lives in the engine): ask first when work would be lost
  const [closing, setClosing] = useState<CaptureInfo | null>(null);
  const closeCapture = useCallback(
    (c: CaptureInfo) => {
      if (c.unsaved) setClosing(c);
      else void run('capture.close', { capture: c.id });
    },
    [run],
  );

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
      'capture.close': () => active && closeCapture(active),
      'history.undo': () => void run('history.undo', {}),
      'history.redo': () => void run('history.redo', {}),
      palette: () => setPalette(true),
      search: () => {
        openInspector();
        setInspector('search');
      },
      'view.sidebar': () => setShowSidebar(!showSidebar),
      'view.inspector': () => (narrow ? setDrawer(!drawer) : setShowInspector(!showInspector)),
      'view.bottom': () => setShowBottom(!showBottom),
      'zoom.in': () => zoom(0.5),
      'zoom.out': () => zoom(2),
      'zoom.all': () => zoom('all'),
      'help.keys': () => setHelp('keys'),
      'help.agent': () => setHelp('agent'),
      'capture.live': () => setLiveOpen(true),
      gallery: () => {
        window.location.hash = 'gallery';
      },
      settings: () => bridge().openSettings(),
    }),
    [openDialog, save, active, run, closeCapture, showSidebar, showInspector, showBottom, setShowSidebar, setShowInspector, setShowBottom, zoom, narrow, drawer, openInspector],
  );

  useEffect(() => bridge().onMenu((id) => actions[id]?.()), [actions]);
  useEffect(() => {
    if (!narrow) setDrawer(false);
    if (!(narrow && drawer)) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setDrawer(false);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [narrow, drawer]);
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
      { id: 'a-live', group: 'Actions', title: 'Capture live…', hint: 'from one or several devices', icon: <Activity />, run: actions['capture.live']! },
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
      hint: `${int(c.sampleCount)} samples at ${formatRate(c.sampleRateHz)}`,
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
        openInspector();
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
      <main className="boot" aria-busy={store.engine.state === 'failed' ? undefined : true}>
        {store.engine.state === 'failed' ? (
          <EngineFailed reason={store.engine.reason} />
        ) : (
          <p>{store.failure ? `The window could not reach the engine: ${store.failure}` : 'Starting the engine…'}</p>
        )}
      </main>
    );
  }

  return (
    <div className="app" onDragOver={(e) => e.preventDefault()} onDrop={onDrop}>
      <header className="head">
        <h1 className="lg-visually-hidden">{active ? `Logica: ${active.title}` : 'Logica'}</h1>
        <div className="tabs">
          <nav className="tabs__list" aria-label="Open captures">
            <ul
              className="tabs__ul"
              ref={tabStrip}
              onWheel={(e) => {
                // more tabs than fit: the wheel scrolls them sideways
                if (e.deltaX === 0) e.currentTarget.scrollLeft += e.deltaY;
              }}
            >
              {session.captures.map((c) => (
                <li key={c.id} className="tabs__tab" data-active={c.id === active?.id || undefined}>
                  <button
                    type="button"
                    aria-current={c.id === active?.id ? 'page' : undefined}
                    className="tabs__title"
                    title={c.title}
                    onClick={() => void client.run('view.set', { capture: c.id, activate: true }).catch(() => {})}
                  >
                    <span className="tabs__label">{c.title}</span>
                    {c.unsaved && c.state !== 'capturing' && (
                      <span className="tabs__unsaved" title="Unsaved changes">
                        <span className="tabs__dot" aria-hidden="true" />
                        <span className="lg-visually-hidden">, unsaved changes</span>
                      </span>
                    )}
                    {/* capturing: a red dot that pulses, the word for screen readers and the tooltip */}
                    {c.state === 'capturing' && (
                      <Status tone="danger" pulse title="Capturing">
                        <span className="lg-visually-hidden">capturing</span>
                      </Status>
                    )}
                  </button>
                  <IconButton className="tabs__close" label={`Close ${c.title}`} icon={<X />} onClick={() => closeCapture(c)} />
                </li>
              ))}
            </ul>
            <IconButton label="Open a capture" shortcut={[mod(), 'O']} icon={<Plus />} onClick={actions['file.open']} />
            <IconButton label="Capture live" icon={<Activity />} onClick={actions['capture.live']} />
          </nav>
          <button type="button" className="tabs__palette" onClick={() => setPalette(true)}>
            <Command aria-hidden="true" /> <span className="tabs__palette-label">Search commands…</span> <Kbd keys={[mod(), 'K']} />
          </button>
          <AgentIndicator />
        </div>
        <EngineBanner onReopen={(files) => void openFiles(files)} />
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
            <IconButton label="Inspector" shortcut={[mod(), 'I']} pressed={inspectorOpen} icon={<PanelRight />} onClick={actions['view.inspector']} />
          </div>
        )}
      </header>

      {active && view ? (
        <>
          <div className="workspace">
            {showSidebar && (
              <aside className="sidebar" aria-label="Channels">
                <div className="sidebar__content" style={{ width: panels.size.sidebar }}>
                  <h2 className="section-title pad-x">Channels</h2>
                  <Boundary name="channel list">
                    <ChannelList capture={active} />
                  </Boundary>
                </div>
                <Splitter
                  orientation="vertical"
                  label="Sidebar width"
                  value={panels.size.sidebar}
                  min={LIMITS.sidebar[0]}
                  max={panels.max.sidebar}
                  onChange={setSidebarW}
                />
              </aside>
            )}
            <main className="center">
              <section className="wave-area" aria-label="Waveform">
                <Boundary name="waveform">
                  {active.state === 'capturing' ? (
                    <LiveWaveform key={active.id} capture={active} view={view} onView={onActiveView} expert={expert} />
                  ) : (
                    <Waveform capture={active} view={view} onView={onActiveView} expert={expert} />
                  )}
                </Boundary>
              </section>
              {showBottom && (
                <>
                  <Splitter
                    orientation="horizontal"
                    label="Bottom panel height"
                    value={panels.size.bottom}
                    min={LIMITS.bottom[0]}
                    max={panels.max.bottom}
                    onChange={setBottomH}
                    invert
                  />
                  <section className="bottom" style={{ height: panels.size.bottom }} aria-label="Bottom panel">
                    <Boundary name="bottom panel">
                    <TabsRoot value={bottomTab} onValueChange={setBottomTab} className="tabs-fill">
                      <TabsList label="Bottom panel">
                        <TabsTrigger value="decoded">Decoded</TabsTrigger>
                        <TabsTrigger value="measure">Measurements</TabsTrigger>
                        <TabsTrigger value="activity">Activity</TabsTrigger>
                      </TabsList>
                      <TabsContent value="decoded" className="fill fill--own">
                        <DecodedTable capture={active} onJump={jump} />
                      </TabsContent>
                      <TabsContent value="measure" className="fill">
                        <Measurements capture={active} view={view} />
                      </TabsContent>
                      <TabsContent value="activity" className="fill fill--own">
                        <ActivityFeed />
                      </TabsContent>
                    </TabsRoot>
                    </Boundary>
                  </section>
                </>
              )}
            </main>
            {narrow && drawer && <div className="drawer-scrim" aria-hidden="true" onClick={() => setDrawer(false)} />}
            {inspectorOpen && (
              <aside
                className="inspector"
                data-drawer={narrow || undefined}
                aria-label="Inspector"
                onKeyDown={(e) => {
                  if (narrow && e.key === 'Escape') {
                    e.stopPropagation();
                    setDrawer(false);
                  }
                }}
              >
                {!narrow && (
                  <Splitter
                    orientation="vertical"
                    label="Inspector width"
                    value={panels.size.inspector}
                    min={LIMITS.inspector[0]}
                    max={panels.max.inspector}
                    onChange={setInspectorW}
                    invert
                  />
                )}
                <div className="inspector__content" style={{ width: panels.size.inspector }}>
                  {narrow && (
                    <div className="inspector__close">
                      <IconButton label="Close the inspector" shortcut={['Esc']} icon={<X />} onClick={() => setDrawer(false)} />
                    </div>
                  )}
                  <Boundary name="inspector">
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
                      <DevicePanel onCapture={actions['capture.live']!} />
                    </TabsContent>
                  </TabsRoot>
                  </Boundary>
                </div>
              </aside>
            )}
          </div>
          <StatusBar capture={active} view={view} />
        </>
      ) : (
        <StartScreen onOpen={actions['file.open']!} onCapture={actions['capture.live']!} onOpenRecent={(f) => void openFiles([f])} onAgent={actions['help.agent']!} />
      )}

      <CommandPalette open={palette} onOpenChange={setPalette} items={items} />
      <CaptureDialog open={liveOpen} onClose={() => setLiveOpen(false)} />
      <Dialog
        open={closing !== null}
        onOpenChange={(o) => !o && setClosing(null)}
        title={closing ? `Close ${closing.title}?` : ''}
        description="Its unsaved changes are lost: recordings, decoders, markers and names since the last save. Closing cannot be undone."
        footer={
          <>
            <Button onClick={() => setClosing(null)}>Cancel</Button>
            <Button
              onClick={async () => {
                const c = closing;
                if (c && (await saveCapture(c, false))) {
                  setClosing(null);
                  void run('capture.close', { capture: c.id });
                }
              }}
            >
              Save first
            </Button>
            <Button
              variant="danger"
              onClick={() => {
                const c = closing;
                setClosing(null);
                if (c) void run('capture.close', { capture: c.id });
              }}
            >
              Close without saving
            </Button>
          </>
        }
      />
      <ShortcutsDialog open={help === 'keys'} onClose={() => setHelp(null)} />
      <AgentDialog open={help === 'agent'} onClose={() => setHelp(null)} />
      <RunCommandDialog command={runCommand} onClose={() => setRunCommand(null)} />
      <Notices />
    </div>
  );
}

function EngineFailed({ reason }: { reason: string | undefined }) {
  return (
    <div className="stack-sm" role="alert">
      <p>
        <strong>The engine is not running.</strong> {reason}
      </p>
      <p className="muted">The details are in the log file (Help, Open the log folder).</p>
      <div className="row">
        <Button variant="primary" onClick={() => bridge().restartEngine()}>
          Start the engine again
        </Button>
      </div>
    </div>
  );
}

/** What happened to the engine, and what can be done: shown under the tabs. */
function EngineBanner({ onReopen }: { onReopen: (files: string[]) => void }) {
  const { engine, lost, dismissLost } = useStore();
  if (engine.state === 'restarting') {
    return (
      <div className="banner" data-tone="warning" role="alert">
        <span className="banner__text">{engine.reason ?? 'The engine stopped and is being restarted.'}</span>
      </div>
    );
  }
  if (engine.state === 'failed') {
    return (
      <div className="banner" data-tone="danger" role="alert">
        <span className="banner__text">{engine.reason}</span>
        <Button size="sm" variant="primary" onClick={() => bridge().restartEngine()}>
          Start the engine again
        </Button>
      </div>
    );
  }
  if (engine.state === 'running' && lost) {
    return (
      <div className="banner" data-tone="warning" role="alert">
        <span className="banner__text">
          The engine was restarted, so the open captures were closed.
          {lost.unsaved.length > 0 && ` Unsaved work in ${lost.unsaved.join(', ')} was lost.`}
        </span>
        {lost.files.length > 0 && (
          <Button
            size="sm"
            variant="primary"
            onClick={() => {
              onReopen(lost.files.map((f) => f.path));
              dismissLost();
            }}
          >
            Open {lost.files.length === 1 ? lost.files[0]!.title : `the ${lost.files.length} files`} again
          </Button>
        )}
        <Button size="sm" variant="ghost" onClick={dismissLost}>
          Dismiss
        </Button>
      </div>
    );
  }
  return null;
}

/** Who can work in this window besides the person: a dot and a word, in the agents' colour when one is. */
function useAgentState(): { tone: 'agent' | 'neutral' | 'warning'; label: string; detail: string } {
  const { session, mcp } = useStore();
  const agents = new Set((session?.recent ?? []).map((h) => h.actor).filter(isAgent));
  if (!mcp.port) return { tone: mcp.error ? 'warning' : 'neutral', label: mcp.error ? 'MCP not started' : 'MCP off', detail: mcp.error ?? 'Switch MCP on in the settings.' };
  const detail = `MCP at http://127.0.0.1:${mcp.port}/mcp`;
  if (agents.size) return { tone: 'agent', label: `${agents.size} agent${agents.size === 1 ? '' : 's'} active`, detail };
  return { tone: 'neutral', label: 'Agents can connect', detail };
}

function AgentIndicator() {
  const { session, startedByAgent } = useStore();
  const state = useAgentState();
  return (
    <div className="agent-indicator">
      {startedByAgent && <Status tone="agent">Started by an agent</Status>}
      {session?.agentsPaused && <Status tone="warning">Agent actions stopped</Status>}
      <Status tone={state.tone} title={state.detail}>
        {state.label}
      </Status>
    </div>
  );
}

function StatusBar({ capture, view }: { capture: CaptureInfo; view: ViewState }) {
  const { session, mcp, busy } = useStore();
  const rate = capture.sampleRateHz;
  const running = session?.running.length ?? 0;
  const device = session?.devices.find((d) => d.connected);
  const loss = capture.notes.find((n) => n.level === 'warning' && /lost|overwrit|dropped/i.test(n.message));
  return (
    <footer className="status" aria-label="Status">
      {/* what does not fit shortens with "…" (the whole text is in the tooltip); the rest keeps its size */}
      <span className="status__fit">
        {capture.state === 'capturing'
          ? `Capturing: ${elapsed(capture.seconds)} so far`
          : running
            ? `Capturing (${running})`
            : device
              ? `${device.info.model}: connected`
              : 'No device connected'}
      </span>
      <span className="lg-num status__keep">{formatRate(rate)}</span>
      <span className="lg-num status__fit">
        {time(view.from / rate)} – {time(view.to / rate)} ({int(view.to - view.from)} samples)
      </span>
      {view.cursorA !== null && view.cursorB !== null && <span className="lg-num status__keep">Δt {time((view.cursorB - view.cursorA) / rate)}</span>}
      {loss && (
        <span className="status__warn status__fit" title={loss.message}>
          {loss.message}
        </span>
      )}
      <span className="status__grow" />
      {busy && (
        <span className="status__busy status__fit" role="status" title={busy}>
          <span className="status__spinner" aria-hidden="true" />
          <span className="status__busy-text">{busy}</span>
        </span>
      )}
      <span className="status__keep">{mcp.port ? `MCP on 127.0.0.1:${mcp.port}` : mcp.error ? 'MCP not started' : 'MCP off'}</span>
    </footer>
  );
}

function StartScreen({ onOpen, onCapture, onOpenRecent, onAgent }: { onOpen: () => void; onCapture: () => void; onOpenRecent: (f: string) => void; onAgent: () => void }) {
  const { settings, session } = useStore();
  const recent = settings?.recent ?? [];
  // devices come and go (an analyser started, a device plugged in): look again while this shows
  useDeviceWatch(true);
  const devices = session?.devices ?? [];
  const agents = useAgentState();
  return (
    <main className="start">
      <div className="start__inner">
        <header className="start__head">
          <h2 className="start__title">Start</h2>
          <p className="muted">Open a recording, or capture from a device. You can also drop a file anywhere on this window.</p>
          <div className="row">
            <Button variant="primary" icon={<FolderOpen />} onClick={onOpen}>
              Open file…
            </Button>
            <Button icon={<Activity />} onClick={onCapture}>
              Capture live…
            </Button>
          </div>
        </header>
        <div className="start__cols">
          <section className="start__section" aria-labelledby="start-recent">
            <h3 id="start-recent" className="start__label">
              Recent
            </h3>
            {recent.length === 0 ? (
              <p className="muted start__empty">Files you open are listed here.</p>
            ) : (
              <ul className="start__list">
                {recent.map((f) => {
                  const parts = f.split(/[\\/]/);
                  const name = parts.pop() ?? f;
                  return (
                    <li key={f}>
                      <button type="button" className="start__item" onClick={() => onOpenRecent(f)} title={f}>
                        <span className="start__name">{name}</span>
                        <span className="start__where">{parts.join('/')}</span>
                      </button>
                    </li>
                  );
                })}
              </ul>
            )}
          </section>
          <div className="start__side">
            <section className="start__section" aria-labelledby="start-devices">
              <h3 id="start-devices" className="start__label">
                Devices
              </h3>
              {devices.length > 0 ? (
                <ul className="start__list">
                  {devices.map((d) => (
                    <li key={d.info.id} className="start__row">
                      <span className="start__name" title={d.info.id}>
                        {d.info.model}
                      </span>
                      {d.capturing ? (
                        <Status tone="danger" pulse>
                          capturing
                        </Status>
                      ) : d.connected ? (
                        <Status tone="success">connected</Status>
                      ) : (
                        <Status>available</Status>
                      )}
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="muted start__empty">
                  No analyser found. Real analysers come with milestone M3; to try live capture now, start the virtual analyser with <code>cargo run -p logica-vla</code>.
                </p>
              )}
            </section>
            <section className="start__section" aria-labelledby="start-agents">
              <h3 id="start-agents" className="start__label">
                Agents
              </h3>
              <div className="start__row">
                <Status tone={agents.tone} title={agents.detail}>
                  {agents.label}
                </Status>
              </div>
              <p className="muted start__empty">Claude Code and other MCP clients can open, decode and mark captures with you, in this window.</p>
              <div className="row">
                <Button size="sm" icon={<Bot />} onClick={onAgent}>
                  How to connect
                </Button>
              </div>
            </section>
          </div>
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
          <span className="notice__text">{n.text}</span>
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
