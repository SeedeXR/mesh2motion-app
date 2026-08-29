# Local services

## SonarQube

Quality gate for the TypeScript frontend (`memory/test.md` §7).

```bash
docker compose -f docker/sonarqube.yml up -d
# wait for status UP
until curl -sf http://localhost:9000/api/system/status | grep -q '"status":"UP"'; do sleep 5; done
```

First run only: log in at http://localhost:9000 as `admin`/`admin`, change the
password when prompted, then create a token under
*My Account → Security → Generate Token*.

```bash
sonar-scanner -Dsonar.host.url=http://localhost:9000 -Dsonar.token=<your-token>
```

**Never commit the token.** It is a local credential; keep it in your shell
environment or a gitignored file.

Stop the server when you are done — it holds around 2 GB:

```bash
docker compose -f docker/sonarqube.yml down
```

### Scope

Community Edition ships no Rust analyser, so Rust quality is gated by
`cargo clippy -D warnings` in CI instead. Sonar covers the TypeScript frontend.
`legacy/` is excluded: it is a frozen reference implementation kept for A/B
comparison, not code under active development.
