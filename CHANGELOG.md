# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Release engineering brought up to the praxec release standard:
  - `.github/workflows/release.yml` builds six targets (Linux, macOS and
    Windows on x86_64 and ARM64), smoke-tests every packaged binary with
    `scripts/mcp-smoke.mjs`, and aggregates `checksums.sha256` and
    `release-manifest.json` before publishing the draft release. The npm
    launcher steps are left as a clearly marked placeholder.
  - `.github/workflows/image.yml` builds `ghcr.io/praxec/scientific-process`
    from the checksum-verified Linux release binaries with
    `Dockerfile.release`, smoke-tests the image, and publishes `server.json`
    to the MCP registry best-effort.
  - `.github/workflows/installer-tests.yml` runs the installer behavior tests.
  - `scripts/check-version-sync.sh` (with `--tag` for release validation),
    `scripts/release-manifest.sh`, `scripts/mcp-smoke.mjs`, `scripts/install.sh`,
    `scripts/install.ps1` and `scripts/release.test.mjs`.
  - `deny.toml` and a `cargo-deny` CI job; clippy is now required to pass with
    `-D warnings`.
  - `docs/releasing.md`, the maintainer release runbook.
- CLI, agent skills and npm launcher, matching the praxec standard:
  - `scientific-process --version`, `--help` and
    `skills install|uninstall|list --target claude|codex|both (--dir <path>|--user)`,
    with no arguments still serving MCP on stdio.
  - Embedded `skills/` (three skills: `scientific-process`, `sp-investigate`,
    `sp-adjudicate`) installed with a manifest beside the skills that refuses
    to overwrite a modified or foreign skill.
  - `@matthew-cochran/scientific-process` npm launcher that downloads the
    release binary matching its version, verifies `checksums.sha256`, caches
    it and execs it with stdio passed through.

### Changed

- `server.json` names the OCI identifier with its version tag
  (`ghcr.io/praxec/scientific-process:0.1.0`); the OCI package carries no
  `version` field.

## [0.1.0] - 2026-08-04

### Added

- Initial release: an append-only scientific-process MCP server exposing
  `session.open`, `append`, `state.get`, `experiment.next`, `report.export`
  and `adjudicate` over stdio for hypotheses, experiments, observations,
  conclusions and verdicts.
