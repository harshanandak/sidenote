# Sidenote

Sidenote is a desktop app for the meetings you record with [Hex](https://github.com/anomalyco/hex).
It lists your meetings and shows their transcripts, summaries and tasks, and it lets you
ask questions about them through an agent CLI or a model endpoint you choose.
Your meetings stay as files on your computer. There is no account.

Status: first public preview (version 0.1.0-dev.1). Expect rough edges.

## What works today

| Area | Windows | Linux |
| --- | --- | --- |
| Desktop window: meetings, transcripts, notes, tasks | Yes | Builds from source and opens; no package yet |
| Start and stop Hex recordings from the app | Yes | No |
| Install Hex from the app | Yes | No; install Hex yourself |
| Tray and floating meeting indicator | Yes | No |
| Automatic update checks | Packaged installs only; disabled in a source build | No |
| Ask through an agent CLI | Yes | Not yet; blocked in code |
| Notes and daily checks written by an agent | Yes, see "What you need" | Not yet; blocked in code |
| Headless server role (no window): MCP and folder sync | Not the main use | Yes, see [docs/linux-server.md](docs/linux-server.md) |

Linux limits in detail: [docs/linux-desktop.md](docs/linux-desktop.md) and
[docs/linux-processing.md](docs/linux-processing.md). On Linux a second launch exits
instead of focusing the open window, and save dialogs are not available yet.

## What you need

- **Hex** records and transcribes meetings. Sidenote reads the meeting folders Hex writes
  and talks to the running Hex app. Hex is a separate program; Sidenote does not change it.
- **An agent CLI for Ask**, such as Claude Code, Codex or OpenCode, signed in to a provider
  you can use. Ask can also use an OpenAI-compatible endpoint, including local servers
  such as Ollama or LM Studio. See [docs/cloud-ask.md](docs/cloud-ask.md).
- **Notes on Windows** use the in-app engine with Claude Code or Codex, or the PowerShell pipeline in `scripts/meeting-pipeline`. Configure the engine and helper in Settings. See [docs/meeting-pipeline.md](docs/meeting-pipeline.md) for the scripts engine.

What you do not need: an account, a Sidenote server, or any cloud service to read
meetings you already have. Reading saved meetings does not need an agent. Building
Sidenote and running its tests does not need Hex.

## Download

The one official source for Sidenote downloads is the [GitHub Releases page](https://github.com/harshanandak/sidenote/releases). No other site or mirror is official.

Each release includes a `SHA256SUMS` file. Download it with your installer, then run `Get-FileHash <file> -Algorithm SHA256` in PowerShell and compare the result with the matching line in `SHA256SUMS`.

Windows may show a "Windows protected your PC" prompt because the build is unsigned for now: choose More info, then Run anyway.

## Build from source

```sh
git clone https://github.com/harshanandak/sidenote.git
cd sidenote
./build.ps1    # Windows, PowerShell, with a MinGW GNU toolchain on PATH
RUSTUP_TOOLCHAIN=stable cargo build --no-default-features --features desktop --target x86_64-unknown-linux-gnu --bin sidenote    # Linux
```

Prerequisites, how to run it against empty test folders, and the checks to run before
a pull request are in [CONTRIBUTING.md](CONTRIBUTING.md).

## Where your data lives

Hex writes meetings to its own folder (`%APPDATA%\hex\meetings` on Windows).
On Windows, Sidenote uses `%APPDATA%\sidenote` if that folder already exists; otherwise it uses `%APPDATA%\hex-meetings`. Set `SIDENOTE_DATA_DIR`, `SIDENOTE_HEX_DIR` and `SIDENOTE_PEERS_DIR` to use other state, meeting and peer folders.
See [docs/configuration.md](docs/configuration.md) and [docs/mcp.md](docs/mcp.md).

## Privacy

- Sidenote stores meeting files and app state locally and has no Sidenote account. Enabling a provider, sync tool or remote host can send data to that service or device.
- Ask and enabled agent features send prompts and supplied context to the CLI or endpoint you select. MCP clients can read the corpus you expose; the client controls its further use.
- Other network features include model and Hex downloads, installed Windows update checks, provider model-list requests, configured knowledge-base fetches, and opt-in folder or relay sync. On Windows, speech-host consent enables status checks, and allowing audio uploads sends windows from new meetings to your configured HTTPS host. See [SECURITY.md](SECURITY.md) and [docs/sync.md](docs/sync.md).

## Documentation

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): where the code for each part lives.
- [docs/architecture/](docs/architecture/README.md): dated design decision records.
- [docs/sync.md](docs/sync.md), [docs/self-host-relay.md](docs/self-host-relay.md): sync between devices.
- [docs/audio-retention.md](docs/audio-retention.md): what happens to meeting audio.

## Contributing

Bug reports, fixes and documentation help are welcome. Start with
[CONTRIBUTING.md](CONTRIBUTING.md) and issues labelled
[good first issue](https://github.com/harshanandak/sidenote/labels/good%20first%20issue).
Please follow the [Code of Conduct](CODE_OF_CONDUCT.md). Report security problems
privately as described in [SECURITY.md](SECURITY.md).

## Licence

Sidenote is licensed under [AGPL-3.0-or-later](LICENSE). The `crates/meetings-domain`
crate is available under MIT or Apache-2.0. [THIRD_PARTY.md](THIRD_PARTY.md) lists the
notices for Hex, Rune, bundled fonts and the Cargo dependencies.
