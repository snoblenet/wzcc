---
name: publish
description: Bump version, tag, and push to trigger the automated release pipeline. Triggers on 'publish', 'release', 'bump version'. This skill handles the full version bump workflow and knows the CI/CD pipeline to avoid manual mistakes.
---

# Publish

## CI/CD Pipeline

Two-stage GitHub Actions pipeline triggered by git tags:

1. **`release.yml`** (`v*` tag push) — verify tag matches `Cargo.toml`, build `aarch64-apple-darwin` binary, create GitHub Release
2. **`publish.yml`** (GitHub Release published) — verify version, run `cargo publish --locked`

**Flow**: `git push tag` → release.yml (build + GitHub Release) → publish.yml (crates.io)

**CRITICAL: Never run `cargo publish` manually.** The pipeline handles it. Manual publish causes duplicate version errors in CI.

## Version Bump Workflow

### Step 1: Pre-flight checks

```bash
git branch --show-current   # Must be on main
git status                  # Must be clean
git fetch --tags --quiet
```

Confirm clean working directory on `main`. Show the latest tag and current `Cargo.toml` version.

### Step 2: Ask for the new version

Use AskUserQuestion. Show the current version and suggest semver options (patch, minor, major).

### Step 3: Update version

1. Use the Edit tool to update the `version` field in `Cargo.toml`.
2. Run `cargo check --quiet` to regenerate `Cargo.lock`.
3. Show `git diff` for user review.

### Step 4: Commit, tag, and push

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to <NEW_VERSION>"
git tag v<NEW_VERSION>
git push origin main
git push origin v<NEW_VERSION>
```

### Step 5: Verify

Inform the user the CI pipeline will build, release, and publish. Link to:
`https://github.com/K-dash/wzcc/actions`
