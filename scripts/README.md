# Build & release scripts (macOS)

`install.sh` is the entry point — it builds and installs in one step. You do not
need to run the others by hand.

| Script | Does |
|--------|------|
| `install.sh` | **Start here.** Builds (signing + notarizing if Apple creds are set), then copies `Mesh2Motion.app` into `/Applications`. |
| `build.sh` | `tauri build` → `.app` + `.dmg`, collected into **`./output/`** (created if missing, git-ignored). Unsigned unless the signing env vars below are set. |
| `notarize.sh` | Checks Apple credentials, runs `build.sh`, then verifies the signature and stapled ticket. `install.sh` calls this automatically when the creds are present. |

Artifacts land in **`./output/`** (`Mesh2Motion.app` and `Mesh2Motion_<version>_<arch>.dmg`).

## Install locally (no Apple account)

```sh
scripts/install.sh
```

Builds unsigned and installs into `/Applications`. An unsigned build runs on the
machine that built it; other machines get a Gatekeeper warning. Pass build args
through, e.g. `scripts/install.sh --bundles app` for a faster app-only build.

## Signed & notarized release

Set a signing identity and one notarization method, then run `install.sh` (or
`notarize.sh` to build + verify without installing). The variable names are
Tauri v2's — it reads them during `tauri build`.

Signing identity (one of):

- `APPLE_SIGNING_IDENTITY` — e.g. `Developer ID Application: Name (TEAMID)` (local, cert in keychain)
- `APPLE_CERTIFICATE` + `APPLE_CERTIFICATE_PASSWORD` — base64 `.p12` + password (CI)

Notarization (one method):

- Apple ID — `APPLE_ID`, `APPLE_PASSWORD` (app-specific password), `APPLE_TEAM_ID`
- API key — `APPLE_API_ISSUER`, `APPLE_API_KEY`, `APPLE_API_KEY_PATH`

```sh
export APPLE_SIGNING_IDENTITY="Developer ID Application: Name (TEAMID)"
export APPLE_ID="you@example.com"
export APPLE_PASSWORD="abcd-efgh-ijkl-mnop"   # app-specific password
export APPLE_TEAM_ID="TEAMID"
scripts/install.sh
```

An app-specific password is created at <https://account.apple.com> → Sign-In and
Security → App-Specific Passwords. Signing requires a "Developer ID Application"
certificate from your Apple Developer account.
