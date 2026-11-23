// The main process (doc 03): windows, menus, native dialogs, file associations, the lock file and
// settings; nothing else. Command traffic goes from the window to the engine over a MessagePort
// and never passes through here.
import { writeFileSync, mkdirSync } from 'node:fs';
import { join } from 'node:path';
import {
  app,
  BrowserWindow,
  dialog,
  ipcMain,
  Menu,
  MessageChannelMain,
  nativeTheme,
  shell,
  utilityProcess,
  type MenuItemConstructorOptions,
  type UtilityProcess,
} from 'electron';
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

// one app per desktop: a second start hands its files to the first (ADR 0007, doc 09)
if (!smoke && !app.requestSingleInstanceLock({ files: settingsFile.filesIn(process.argv) })) {
  app.quit();
}

const dataDir = (): string => app.getPath('userData');
const settingsPath = (): string => join(dataDir(), 'settings.json');
let settings: Settings = settingsFile.DEFAULTS;
let engine: UtilityProcess | undefined;
let main: BrowserWindow | undefined;
let settingsWindow: BrowserWindow | undefined;
let mcpPort: number | null = null;
let mcpError: string | null = null;
const pendingFiles: string[] = settingsFile.filesIn(process.argv.slice(1));

function nativeModule(): string {
  return app.isPackaged ? join(process.resourcesPath, 'native', 'logica.node') : join(app.getAppPath(), 'native', 'logica.node');
}

/** Tells the CLI where the app is (`logica mcp` starts it, `logica open` hands it files). */
function recordInstall(): void {
  if (!app.isPackaged && !process.env.LOGICA_RECORD_APP) {
    return;
  }
  try {
    mkdirSync(dataDir(), { recursive: true });
    writeFileSync(join(dataDir(), 'app.json'), JSON.stringify({ executable: process.execPath, version: app.getVersion() }, null, 2));
  } catch (e) {
    console.error(`logica: app.json could not be written: ${String(e)}`);
  }
}

function applyTheme(): void {
  nativeTheme.themeSource = settings.theme;
}

function broadcast(channel: string, ...args: unknown[]): void {
  for (const w of BrowserWindow.getAllWindows()) {
    w.webContents.send(channel, ...args);
  }
}

function setSettings(next: Settings): void {
  const before = settings;
  settings = settingsFile.sanitize(next);
  settingsFile.save(settingsPath(), settings);
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

function startEngine(): void {
  engine = utilityProcess.fork(join(__dirname, 'host.js'), [], {
    serviceName: 'Logica engine',
    env: {
      ...process.env,
      LOGICA_NATIVE: nativeModule(),
      LOGICA_WORK_DIR: join(dataDir(), 'captures'),
      LOGICA_DATA_DIR: dataDir(),
      LOGICA_MCP: settings.mcp && !smoke ? '1' : '0',
    },
  });
  engine.on('message', (m: { type?: string; port?: number; error?: string }) => {
    if (m.type === 'mcp') {
      mcpPort = m.port ?? null;
      mcpError = m.error ?? null;
      broadcast('logica:mcp', { port: mcpPort, error: mcpError });
    }
  });
  engine.on('exit', (code) => {
    if (code !== 0) {
      console.error(`logica: the engine process ended with code ${code}.`);
    }
    engine = undefined;
  });
}

function connect(win: BrowserWindow): void {
  if (!engine) return;
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
}

function load(win: BrowserWindow, hash = ''): void {
  const url = process.env.ELECTRON_RENDERER_URL;
  if (!app.isPackaged && url) {
    void win.loadURL(`${url}${hash ? `#${hash}` : ''}`);
  } else {
    void win.loadFile(join(__dirname, '../renderer/index.html'), hash ? { hash } : {});
  }
}

const webPreferences = {
  preload: join(__dirname, '../preload/index.js'),
  sandbox: true,
  contextIsolation: true,
  nodeIntegration: false,
};

function createMain(): void {
  const win = (main = new BrowserWindow({
    width: 1440,
    height: 900,
    minWidth: 960,
    minHeight: 600,
    show: false,
    title: 'Logica',
    backgroundColor: '#16181d',
    webPreferences,
  }));
  secure(win);
  win.webContents.once('did-finish-load', () => {
    connect(win);
    win.webContents.send('logica:settings', settings);
    win.webContents.send('logica:mcp', { port: mcpPort, error: mcpError });
    win.webContents.send('logica:env', { packaged: app.isPackaged });
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
    height: 620,
    minWidth: 480,
    minHeight: 480,
    title: 'Logica settings',
    parent: main,
    backgroundColor: '#16181d',
    webPreferences,
  }));
  secure(win);
  win.setMenuBarVisibility(false);
  win.webContents.once('did-finish-load', () => {
    connect(win);
    win.webContents.send('logica:settings', settings);
    win.webContents.send('logica:mcp', { port: mcpPort, error: mcpError });
  });
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
  console.error(`logica: the window could not reach the engine: ${String(message)}`);
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
app.on('window-all-closed', () => app.quit());
app.on('will-quit', () => {
  engine?.postMessage({ type: 'mcp-stop' });
  engine?.kill();
});

void app.whenReady().then(() => {
  settings = settingsFile.load(settingsPath());
  applyTheme();
  recordInstall();
  buildMenu();
  startEngine();
  createMain();
});
