# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- crates.io package metadata (description, license, repository, authors, readme, keywords, categories), so the crate can be published to crates.io.

## [0.3.1] - 2026-10-22

### Security

- `rustls-webpki` 0.103.16 and `webpki-roots` 1.0.10 fix RUSTSEC-2026-0381 and
  RUSTSEC-2026-0382 (distrusted emSign intermediates) in the shipped binaries.

### Changed

- Builds, CI, releases and the Docker builder use the pinned Rust 1.99.1
  (`rust-toolchain.toml`).

## [0.3.0] - 2026-10-20

### Added

- `append`, `state.get`, `experiment.next` and `report.export` accept an
  optional per-call `session_id`, so per-call clients need no separate
  `session.open` (#12).

### Changed

- Agent skills trigger on evidence-driven work and document "Calling the
  tools", including the per-call `session_id` (#11).

## [0.2.0] - 2026-10-16

### Added

- Opt-in, paid Jev assist layer behind the default-on `jev` cargo feature,
  following crossmatrix's provider-neutral judge design. Four advisory MCP tools
  (`assist.experiment_check`, `assist.observation_support`,
  `assist.verdict_check`, `assist.hypothesis_check`) propose probabilities and
  concrete fixes without ever appending an event; the six core tools and the
  pure domain fold are unchanged. Config comes from `TYPESAFE_JEV_API_KEY` or
  `OPENROUTER_API_KEY`, gated by `SCIENTIFIC_PROCESS_ASSIST` and optionally
  tuned with `SCIENTIFIC_PROCESS_JEV_TIMEOUT_SECS`, `SCIENTIFIC_PROCESS_JEV_BASE_URL`
  and `SCIENTIFIC_PROCESS_JEV_MODEL`. With no key every assist tool reports
  `ASSIST_UNAVAILABLE`.
- Release engineering brought up to the praxec release standard:
  - `.github/workflows/release.yml` builds six targets (Linux, macOS and
    Windows on x86_64 and ARM64), smoke-tests every packaged binary with
    `scripts/mcp-smoke.mjs`, and aggregates `checksums.sha256` and
    `release-manifest.json` before publishing the draft release. The finalize
    job also packs the npm launcher with its bundled agent skills and attaches
    the tarball and `npm-package.sha256`; an `npm` job publishes that exact
    tarball over npm trusted publishing when `NPM_TRUSTED_PUBLISHING` is set.
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
  - `scripts/npm-publish.sh` (trusted publishing, skips an already-published
    version, pre-releases to `next`) and `scripts/npm-release.sh` (manual first
    publish with OTP), plus `npm/lib/sync-skills.js` to bundle `skills/` into
    the package.
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
  (`ghcr.io/praxec/scientific-process:0.2.0`); the OCI package carries no
  `version` field.

## [0.1.0] - 2026-08-04

### Added

- Initial release: an append-only scientific-process MCP server exposing
  `session.open`, `append`, `state.get`, `experiment.next`, `report.export`
  and `adjudicate` over stdio for hypotheses, experiments, observations,
  conclusions and verdicts.
