# MemGuard MCP — Release Guide

> **Target**: `memguard-mcp` repository (Rust MCP server)
> **Prerequisites**: `gh` CLI authenticated, push access to `liuhengyuan666/memguard-mcp`, valid `NPM_TOKEN` secret in GitHub repo settings

---

## Overview

Release is fully automated via GitHub Actions (`.github/workflows/release.yml`). Pushing a `v*` tag triggers:

```
push tag → build 5 platforms → upload release assets → npm publish
```

The npm package (`@henry_lhy/memguard-mcp`) is a thin wrapper (`cli.js` + `install.js`) that downloads the correct platform binary from GitHub Releases. **Binary changes are distributed via Release assets, not inside the npm tarball.**

---

## Step-by-Step

### 0. Pre-release hygiene

Before starting, verify your working state is clean:

```powershell
git status              # must be clean — no uncommitted changes
git branch              # confirm on master
git pull origin master  # pull latest
git fetch --tags        # see existing tags to avoid version collision
```

### 1. Bump the version

Three files must agree. Use `Select-String` to confirm current state:

```powershell
Select-String -Path Cargo.toml,src/mcp/server.rs,npm/package.json -Pattern '"version"'
```

| File | Field | Example |
|------|-------|---------|
| `Cargo.toml` | `version` | `0.2.1` |
| `src/mcp/server.rs` | `serverInfo.version` | `"0.2.1"` |
| `npm/package.json` | `version` | `0.2.1` |

> `architecture.md` also contains a version string (`0.1.0 (core)`) — this is a document metadata version, **not** tied to the binary release. Do not bump it.

**If adding a new target platform** to the workflow matrix, also update the `PLATFORM_MAP` in `npm/cli.js` and `TARGET_MAP` in `npm/install.js`.

### 2. Make code changes

Implement features / fixes. Update tests. Verify:

```powershell
cargo test        # all tests must pass
cargo build       # no new warnings
cargo clippy      # no new lints (optional but recommended)
```

> Running `cargo build` regenerates `Cargo.lock`. Include it in the commit (Step 4).

### 3. Update documentation

Check and update if the changes affect them:

| Document | When to update |
|----------|---------------|
| `README.md` | Tool spec changes, new capabilities |
| `npm/README.md` | **Must** stay in sync with root `README.md` |
| `architecture.md` | Architecture-level changes (data model, module responsibilities) |
| `memguard.md` | New SOP-level patterns (rare) |
| `RELEASE.md` | If the release process itself changed |

### 4. Commit

Single, well-scoped commit with conventional commit message. Include `Cargo.lock`:

```bash
git add src/ Cargo.toml Cargo.lock npm/package.json
git add README.md npm/README.md architecture.md   # if docs changed
git commit -m "feat: <short description>

- bullet point of key change
- bullet point of another key change"
```

### 5. Version confirmation grep (mandatory)

Before tagging, confirm the new version string appears exactly once in each of the three files and they all match:

```powershell
Select-String -Path Cargo.toml,src/mcp/server.rs,npm/package.json -Pattern '"0\.\d+\.\d+"'
```

All three should show the same `"0.X.Y"` value and nothing else.

### 6. Tag and push

```bash
git tag -a v<version> -m "v<version>: <one-line summary>"
git push origin master --tags
```

**重要**: 如果该 tag 之前已存在（例如上个版本号被重复使用），必须先删除旧 tag 再重建：

```bash
git tag -d v<version>
git push origin :refs/tags/v<version>
git tag -a v<version> -m "v<version>: <one-line summary>"
git push origin v<version>
```

### 7. Wait for GitHub Actions

Push 后自动触发。去 [Actions](https://github.com/liuhengyuan666/memguard-mcp/actions) 监控：

| Job | What it does |
|-----|-------------|
| `build (windows-x64)` | Compile + test + zip → attach to Release |
| `build (darwin-arm64)` | Compile + test + tar.gz → attach to Release |
| `build (darwin-x64)` | Compile + test + tar.gz → attach to Release |
| `build (linux-x64)` | Compile + test + tar.gz → attach to Release |
| `build (linux-arm64)` | Compile (cross) + tar.gz → attach to Release |
| `publish-npm` | `cd npm && npm publish` (runs only after **ALL** builds pass) |

> The first successful build job creates the GitHub Release. Subsequent jobs attach to the same release. The `publish-npm` job waits for the entire matrix — if any build fails, npm will not be published.

### 8. Workflow failure recovery

If a matrix job fails, you have a **partial release** — some assets uploaded, others missing. Do not leave it in this state:

```bash
# 1. Delete the partial GitHub Release
gh release delete v<version> --repo liuhengyuan666/memguard-mcp --yes

# 2. Delete remote tag
git push origin :refs/tags/v<version>

# 3. Fix the issue (code bug, missing dependency, etc.)

# 4. Re-tag (local tag still exists or recreate it)
git tag -d v<version>
git tag -a v<version> -m "v<version>: <summary>"
git push origin v<version>
```

If the issue is **only** with `publish-npm` (builds all green but npm publish fails), check:

| Symptom | Likely cause | Action |
|---------|-------------|--------|
| "cannot publish over previously published version" | Previous workflow already published this version to npm | **Safe to ignore.** The npm wrapper hasn't changed — the binary Release is what matters. The red badge is cosmetic. |
| "402 Payment Required" or "401 Unauthorized" | `NPM_TOKEN` expired or missing | Update the secret in repo Settings → Secrets → Actions, then re-run the `publish-npm` job |
| npm wrapper (`cli.js`/`install.js`) actually changed | Code change needs a new npm version | Bump to next patch version (e.g. `0.2.1` → `0.2.2`), redo from Step 1 |

### 9. Verify

| Check | How |
|-------|-----|
| Release page | https://github.com/liuhengyuan666/memguard-mcp/releases → confirm 5 assets |
| npm registry | https://www.npmjs.com/package/@henry_lhy/memguard-mcp → confirm version |
| Integration test | Install in a test project, verify `runtime_bootstrap` output |

### 10. Companion repo (memguard Skill)

**When needed**: If the release changes SOP-observable behavior — new bootstrap fields, new event constraints, modified phase semantics, new error types — update the companion `memguard` repo.

**When optional**: Pure internal changes (refactors, performance, bug fixes that don't change agent-facing behavior).

```bash
cd ../memguard                   # sibling directory to memguard-mcp
git pull origin main
git status                       # must be clean

# edit memguard/SKILL.md
git add memguard/SKILL.md
git commit -m "feat: <description>"
git tag -a v3.0.X -m "v3.0.X: <summary>"
git push origin main --tags

gh release create v3.0.X --repo liuhengyuan666/memguard \
    --title "v3.0.X — <summary>" \
    --notes-file release-notes.md
```

---

## Common Pitfalls

| Problem | Cause | Fix |
|---------|-------|-----|
| `npm publish` fails — "cannot publish over previously published version" | Previous workflow run already published this npm version | If `cli.js`/`install.js` didn't change: **ignore the red badge** — binary Release is what matters. If they did change: bump patch version + re-tag. |
| macOS x86_64 build stuck in queue | `macos-13` runner capacity on GitHub Actions | Wait. If >30 min, it's a capacity issue. The Release is usable with the 4 other platforms; re-run the failed job later from the Actions UI. |
| Tag already exists locally | Previous release used same version number | See Step 6 re-tag instructions |
| `serverInfo.version` mismatch | Forgot to update `src/mcp/server.rs` | Run the version confirmation grep from Step 5 |
| Partial release (some builds failed) | Code bug or transient CI issue | Follow Step 8 recovery procedure |
| `Cargo.lock` not committed | Forgot `git add Cargo.lock` after build | Add to Step 4 commit. Running `cargo build` regenerates it even without dependency changes |

---

## Release Checklist

```
[ ] git status clean, on master, pulled latest
[ ] Cargo.toml version bumped
[ ] src/mcp/server.rs serverInfo.version bumped
[ ] npm/package.json version bumped
[ ] version grep: all three files agree on same "0.X.Y"
[ ] cargo test — all pass
[ ] cargo build — no new warnings
[ ] Cargo.lock included in commit
[ ] Docs updated if needed (README, npm/README, architecture.md)
[ ] Commit message follows conventional format
[ ] Tag created and pushed
[ ] GitHub Actions: all 5 builds green (or known macOS runner queue)
[ ] GitHub Release shows 5 platform assets
[ ] npm registry shows new version
[ ] Companion memguard SKILL.md updated (if needed)
```
