// The sizes of the sidebar, inspector and bottom panel, fitted to the window (doc 08: panels are
// resizable and remember their size). A size remembered in a large window must not push the
// waveform off a small one, and a stored value that is not a number falls back to the default.

export interface PanelSizes {
  sidebar: number;
  inspector: number;
  bottom: number;
}

/** What each panel may be at most and at least, whatever the window. */
export const LIMITS: Record<keyof PanelSizes, [number, number]> = {
  sidebar: [180, 420],
  inspector: [260, 560],
  bottom: [120, 600],
};

export const DEFAULTS: PanelSizes = { sidebar: 240, inspector: 340, bottom: 240 };

/** The waveform keeps at least this much room next to and above the panels. */
export const MIN_WAVE_W = 320;
export const MIN_WAVE_H = 140;
/** Tab bar, toolbar, status bar and the bottom panel's splitter, in px (app.css). */
export const CHROME_H = 38 + 36 + 24 + 1;
/** A drawer leaves this much of the window uncovered, so it can be closed by clicking beside it. */
export const DRAWER_GAP = 48;

export interface Shown {
  sidebar: boolean;
  /** The inspector is beside the waveform (not a drawer over it). */
  dockedInspector: boolean;
}

export interface Fitted {
  /** The sizes to show. */
  size: PanelSizes;
  /** The largest each splitter may make its panel now. */
  max: PanelSizes;
}

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));
const sane = (v: unknown, fallback: number) => (typeof v === 'number' && Number.isFinite(v) ? Math.round(v) : fallback);

export function fitPanels(stored: Partial<Record<keyof PanelSizes, unknown>>, window: { width: number; height: number }, shown: Shown): Fitted {
  const [sMin, sMax] = LIMITS.sidebar;
  const [iMin, iMax] = LIMITS.inspector;
  const [bMin, bMax] = LIMITS.bottom;
  const want = {
    sidebar: sane(stored.sidebar, DEFAULTS.sidebar),
    inspector: sane(stored.inspector, DEFAULTS.inspector),
    bottom: sane(stored.bottom, DEFAULTS.bottom),
  };
  const room = window.width - MIN_WAVE_W;
  const max: PanelSizes = {
    sidebar: clamp(room - (shown.dockedInspector ? iMin : 0), sMin, sMax),
    inspector: 0,
    bottom: clamp(window.height - CHROME_H - MIN_WAVE_H, bMin, bMax),
  };
  const sidebar = clamp(want.sidebar, sMin, max.sidebar);
  max.inspector = shown.dockedInspector
    ? clamp(room - (shown.sidebar ? sidebar : 0), iMin, iMax)
    : clamp(window.width - DRAWER_GAP, iMin, iMax);
  return {
    size: {
      sidebar,
      inspector: clamp(want.inspector, iMin, max.inspector),
      bottom: clamp(want.bottom, bMin, max.bottom),
    },
    max,
  };
}
