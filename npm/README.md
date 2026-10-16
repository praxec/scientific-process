# @matthew-cochran/scientific-process

An `npx` launcher for
[scientific-process](https://github.com/praxec/scientific-process), an MCP
server for evidence-driven work: hypotheses, experiments, observations and
conclusions. The package contains no server code. On first run it downloads the
prebuilt `scientific-process` release binary for your platform from GitHub,
verifies its SHA-256 checksum, caches it, and runs it over stdio.

## Use it as an MCP server

Point your MCP client at:

```sh
npx -y @matthew-cochran/scientific-process
```

For example, in Claude Code:

```sh
claude mcp add scientific-process --scope user -- npx -y @matthew-cochran/scientific-process
```

Other clients take the same command (`npx`) and arguments (`-y`,
`@matthew-cochran/scientific-process`) in their MCP configuration.

All arguments are passed through to the binary, so the CLI subcommands work too:

```sh
npx -y @matthew-cochran/scientific-process --version
npx -y @matthew-cochran/scientific-process skills install --target claude --user
```

The package installs one command, `scientific-process`, which runs the launcher.

## What it does

1. Maps `process.platform`/`process.arch` to a release target:

   | Node platform/arch | Release asset |
   | --- | --- |
   | linux/x64 | `scientific-process-x86_64-unknown-linux-gnu.tar.gz` |
   | linux/arm64 | `scientific-process-aarch64-unknown-linux-gnu.tar.gz` |
   | darwin/x64 | `scientific-process-x86_64-apple-darwin.tar.gz` |
   | darwin/arm64 | `scientific-process-aarch64-apple-darwin.tar.gz` |
   | win32/x64 | `scientific-process-x86_64-pc-windows-msvc.zip` |
   | win32/arm64 | `scientific-process-aarch64-pc-windows-msvc.zip` |

   An x64 Node running under Rosetta on Apple Silicon gets the native arm64
   binary, as with `install.sh`. Other platforms fail with a clear error. The
   Linux binaries need glibc, so they do not run on musl systems such as Alpine.
2. Downloads
   `https://github.com/praxec/scientific-process/releases/download/v<version>/<asset>`
   and `checksums.sha256` from the same release, where `<version>` is this
   package's version. Only HTTPS is used. Redirects are followed only to
   `github.com`, `objects.githubusercontent.com` and
   `release-assets.githubusercontent.com` (or the mirror's own host), at most
   five times. Each download has a timeout and a 128 MiB size cap. A user and
   password in a mirror URL are never printed.
3. Verifies the archive's SHA-256 against `checksums.sha256` before extracting
   it. On any failure the partial files are deleted.
4. Extracts the binary with the system tools. A `.tar.gz` goes through `tar`,
   which ships with Linux, macOS and Windows 10 and later. A `.zip` (Windows)
   goes through PowerShell `Expand-Archive`. Archives with absolute paths, `..`
   segments or links are refused.
5. Moves the binary into a per-version cache with an atomic rename:

   | OS | Cache directory |
   | --- | --- |
   | Windows | `%LOCALAPPDATA%\scientific-process\<version>\` |
   | Linux, macOS | `$XDG_CACHE_HOME/scientific-process/<version>/`, else `~/.cache/scientific-process/<version>/` |

   A lock file makes concurrent first runs (two MCP clients starting at once)
   download only once. The holder refreshes the lock every 30 seconds; a lock
   that names a dead process, or has not been refreshed for 2 minutes, is
   taken over, and the new holder removes any partial download left behind.
   An interrupted download removes its lock and partial files before exiting.
   Later runs start the cached binary with no network access.

   The binary's SHA-256 is stored in `.verified` next to it and checked on
   every start. On Linux and macOS the version directory is created private
   (mode 700), and a cached binary is refused if that directory belongs to
   another user or is writable by group or others. Delete the directory to
   download the binary again.
6. Starts the binary with stdin, stdout and stderr inherited, and forwards its
   exit code and the signals SIGINT, SIGTERM and (not on Windows) SIGHUP.

The launcher never writes to stdout, because stdout is the MCP channel.
Progress and errors go to stderr.

The package has no runtime dependencies. It uses only Node built-ins plus the
system `tar` and, on Windows, PowerShell. Node 18 or later is required.

## Install-time pre-fetch

A `postinstall` script downloads the binary during `npm install`, so the first
MCP start does not wait for it. It never fails the install: errors become a
warning, and the launcher downloads again on first run. It is skipped when
scripts are disabled (`--ignore-scripts`), when `CI` is set, when
`SCIENTIFIC_PROCESS_BINARY` is set, or when `SCIENTIFIC_PROCESS_SKIP_DOWNLOAD=1`.

## Environment variables

| Variable | Effect |
| --- | --- |
| `SCIENTIFIC_PROCESS_BINARY` | Path to a `scientific-process` executable to run instead of downloading one. It must be a native executable (`scientific-process.exe` on Windows); `.cmd` and `.bat` files are refused. |
| `SCIENTIFIC_PROCESS_DOWNLOAD_BASE` | Release base URL for a mirror, in place of `https://github.com/praxec/scientific-process/releases`. Files are fetched from `<base>/download/v<version>/`. |
| `SCIENTIFIC_PROCESS_CACHE_DIR` | Cache root in place of the default above. |
| `SCIENTIFIC_PROCESS_SKIP_DOWNLOAD=1` | Skip the install-time pre-fetch. |
| `PRAXEC_ALLOW_INSECURE=1` | Allow a non-HTTPS download base and redirects. This is for tests and local mirrors only, and has the same meaning as in `install.sh` and `install.ps1`. |

## When the download fails

The error on stderr names the cause and the remedy. If you cannot reach GitHub,
install the binary yourself with `install.sh`/`install.ps1` or from the
[releases page](https://github.com/praxec/scientific-process/releases), then set
`SCIENTIFIC_PROCESS_BINARY` to its path, or put the binary in your MCP client
configuration directly.

## Development

```sh
node --test "test/*.test.mjs"   # from npm/; offline, uses a local fixture server
npm pack --dry-run              # check the published file list
```

The tests serve fixture releases over plain HTTP on `127.0.0.1` and set
`PRAXEC_ALLOW_INSECURE=1`, the same override the installer tests use. The
launcher has no test-only switches.

## License

Apache-2.0
