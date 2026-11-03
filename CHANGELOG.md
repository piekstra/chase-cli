# Changelog

All notable changes to `chase-cli` are documented here. Newest first. The project
follows [SemVer](https://semver.org/); until 1.0, minors carry the breaking-change
signal.

## 0.2.0 — 2026-11-03

- `chse mcp`: read-only tools for Claude, `chase_transactions` and `chase_exports`, over
  the CSV exports in your configured `inbox`. They run over stdio for Claude Code and
  Claude Desktop, and with `--http` on 127.0.0.1 behind a bearer token for Claude on the
  web and mobile through your own Cloudflare Worker and tunnel. No tool takes a path, so
  a model can read nothing outside that folder.
- `mcp --new-token` can write one keychain item, the `--http` bearer token. That is the
  first credential this CLI can hold; it signs in to nothing at Chase.
- Releases are cut by pushing a tag, no longer by merging a version bump. They attach
  `chse-<version>.mcpb`, a Claude Desktop extension, and a Windows x86_64 tarball, and a
  release is published only once every asset is there.
- `self-update` says to restart a running `chse mcp --http` and to install the new
  `.mcpb`.
- cli-common v0.21.0, up from v0.15.1. The minimum Rust version is 1.89.
