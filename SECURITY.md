# Security policy

## Reporting a vulnerability

**Do not open a public issue, pull request or discussion for a vulnerability.**

Report it privately through GitHub: *Security* → *Report a vulnerability*
(<https://github.com/felix-homelab/logica-rust/security/advisories/new>). This is the only
reporting channel. Please include:

- the affected version (`logica --version`) or commit, and the operating system;
- what an attacker can do, and under which conditions;
- steps to reproduce, or a proof of concept (for a file parser, the file);
- whether the issue is already public.

## What counts

For example: a capture file that makes Logica run code, write outside the folder it was asked
to, or crash in a way that loses other work; a way for a program or web page other than the
person's own agent to reach the MCP server (it listens on 127.0.0.1 with a token and checks
`Origin` and `Host`, [doc 07](docs/07-mcp-server.md)); a way around the window's sandbox; a way
to switch on a hardware output without the person confirming it in the app.

Not a vulnerability by itself: the installers are unsigned (decision D8 in
[doc 12](docs/12-decisions-and-risks.md)), and an agent the person connected can do what the MCP
tools allow.

## What happens next

| Step | Target |
|---|---|
| Acknowledgement | 3 working days |
| First assessment (valid or not, severity) | 10 working days |
| Fix released, critical severity | 14 days after the assessment |
| Fix released, high severity | 30 days after the assessment |
| Fix released, medium or low severity | With the next regular release |

Logica has one maintainer, so these are targets, not guarantees; if a target will be missed, the
reporter is told why and when to expect the fix.

The fix is developed in a private security advisory and released as a patch of the latest release
([RELEASING.md](RELEASING.md#security-releases)). The advisory, with a CVE where applicable, is
published when the fixed release is available. Reporters are credited unless they prefer not to
be.

## Supported versions

Only the latest release receives fixes ([RELEASING.md](RELEASING.md#supported-versions)). The app
updates itself on Windows and Linux; on macOS, download the latest release.
