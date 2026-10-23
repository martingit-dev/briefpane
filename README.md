# Briefpane

A desktop cockpit for coding agents. Claude Code and Codex keep running in
tmux as they do today; Briefpane attaches through tmux control mode and shows
the live pane, what the agent needs from you, the proof, and the code.

Design and decisions: `MG/06_Projects/briefpane/README.md` in the vault.

## Run

Needs tmux 3.2+ and the Tauri Linux prerequisites
(https://tauri.app/start/prerequisites/#linux).

```sh
pnpm install
pnpm tauri dev
```

Checks: `pnpm build` · `cargo test --manifest-path src-tauri/tmux-cc/Cargo.toml`
