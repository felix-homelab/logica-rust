// The main process (doc 03): windows, menus, native dialogs, file associations, settings, and the
// supervision of the engine process; nothing else. Command traffic goes from the window to the
// engine over a MessagePort and never passes through here.
import { existsSync, readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { join, normalize, sep } from 'node:path';
import { pathToFileURL } from 'node:url';
import {
  app,
  BrowserWindow,
  dialog,
  ipcMain,
  Menu,
  MessageChannelMain,
  nativeTheme,
  net,
  protocol,
  screen,
  shell,
  utilityProcess,
  type MenuItemConstructorOptions,
  type UtilityProcess,
} from 'electron';
import { autoUpdater } from 'electron-updater';
import * as log from './log';
import { afterExit, restorable, type EngineState, type WindowState } from './policy';
import * as settingsFile from './settings';
import type { Settings } from './settings';

// when the process started, for the cold start measurement (spike S7)
const launchedAt = Date.now() - process.uptime() * 1000;
const smoke = process.env.LOGICA_SMOKE === '1';
const startedByAgent = process.argv.includes('--started-by-agent');

// the data folder (doc 03); tests and manual runs point it elsewhere
if (process.env.LOGICA_DATA_DIR) {
  app.setPath('userData', process.env.LOGICA_DATA_DIR);
}

// the window loads only the app's own files, through app:// (doc 03 security baseline)
protocol.registerSchemesAsPrivileged([{ scheme: 'app', privileges: { standard: true, secure: true, supportFetchAPI: true } }]);

/** Serves app://logica/<path> from the renderer folder, and nothing outside it. */
function serveApp(): void {
  const root = normalize(join(__dirname, '../renderer'));
  protocol.handle('app', (req) => {
    const path = decodeURIComponent(new URL(req.url).pathname);
    const file = normalize(join(root, path === '/' ? 'index.html' : path));
    if ((file !== root && !file.startsWith(root + sep)) || !existsSync(file)) {
      return new Response('Not found', { status: 404 });
    }
    return net.fetch(pathToFileURL(file).toString());
  });
}

// one app per desktop: a second start hands its files to the first (ADR 0007, doc 09)
if (!smoke && !app.requestSingleInstanceLock({ files: settingsFile.filesIn(process.argv) })) {
  app.quit();
}

// nothing that goes wrong in here is silent
process.on('uncaughtException', (e) => log.write('error', 'main', e.stack ?? String(e)));
process.on('unhandledRejection', (e) => log.write('error', 'main', e instanceof Error ? (e.stack ?? e.message) : String(e)));

const dataDir = (): string => app.getPath('userData');
const settingsPath = (): string => join(dataDir(), 'settings.json');
const windowPath = (): string => join(dataDir(), 'window.json');
let settings: Settings = settingsFile.DEFAULTS;
let engine: UtilityProcess | undefined;
let engineState: EngineState = { state: 'starting' };
let restarts = 0;
const crashes: number[] = [];
let quitting = false;
let main: BrowserWindow | undefined;
let settingsWindow: BrowserWindow | undefined;
/** Windows whose page has loaded: they get an engine port whenever an engine is ready. */
const loaded = new Set<BrowserWindow>();
let mcpPort: number | null = null;
let mcpError: string | null = null;
const pendingFiles: string[] = settingsFile.filesIn(process.argv.slice(1));

function nativeModule(): string {
  // a test can point it elsewhere, to check what happens when the engine cannot start
  if (process.env.LOGICA_NATIVE_PATH) return process.env.LOGICA_NATIVE_PATH;
  return app.isPackaged ? join(process.resourcesPath, 'native', 'logica.node') : join(app.getAppPath(), 'native', 'logica.node');
}

/** Tells the CLI where the app is (`logica mcp` starts it, `logica open` hands it files). */
function recordInstall(): void {
  if (!app.isPackaged && !process.env.LOGICA_RECORD_APP) return;
  try {
    mkdirSync(dataDir(), { recursive: true });
    writeFileSync(join(dataDir(), 'app.json'), JSON.stringify({ executable: process.execPath, version: app.getVersion() }, null, 2));
  } catch (e) {
    log.write('warn', 'main', `app.json could not be written: ${String(e)}`);
  }
}

function applyTheme(): void {
  nativeTheme.themeSource = settings.theme;
}

function broadcast(channel: string, ...args: unknown[]): void {
  for (const w of BrowserWindow.getAllWindows()) {
    if (!w.isDestroyed()) w.webContents.send(channel, ...args);
  }
}

function setSettings(next: Settings): void {
  const before = settings;
  settings = settingsFile.sanitize(next);
  try {
    settingsFile.save(settingsPath(), settings);
  } catch (e) {
    log.write('error', 'main', `The settings could not be saved: ${String(e)}`);
  }
  applyTheme();
  if (before.mcp !== settings.mcp) {
    engine?.postMessage({ type: settings.mcp ? 'mcp-start' : 'mcp-stop', dataDir: dataDir() });
  }
  broadcast('logica:settings', settings);
}

function openFiles(files: string[]): void {
  if (files.length === 0) return;
  if (main) {
    main.webContents.send('logica:open-files', files);
    if (main.isMinimized()) main.restore();
    main.focus();
  } else {
    pendingFiles.push(...files);
  }
}

function setEngineState(s: EngineState): void {
  engineState = s;
  broadcast('logica:engine', s);
}

/**
 * Starts the engine process and watches it. It says "ready" once its module is loaded; then every
 * loaded window gets a port. If it ends unexpectedly it is restarted (policy.afterExit), and the
 * windows learn why; a start failure is shown, not retried.
 */
function startEngine(): void {
  let fatal: string | null = null;
  const child = utilityProcess.fork(join(__dirname, 'host.js'), [], {
    serviceName: 'Logica engine',
    stdio: 'pipe',
    env: {
      ...process.env,
      LOGICA_NATIVE: nativeModule(),
      LOGICA_WORK_DIR: join(dataDir(), 'captures'),
      LOGICA_DATA_DIR: dataDir(),
      LOGICA_MCP: settings.mcp && !smoke ? '1' : '0',
      LOGICA_CACHE_MB: String(settings.memoryMb),
    },
  });
  engine = child;
  child.stdout?.on('data', (d: Buffer) => log.write('info', 'engine', d.toString()));
  child.stderr?.on('data', (d: Buffer) => log.write('error', 'engine', d.toString()));
  child.on('message', (m: { type?: string; port?: number; error?: string; message?: string }) => {
    if (m.type === 'ready') {
      setEngineState({ state: 'running', restarts });
      for (const w of loaded) connect(w);
    } else if (m.type === 'fatal') {
      fatal = m.message ?? 'unknown reason';
    } else if (m.type === 'mcp') {
      mcpPort = m.port ?? null;
      mcpError = m.error ?? null;
      if (mcpError) log.write('warn', 'main', `MCP: ${mcpError}`);
      broadcast('logica:mcp', { port: mcpPort, error: mcpError });
    }
  });
  child.on('exit', (code) => {
    if (engine === child) engine = undefined;
    if (quitting) return;
    mcpPort = null;
    broadcast('logica:mcp', { port: null, error: null });
    const next = afterExit(code, fatal, crashes, Date.now());
    log.write('error', 'main', `The engine process ended (exit code ${code}). ${next.reason}`);
    if (next.kind === 'fail') {
      setEngineState({ state: 'failed', reason: next.reason });
    } else {
      restarts++;
      setEngineState({ state: 'restarting', reason: next.reason, restarts });
      setTimeout(() => {
        if (!quitting) startEngine();
      }, next.delayMs);
    }
  });
}

function connect(win: BrowserWindow): void {
  if (!engine || win.isDestroyed()) return;
  const { port1, port2 } = new MessageChannelMain();
  engine.postMessage({ type: 'port' }, [port1]);
  win.webContents.postMessage('logica:engine-port', { launchedAt }, [port2]);
}

function secure(win: BrowserWindow): void {
  // no navigation and no new windows (doc 03 security baseline); links go to the browser
  win.webContents.setWindowOpenHandler(({ url }) => {
    if (url.startsWith('https://')) void shell.openExternal(url);
    return { action: 'deny' };
  });
  win.webContents.on('will-navigate', (e) => e.preventDefault());
  // a crashed or hung page is logged; a crashed one is loaded again
  win.webContents.on('render-process-gone', (_e, d) => {
    log.write('error', 'main', `A window's page ended (${d.reason}, exit code ${d.exitCode}); it is loaded again.`);
    if (!win.isDestroyed() && d.reason !== 'clean-exit') win.webContents.reload();
  });
  win.on('unresponsive', () => log.write('warn', 'main', 'A window is not responding.'));
  win.on('responsive', () => log.write('info', 'main', 'The window responds again.'));
}

function load(win: BrowserWindow, hash = ''): void {
  const url = process.env.ELECTRON_RENDERER_URL;
  if (!app.isPackaged && url) void win.loadURL(`${url}${hash ? `#${hash}` : ''}`);
  else void win.loadURL(`app://logica/index.html${hash ? `#${hash}` : ''}`);
}

const webPreferences = {
  preload: join(__dirname, '../preload/index.js'),
  sandbox: true,
  contextIsolation: true,
  nodeIntegration: false,
};

/** On every page load (a reload too): settings, state, and a port once the engine is ready. */
function onLoaded(win: BrowserWindow, extra: () => void): void {
  win.webContents.on('did-finish-load', () => {
    loaded.add(win);
    win.webContents.send('logica:settings', settings);
    win.webContents.send('logica:mcp', { port: mcpPort, error: mcpError });
    win.webContents.send('logica:env', { packaged: app.isPackaged, logFile: log.file() });
    win.webContents.send('logica:engine', engineState);
    if (engineState.state === 'running') connect(win);
    extra();
  });
  win.on('closed', () => loaded.delete(win));
}

function loadWindowState(): WindowState | null {
  try {
    const displays = screen.getAllDisplays().map((d) => d.workArea);
    return restorable(JSON.parse(readFileSync(windowPath(), 'utf8')), displays);
  } catch {
    return null;
  }
}

function saveWindowState(win: BrowserWindow): void {
  try {
    const state: WindowState = { bounds: win.getNormalBounds(), maximized: win.isMaximized() };
    writeFileSync(windowPath(), JSON.stringify(state));
  } catch (e) {
    log.write('warn', 'main', `The window's size could not be kept: ${String(e)}`);
  }
}

/** Asks the window which captures have unsaved changes (an empty list if it does not answer). */
function unsavedTitles(win: BrowserWindow): Promise<string[]> {
  return new Promise((resolve) => {
    const timer = setTimeout(() => {
      ipcMain.removeListener('logica:unsaved', reply);
      log.write('warn', 'main', 'The window did not say whether there are unsaved changes; closing.');
      resolve([]);
    }, 1500);
    const reply = (e: Electron.IpcMainEvent, titles: unknown) => {
      if (e.sender !== win.webContents) return;
      clearTimeout(timer);
      ipcMain.removeListener('logica:unsaved', reply);
      resolve(Array.isArray(titles) ? titles.filter((t): t is string => typeof t === 'string') : []);
    };
    ipcMain.on('logica:unsaved', reply);
    win.webContents.send('logica:before-close');
  });
}

function createMain(): void {
  const saved = loadWindowState();
  const win = (main = new BrowserWindow({
    width: saved?.bounds.width ?? 1440,
    height: saved?.bounds.height ?? 900,
    ...(saved ? { x: saved.bounds.x, y: saved.bounds.y } : {}),
    minWidth: 960,
    minHeight: 600,
    show: false,
    title: 'Logica',
    backgroundColor: '#16181d',
    webPreferences,
  }));
  if (saved?.maximized) win.maximize();
  secure(win);
  onLoaded(win, () => {
    if (startedByAgent) win.webContents.send('logica:started-by-agent');
    openFiles(pendingFiles.splice(0));
  });
  win.once('ready-to-show', () => {
    if (startedByAgent) {
      win.showInactive();
      win.minimize();
    } else {
      win.show();
    }
  });
  // closing with unsaved work asks first (captures exist only in the engine until saved)
  let confirmed = false;
  win.on('close', (e) => {
    saveWindowState(win);
    if (confirmed || smoke) return;
    e.preventDefault();
    void unsavedTitles(win).then((titles) => {
      if (titles.length > 0) {
        const choice = dialog.showMessageBoxSync(win, {
          type: 'warning',
          buttons: ['Close without saving', 'Cancel'],
          defaultId: 1,
          cancelId: 1,
          message: titles.length === 1 ? 'A capture has unsaved changes.' : `${titles.length} captures have unsaved changes.`,
          detail: `${titles.join('\n')}\n\nRecordings, decoders, markers and names since the last save are lost when Logica closes.`,
        });
        if (choice !== 0) {
          quitting = false;
          return;
        }
      }
      confirmed = true;
      win.close();
    });
  });
  win.on('closed', () => {
    main = undefined;
  });
  load(win);
}

function openSettings(): void {
  if (settingsWindow) {
    settingsWindow.focus();
    return;
  }
  const win = (settingsWindow = new BrowserWindow({
    width: 560,
    height: 640,
    minWidth: 480,
    minHeight: 480,
    title: 'Logica settings',
    parent: main,
    backgroundColor: '#16181d',
    webPreferences,
  }));
  secure(win);
  win.setMenuBarVisibility(false);
  onLoaded(win, () => {});
  win.on('closed', () => {
    settingsWindow = undefined;
  });
  load(win, 'settings');
}

async function chooseFiles(): Promise<string[]> {
  const r = await dialog.showOpenDialog(main ?? BrowserWindow.getFocusedWindow()!, {
    title: 'Open capture',
    properties: ['openFile', 'multiSelections'],
    filters: [
      { name: 'Captures', extensions: ['lacap', 'demo', 'sr', 'vcd'] },
      { name: 'All files', extensions: ['*'] },
    ],
  });
  return r.canceled ? [] : r.filePaths;
}

/** A menu item that runs an app action in the window. The window owns what actions do and their
 * keys; the menu shows the key without registering it, so it does not fire twice. */
const action = (label: string, id: string, accelerator?: string): MenuItemConstructorOptions => ({
  label,
  ...(accelerator ? { accelerator, registerAccelerator: false } : {}),
  click: () => main?.webContents.send('logica:menu', id),
});

function buildMenu(): void {
  const mac = process.platform === 'darwin';
  const template: MenuItemConstructorOptions[] = [
    ...(mac ? [{ role: 'appMenu' as const }] : []),
    {
      label: 'File',
      submenu: [
        { label: 'Open…', accelerator: 'CmdOrCtrl+O', registerAccelerator: false, click: () => void chooseFiles().then(openFiles) },
        {
          label: 'Open recent',
          submenu: settings.recent.length
            ? settings.recent.map((f) => ({ label: f, click: () => openFiles([f]) }))
            : [{ label: 'No recent captures', enabled: false }],
        },
        action('Capture live…', 'capture.live'),
        { type: 'separator' },
        action('Save', 'file.save', 'CmdOrCtrl+S'),
        action('Save as…', 'file.saveAs', 'CmdOrCtrl+Shift+S'),
        action('Close tab', 'capture.close', 'CmdOrCtrl+W'),
        { type: 'separator' },
        { label: 'Settings…', accelerator: 'CmdOrCtrl+,', registerAccelerator: false, click: openSettings },
        ...(mac ? [] : [{ type: 'separator' as const }, { role: 'quit' as const }]),
      ],
    },
    {
      label: 'Edit',
      submenu: [
        action('Undo', 'history.undo', 'CmdOrCtrl+Z'),
        action('Redo', 'history.redo', mac ? 'Cmd+Shift+Z' : 'Ctrl+Y'),
        { type: 'separator' },
        { role: 'cut' },
        { role: 'copy' },
        { role: 'paste' },
        { role: 'selectAll' },
        { type: 'separator' },
        action('Command palette…', 'palette', 'CmdOrCtrl+K'),
        action('Find in capture…', 'search', 'CmdOrCtrl+F'),
      ],
    },
    {
      label: 'View',
      submenu: [
        action('Sidebar', 'view.sidebar', 'CmdOrCtrl+B'),
        action('Inspector', 'view.inspector', 'CmdOrCtrl+I'),
        action('Bottom panel', 'view.bottom', 'CmdOrCtrl+J'),
        { type: 'separator' },
        action('Zoom in', 'zoom.in', 'CmdOrCtrl+='),
        action('Zoom out', 'zoom.out', 'CmdOrCtrl+-'),
        action('Show the whole capture', 'zoom.all', 'CmdOrCtrl+0'),
        { type: 'separator' },
        { role: 'togglefullscreen' },
        ...(app.isPackaged ? [] : [{ role: 'toggleDevTools' as const }, action('Component gallery', 'gallery')]),
      ],
    },
    {
      label: 'Help',
      submenu: [
        action('Keyboard shortcuts', 'help.keys', 'F1'),
        action('Connect an agent', 'help.agent'),
        { type: 'separator' },
        { label: 'Open the log folder', click: () => void shell.openPath(join(dataDir(), 'logs')) },
        { label: 'Documentation', click: () => void shell.openExternal('https://github.com/felix-homelab/logica-rust#readme') },
      ],
    },
  ];
  Menu.setApplicationMenu(Menu.buildFromTemplate(template));
}

ipcMain.handle('logica:choose-files', () => chooseFiles());
ipcMain.handle('logica:choose-save', async (_e, suggested: string) => {
  const r = await dialog.showSaveDialog(main ?? BrowserWindow.getFocusedWindow()!, {
    title: 'Save capture',
    defaultPath: suggested,
    filters: [{ name: 'Logica capture', extensions: ['lacap'] }],
  });
  return r.canceled ? null : (r.filePath ?? null);
});
ipcMain.handle('logica:settings-get', () => settings);
ipcMain.handle('logica:settings-set', (_e, next: Settings) => {
  setSettings(next);
  buildMenu();
  return settings;
});
ipcMain.on('logica:opened', (_e, file: string) => {
  setSettings(settingsFile.withRecent(settings, file));
  buildMenu();
});
ipcMain.on('logica:open-settings', () => openSettings());
ipcMain.on('logica:engine-restart', () => {
  if (engine || engineState.state !== 'failed') return;
  crashes.length = 0;
  log.write('info', 'main', 'The person restarts the engine.');
  setEngineState({ state: 'starting' });
  startEngine();
});
ipcMain.on('logica:log', (_e, level: unknown, message: unknown) => {
  const l = level === 'error' || level === 'warn' ? level : 'info';
  log.write(l, 'window', String(message).slice(0, 4000));
});

ipcMain.once('logica:ready', (e, ms: unknown) => {
  console.log(`logica: cold start ${typeof ms === 'number' ? Math.round(ms) : '?'} ms`);
  const done = (): void => {
    if (smoke) app.quit();
  };
  // smoke runs can keep a picture of the window, to check by eye what it showed
  const shot = process.env.LOGICA_SCREENSHOT;
  if (shot) {
    setTimeout(() => {
      void e.sender.capturePage().then((img) => {
        writeFileSync(shot, img.toPNG());
        done();
      });
    }, Number(process.env.LOGICA_SCREENSHOT_DELAY_MS ?? 300));
  } else {
    done();
  }
});

ipcMain.once('logica:failed', (_e, message: unknown) => {
  log.write('error', 'main', `The window could not reach the engine: ${String(message)}`);
  if (smoke) app.exit(1);
});

app.on('second-instance', (_e, argv, _cwd, extra) => {
  const files = (extra as { files?: string[] } | undefined)?.files ?? settingsFile.filesIn(argv);
  openFiles(files);
});
// macOS hands files over this way (file associations, drag onto the dock icon)
app.on('open-file', (e, file) => {
  e.preventDefault();
  openFiles([file]);
});
app.on('before-quit', () => {
  quitting = true;
});
app.on('window-all-closed', () => app.quit());
app.on('will-quit', () => {
  quitting = true;
  engine?.postMessage({ type: 'mcp-stop' });
  engine?.kill();
});

/** Updates from GitHub Releases (doc 11 M2): Windows, and Linux as an AppImage (the only Linux
 * format that can replace itself). Never in development, smoke runs, or when switched off. */
function checkUpdates(): void {
  const supported = process.platform === 'win32' || (process.platform === 'linux' && !!process.env.APPIMAGE);
  if (!app.isPackaged || smoke || !settings.updates || !supported) return;
  autoUpdater.on('update-downloaded', (info) => broadcast('logica:update', { version: info.version }));
  autoUpdater.on('error', (e) => log.write('warn', 'main', `The update check failed: ${e.message}`));
  void autoUpdater.checkForUpdatesAndNotify().catch(() => {});
}

void app.whenReady().then(() => {
  log.init(join(dataDir(), 'logs'));
  log.write('info', 'main', `Logica ${app.getVersion()} starts (${process.platform} ${process.arch}, Electron ${process.versions.electron}).`);
  serveApp();
  settings = settingsFile.load(settingsPath());
  applyTheme();
  recordInstall();
  buildMenu();
  startEngine();
  createMain();
  checkUpdates();
});
