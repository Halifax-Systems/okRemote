# okRemote

A desktop connection manager for SSH, RDP, VNC, web and external-tool sessions. Hosts live in an encrypted vault file, organised in folders that pass down credentials and settings. Runs on macOS, Windows and Linux. No telemetry.

## Features

- **Encrypted vault**: one `.okvault` file (Argon2id + XChaCha20-Poly1305), with backups and an audit log
- **SSH terminal** with key, agent, password and keyboard-interactive login
- **Built-in RDP client**; VNC, web and custom tools open through your installed clients
- **Jump chains and port forwards** (local, SOCKS5, remote)
- **Tabs, split panes**, command palette and quick connect
- **Import** from OpenSSH config, PuTTY, RDCMan, CSV and JSON

## Build

Requires [Rust](https://rustup.rs) 1.85+ and Node.js 20+. On Linux, install `libwebkit2gtk-4.1-dev libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev libdbus-1-dev`.

```sh
cd app
npm install
npx tauri dev      # run
npx tauri build    # installers
```

Run the tests with `cargo test --workspace`.

## Layout

| Path | Contents |
| --- | --- |
| `crates/` | Core libraries: model, crypto, vault store, importers |
| `app/src-tauri/` | Desktop shell, SSH, RDP and tunnels |
| `app/src/` | React + TypeScript interface |
