# Build & release scripts (macOS)

Three scripts that turn a checkout into an installed app. Run them from anywhere;
each resolves the repo root itself.

| Script | Does |
|--------|------|
| `build.sh` | `tauri build` → `.app` + `.dmg` (builds the frontend first). Unsigned unless the signing env vars below are set. |
| `notarize.sh` | Checks Apple credentials are set, runs `build.sh` (Tauri signs + notarizes inline), then verifies the signature and stapled ticket. |
| `install.sh` | Mounts the built `.dmg` (or uses the `.app`) and copies `Mesh2Motion.app` into `/Applications`. |

Artifacts land in `target/release/bundle/{macos,dmg}/`.

## Local build (no Apple account)

```sh
scripts/build.sh        # .app + .dmg, unsigned
scripts/install.sh      # into /Applications
```

An unsigned build runs locally; other machines get a Gatekeeper warning.

## Signed & notarized release

Set a signing identity and one notarization method, then run `notarize.sh`.
Variable names are Tauri v2's — it reads them during `tauri build`.

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
scripts/notarize.sh
```

An app-specific password is created at <https://account.apple.com> → Sign-In and
Security → App-Specific Passwords. Signing requires a "Developer ID Application"
certificate from your Apple Developer account.
