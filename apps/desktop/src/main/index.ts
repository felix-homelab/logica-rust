// The main process (doc 03): windows and the engine process, nothing else. Command traffic goes
// from the window to the engine over a MessagePort and never passes through here.
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { app, BrowserWindow, ipcMain, MessageChannelMain, utilityProcess, type UtilityProcess } from 'electron';

// when the process started, for the cold start measurement (spike S7)
const launchedAt = Date.now() - process.uptime() * 1000;
const smoke = process.env.LOGICA_SMOKE === '1';

// the data folder (doc 03); tests and manual runs point it elsewhere
if (process.env.LOGICA_DATA_DIR) {
  app.setPath('userData', process.env.LOGICA_DATA_DIR);
}

/** Where the engine's Node-API module is: unpacked next to the app when packaged. */
function nativeModule(): string {
  return app.isPackaged
    ? join(process.resourcesPath, 'native', 'logica.node')
    : join(app.getAppPath(), 'native', 'logica.node');
}

let engine: UtilityProcess | undefined;

function start(): void {
  engine = utilityProcess.fork(join(__dirname, 'host.js'), [], {
    serviceName: 'Logica engine',
    env: {
      ...process.env,
      LOGICA_NATIVE: nativeModule(),
      LOGICA_WORK_DIR: join(app.getPath('userData'), 'captures'),
    },
  });
  engine.on('exit', (code) => {
    if (code !== 0) {
      console.error(`logica: the engine process ended with code ${code}.`);
    }
    engine = undefined;
  });

  const win = new BrowserWindow({
    width: 1100,
    height: 720,
    show: false,
    title: 'Logica',
    webPreferences: {
      preload: join(__dirname, '../preload/index.js'),
      sandbox: true,
      contextIsolation: true,
      nodeIntegration: false,
    },
  });
  // no navigation and no new windows (doc 03 security baseline)
  win.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));
  win.webContents.on('will-navigate', (e) => e.preventDefault());

  const { port1, port2 } = new MessageChannelMain();
  engine.postMessage({ type: 'port' }, [port1]);
  win.webContents.once('did-finish-load', () => {
    win.webContents.postMessage('logica:engine-port', { launchedAt }, [port2]);
  });
  win.once('ready-to-show', () => win.show());

  const url = process.env.ELECTRON_RENDERER_URL;
  if (!app.isPackaged && url) {
    void win.loadURL(url);
  } else {
    void win.loadFile(join(__dirname, '../renderer/index.html'));
  }
}

ipcMain.once('logica:ready', (e, ms: unknown) => {
  console.log(`logica: cold start ${typeof ms === 'number' ? Math.round(ms) : '?'} ms`);
  const done = (): void => {
    if (smoke) {
      app.quit();
    }
  };
  // smoke runs can keep a picture of the window, to check by eye what it showed
  const shot = process.env.LOGICA_SCREENSHOT;
  if (shot) {
    setTimeout(() => {
      void e.sender.capturePage().then((img) => {
        writeFileSync(shot, img.toPNG());
        done();
      });
    }, 300);
  } else {
    done();
  }
});

ipcMain.once('logica:failed', (_e, message: unknown) => {
  console.error(`logica: the window could not reach the engine: ${String(message)}`);
  if (smoke) {
    app.exit(1);
  }
});

app.on('window-all-closed', () => app.quit());
app.on('will-quit', () => engine?.kill());
void app.whenReady().then(start);
