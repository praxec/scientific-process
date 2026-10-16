# Releasing

This is the maintainer runbook for cutting a scientific-process release. A tag
`vX.Y.Z` on `main` runs
[`.github/workflows/release.yml`](../.github/workflows/release.yml), which
builds and publishes the GitHub release, the container image and the MCP
registry entry, and attaches the npm launcher tarball. The npm package is then
published with [`scripts/npm-release.sh`](../scripts/npm-release.sh) (or by the
workflow's npm job once trusted publishing is set up), and crates.io by hand.

Throughout, `X.Y.Z` is the new version and `vX.Y.Z` its tag.

## 1. Bump the version

On a branch off `dev`, set the same version in the files that carry it:

- `Cargo.toml`: `[package] version`.
- `Cargo.lock`: run `cargo check` (or `cargo update -p scientific-process`) so
  the lock file records the new version.
- `server.json`: the top-level `version` and the tag of the OCI identifier
  (`ghcr.io/praxec/scientific-process:X.Y.Z`). The OCI package has no `version`
  field: the MCP registry rejects one.
- `npm/package.json`: the `@matthew-cochran/scientific-process` launcher
  version.

Then check them:

```sh
sh scripts/check-version-sync.sh
```

It fails when any `server.json` version, OCI identifier tag or
`npm/package.json` version differs from `Cargo.toml`. The release workflow
repeats the check against the tag (`scripts/check-version-sync.sh --tag
vX.Y.Z`) and refuses to build on a mismatch. Nothing else needs a version bump:
the README writes `<version>` for a release tag instead of pinning one.

## 2. Update the CHANGELOG

In [CHANGELOG.md](../CHANGELOG.md), rename `## [Unreleased]` to
`## [X.Y.Z] - YYYY-MM-DD` and add a fresh empty `## [Unreleased]` above it.
Follow Keep a Changelog: group entries under Added, Changed, Fixed and so on.
The date here is the planned release date; step 3 corrects it on the day.

Open the bump as a pull request into `dev` and merge it once CI is green.

## 3. Release PR from `dev` to `main`

On release day, first set the `## [X.Y.Z] - YYYY-MM-DD` date in
`CHANGELOG.md` to today's date (a one-line pull request into `dev`) if it
differs from the planned one.

Then open a pull request with base `main` and head `dev`. The gitflow guard
(`gitflow-guard.yml`) rejects any other head branch. Wait for every required
check, then merge it with a **merge commit**, not squash or rebase, so `main`
and `dev` keep a shared history.

## 4. Tag `main`

```sh
git fetch origin
git checkout main
git pull --ff-only origin main
git tag -a vX.Y.Z -m "vX.Y.Z"
git push origin vX.Y.Z
```

Tag only commits on `main`: the workflow triggers on any `v*` tag. The tag must
match `vMAJOR.MINOR.PATCH` (an optional pre-release or build suffix is
allowed) and equal the `Cargo.toml` and `server.json` versions.

The workflow then runs these jobs:

1. **validate + draft release**: runs `scripts/check-version-sync.sh --tag`
   against the versions and creates a draft GitHub release with generated
   notes.
2. **build**: six targets (Linux, macOS and Windows on x86_64 and ARM64). Each
   archive is smoke-tested with `scripts/mcp-smoke.mjs` (MCP `initialize` and
   `tools/list`) and uploaded to the draft.
3. **aggregate metadata + publish**: writes `checksums.sha256` and
   `release-manifest.json` (it fails if the matrix is incomplete), uploads them
   with `install.sh` and `install.ps1`, packs `npm/` with the agent skills
   bundled and attaches `matthew-cochran-scientific-process-X.Y.Z.tgz` plus
   `npm-package.sha256`, then publishes the release.
4. **npm publish**: publishes exactly the tarball step 3 attached, over npm
   trusted publishing (GitHub OIDC, no stored token). It is skipped unless the
   repository variable `NPM_TRUSTED_PUBLISHING` is `'true'`; the `npm`
   environment can require a maintainer's approval first. It skips a version
   already on npm and sends pre-releases to the `next` dist-tag.
5. **container image** (`.github/workflows/image.yml`): downloads the two
   Linux release archives, checks them against `checksums.sha256` and builds
   the image from those binaries with `Dockerfile.release`, so nothing is
   compiled under emulation. It smoke-tests the amd64 image over MCP, then
   pushes `ghcr.io/praxec/scientific-process` for linux/amd64 and linux/arm64
   with the tags `X.Y.Z`, `X.Y` and `latest`.
6. **MCP registry**: publishes `server.json` as
   `io.github.praxec/scientific-process` with a pinned, checksum-verified
   `mcp-publisher` over GitHub OIDC. This job is best-effort
   (`continue-on-error`), so a failure does not fail the release; check its log.

## 5. Publish to npm

The npm launcher (`npm/`, package `@matthew-cochran/scientific-process`)
downloads the GitHub release whose version equals its own, so publish it only
after the release is public (step 4 ends with the release published). Until
then `npx -y @matthew-cochran/scientific-process` fails.

Logged in to npm (`npm login`) as an owner of the `@matthew-cochran` scope and
to GitHub (`gh auth login`):

```sh
git checkout vX.Y.Z
bash scripts/npm-release.sh            # version from npm/package.json; or: bash scripts/npm-release.sh X.Y.Z
```

The script:

1. refuses to run unless you are logged in to npm and the GitHub release
   `vX.Y.Z` exists and is not a draft (a version already on npm is skipped);
2. downloads the `matthew-cochran-scientific-process-X.Y.Z.tgz` that
   release.yml attached, checks it against `npm-package.sha256`, checks that it
   holds only `LICENSE`, `NOTICE`, `README.md`, `package.json`, `bin/`, `lib/`
   and `skills/`, and that the agent skills are bundled;
3. publishes that file with `scripts/npm-publish.sh --local`, so npm prompts
   for your 2FA one-time password (pre-releases `X.Y.Z-rc.N` go to the `next`
   dist-tag);
4. waits (up to `SCIENTIFIC_PROCESS_RELEASE_WAIT_SECS`, default 180) for the
   registry to serve the version, then checks that
   `npx -y @matthew-cochran/scientific-process@X.Y.Z --version` prints
   `scientific-process X.Y.Z`.

It never tags, pushes or edits the release. No npm token is stored, and a
manual publish carries no provenance attestation (only a trusted-publishing run
can create one).

### Optional: npm trusted publishing

release.yml's **npm publish** job publishes the same tarball over GitHub OIDC
with provenance. It is off until you set it up:

1. On npmjs.com, the package's **Settings → Trusted publishing** names GitHub
   Actions, the repository `praxec/scientific-process`, the workflow
   **`release.yml`** (exactly; `release.yaml` makes the publish 404) and the
   environment `npm`.
2. On GitHub, the `npm` environment has required reviewers and allows only
   `v*` tags.
3. Set the repository variable `NPM_TRUSTED_PUBLISHING` to `true`.

The job then waits for approval in the `npm` environment after each release;
approving it is the whole npm step. It skips a version already on npm, so it is
safe after a manual publish.

## 6. Verify the release

Assets: the release page lists six archives, `checksums.sha256`,
`release-manifest.json`, `install.sh`, `install.ps1`,
`matthew-cochran-scientific-process-X.Y.Z.tgz` and `npm-package.sha256`.

```sh
gh release view vX.Y.Z --json assets --jq '.assets[].name'
gh release download vX.Y.Z --pattern checksums.sha256 --pattern '*linux-gnu.tar.gz'
sha256sum -c --ignore-missing checksums.sha256
```

Installer, on a clean shell (repeat with `install.ps1` on Windows):

```sh
curl -fsSL https://github.com/praxec/scientific-process/releases/latest/download/install.sh | sh -s -- --version vX.Y.Z
node scripts/mcp-smoke.mjs ~/.local/bin/scientific-process
```

Container image:

```sh
docker pull ghcr.io/praxec/scientific-process:X.Y.Z
node scripts/mcp-smoke.mjs docker run -i --rm ghcr.io/praxec/scientific-process:X.Y.Z
```

npm and skills:

```sh
npx -y @matthew-cochran/scientific-process@X.Y.Z --version
npx -y @matthew-cochran/scientific-process@X.Y.Z skills install --target claude --project "$(mktemp -d)" --dry-run
```

MCP registry: confirm the registry job succeeded, or look the server up in the
registry and check that its version is `X.Y.Z`.

## 7. Publish to crates.io (manual)

The workflow does not publish the crate. From the tagged commit, with a
crates.io token configured (`cargo login`):

```sh
git checkout vX.Y.Z
cargo publish --dry-run --locked
cargo package --list --locked
cargo publish --locked
```

Until this step runs, crates.io keeps serving the previous version. docs.rs
builds the new version on its own a few minutes after publishing.

## Rollback

- **A build failed before publishing.** The release is still a draft. Fix the
  problem on `dev`, then delete the draft and the tag and re-release with the
  next patch version (or re-push the same tag if nothing was published):

  ```sh
  gh release delete vX.Y.Z --yes
  git push origin :refs/tags/vX.Y.Z
  git tag -d vX.Y.Z
  ```

  Re-running the workflow on an existing draft is also safe: assets are
  uploaded with `--clobber`, and an existing release is left as is.
- **A bad release was published.** Do not reuse its version. Mark the GitHub
  release as a pre-release or edit its notes to warn users, then ship a fixed
  `X.Y.(Z+1)`. The installers resolve `releases/latest`, so they pick up the
  fix as soon as it is published.
- **A bad npm version was published.** npm versions cannot be reused.
  Deprecate it so installs warn, then publish a fix:

  ```sh
  npm deprecate @matthew-cochran/scientific-process@X.Y.Z "broken release, use X.Y.(Z+1)"
  ```

  `npm unpublish` works only within 72 hours and blocks the version number for
  24 hours; prefer deprecating and shipping the next patch.
- **A bad crate was published.** crates.io versions cannot be deleted or
  overwritten. Yank it so new lock files do not pick it up, then publish a fix:

  ```sh
  cargo yank --version X.Y.Z scientific-process
  # undo: cargo yank --version X.Y.Z --undo scientific-process
  ```

- **The image job failed or timed out.** The GitHub release is not affected.
  Rebuild the image for the existing release with
  `gh workflow run image.yml --ref main -f tag=vX.Y.Z`, which also runs the
  MCP registry step.
- **A bad image was pushed.** Push the fixed release, which moves `latest` and
  `X.Y`. Delete the bad `X.Y.Z` tag from the GHCR package settings only if it is
  harmful to keep.
- **A bad MCP registry entry.** Publish the fixed version; the registry lists
  the newest version.
