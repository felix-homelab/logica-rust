# ADR 0007: One engine per desktop; `logica mcp` launches the app

Status: accepted

## Context
If an agent's `logica mcp` started its own headless engine whenever no app was running, that engine
would hold the USB device, and a person opening the app to watch would see an empty window that
cannot open the device. The shared live view, a core differentiator, would fail silently in the
common case where the agent starts first.

## Decision
- On a machine with a display, `logica mcp` looks for a running app through the lock file. If none
  runs, it launches the app (window minimised) and relays to it. There is always exactly one engine
  on a desktop.
- A headless engine starts only with `logica mcp --headless` or when no display is available (CI,
  SSH sessions).
- A headless engine refuses `output` commands unless started with `--allow-outputs` (doc 07).
- If the app starts while a headless engine runs, it says so and offers to stop it; it never runs a
  second engine against the same devices.

## Consequences
- The shared live view works no matter who starts first.
- `logica mcp` must find the installed app (path recorded at install time, or next to the CLI in the
  installed bundle). A standalone `logica` without an installed app runs headless.
- Under WSL, `logica.exe mcp` through interop starts the Windows app, which is the owner's setup.
- An app launched by an agent opens minimised and shows "started by an agent" in the activity feed.
