# Dev Database Isolation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `npm run dev` use a repo-local SQLite database and log directory by default, while leaving non-debug builds on `~/.timetracker`.

**Architecture:** Add a small Rust runtime-path helper that resolves database and log locations from the build mode instead of duplicating path logic inside the Tauri setup closure. Keep that helper pure enough to unit test both debug and non-debug behavior directly, then wire startup and docs to the new split.

**Tech Stack:** Rust, Tauri 2, SQLx, SQLite, Markdown docs, gitignore

---

### Task 1: Add a runtime-path helper with test coverage

**Files:**
- Create: `src-tauri/src/runtime_paths.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: `src-tauri/src/runtime_paths.rs`

- [ ] **Step 1: Write the failing test**

Create `src-tauri/src/runtime_paths.rs` with tests for both modes and an intentionally incomplete implementation that still points everything at the home directory:

```rust
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimePaths {
    pub db_path: PathBuf,
    pub log_dir: PathBuf,
}

pub fn resolve_runtime_paths(home_dir: &Path, _manifest_dir: &Path, _debug_build: bool) -> RuntimePaths {
    let base = home_dir.join(".timetracker");
    RuntimePaths {
        db_path: base.join("timetracker.db"),
        log_dir: base.join("logs"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_resolve_runtime_paths_uses_repo_local_paths_in_debug_builds() {
        let home_dir = Path::new("/Users/example");
        let manifest_dir = Path::new("/workspace/src-tauri");

        let paths = resolve_runtime_paths(home_dir, manifest_dir, true);

        assert_eq!(
            paths.db_path,
            Path::new("/workspace/.timetracker-dev/timetracker.db")
        );
        assert_eq!(paths.log_dir, Path::new("/workspace/.timetracker-dev/logs"));
    }

    #[test]
    fn test_resolve_runtime_paths_uses_home_directory_in_non_debug_builds() {
        let home_dir = Path::new("/Users/example");
        let manifest_dir = Path::new("/workspace/src-tauri");

        let paths = resolve_runtime_paths(home_dir, manifest_dir, false);

        assert_eq!(
            paths.db_path,
            Path::new("/Users/example/.timetracker/timetracker.db")
        );
        assert_eq!(paths.log_dir, Path::new("/Users/example/.timetracker/logs"));
    }
}
```

Expose the module from `src-tauri/src/lib.rs` near the other `pub mod` declarations:

```rust
pub mod runtime_paths;
```

- [ ] **Step 2: Run the targeted test to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml runtime_paths`

Expected: FAIL because the debug-mode test still receives `~/.timetracker` paths instead of repo-local `.timetracker-dev` paths.

- [ ] **Step 3: Write the minimal implementation**

Replace the stub implementation in `src-tauri/src/runtime_paths.rs` with the real branch logic and a convenience wrapper for production code:

```rust
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimePaths {
    pub db_path: PathBuf,
    pub log_dir: PathBuf,
}

fn repo_root_from_manifest_dir(manifest_dir: &Path) -> PathBuf {
    manifest_dir.parent().unwrap_or(manifest_dir).to_path_buf()
}

pub fn resolve_runtime_paths(home_dir: &Path, manifest_dir: &Path, debug_build: bool) -> RuntimePaths {
    let base = if debug_build {
        repo_root_from_manifest_dir(manifest_dir).join(".timetracker-dev")
    } else {
        home_dir.join(".timetracker")
    };

    RuntimePaths {
        db_path: base.join("timetracker.db"),
        log_dir: base.join("logs"),
    }
}

pub fn runtime_paths(home_dir: &Path) -> RuntimePaths {
    resolve_runtime_paths(
        home_dir,
        Path::new(env!("CARGO_MANIFEST_DIR")),
        cfg!(debug_assertions),
    )
}
```

- [ ] **Step 4: Run the targeted test to verify it passes**

Run: `cargo test --manifest-path src-tauri/Cargo.toml runtime_paths`

Expected: PASS with both `test_resolve_runtime_paths_*` cases green.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/runtime_paths.rs src-tauri/src/lib.rs
git commit -m "Add runtime path resolver for dev data isolation"
```

### Task 2: Wire startup to the shared runtime paths and ignore dev artifacts

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Modify: `.gitignore`
- Test: `src-tauri/src/lib.rs`

- [ ] **Step 1: Write the failing integration-oriented change**

Update `src-tauri/src/lib.rs` so the setup block uses the helper, but keep the startup log message and managed state expectations intact:

```rust
.setup(|app| {
    let home = app.path().home_dir()?;
    let runtime_paths = runtime_paths::runtime_paths(&home);

    let guard = logger::init_logger(&runtime_paths.log_dir);
    app.manage(guard);

    let pool = tauri::async_runtime::block_on(db::init_db(&runtime_paths.db_path))?;
    app.manage(pool);

    tracing::info!(
        "TimeTracker started. DB: {:?}, logs: {:?}",
        runtime_paths.db_path,
        runtime_paths.log_dir
    );

    Ok(())
})
```

Add the repo-local dev runtime directory to `.gitignore`:

```gitignore
.timetracker-dev/
```

- [ ] **Step 2: Run the backend tests to verify the integration still holds**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`

Expected: PASS with the existing database, command, and seed tests still green after startup path resolution changed.

- [ ] **Step 3: If any startup or borrow errors appear, make the minimal fix**

Keep the final `lib.rs` shape close to the existing code:

```rust
let home = app.path().home_dir()?;
let runtime_paths = runtime_paths::runtime_paths(&home);
let guard = logger::init_logger(&runtime_paths.log_dir);
app.manage(guard);

let pool = tauri::async_runtime::block_on(db::init_db(&runtime_paths.db_path))?;
app.manage(pool);
```

Do not move unrelated command registration or change any database initialization semantics beyond the resolved path.

- [ ] **Step 4: Re-run the backend tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`

Expected: PASS again, confirming the shared runtime-path helper is now active in startup.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/lib.rs .gitignore
git commit -m "Use repo-local dev runtime data in debug builds"
```

### Task 3: Align the docs with the new dev-vs-release split

**Files:**
- Modify: `README.md`
- Modify: `docs/getting-started.md`
- Modify: `docs/configuration.md`
- Modify: `docs/architecture.md`
- Modify: `docs/data-and-backup.md`
- Modify: `docs/development-notes.md`

- [ ] **Step 1: Update the contributor-facing docs**

Make the doc changes explicit rather than implying a single runtime location for every mode.

Add this note to `README.md` after the quick-start command block:

```md
During `npm run dev`, TimeTracker stores runtime data in `.timetracker-dev/` inside the repository so development work stays isolated from the live `~/.timetracker` directory.
```

Add this note to `docs/getting-started.md` under "Run the app":

```md
Development runs use repo-local runtime data under `.timetracker-dev/`. Release builds continue to use `~/.timetracker/`.
```

Replace the opening runtime-location section in `docs/configuration.md` with:

````md
## Runtime locations

Development (`npm run dev`) writes local runtime data under the repository:

```text
.timetracker-dev/timetracker.db
.timetracker-dev/logs/
```

Non-debug builds write runtime data under the user's home directory:

```text
~/.timetracker/timetracker.db
~/.timetracker/logs/
```
````

Replace the storage/logging statements in `docs/architecture.md` with wording like:

```md
Development runs store SQLite data in `.timetracker-dev/timetracker.db` inside the repository. Non-debug builds use `~/.timetracker/timetracker.db`.

The backend initializes daily-rotating logs in `.timetracker-dev/logs/` during development and `~/.timetracker/logs/` in non-debug builds.
```

Replace the database-location sentence in `docs/data-and-backup.md` with:

```md
During development, the working database lives at `.timetracker-dev/timetracker.db`. Installed or other non-debug builds use `~/.timetracker/timetracker.db`.
```

Add this note to `docs/development-notes.md` near the development-only seed section:

```md
Debug builds also isolate runtime data in the repo-local `.timetracker-dev/` directory, which keeps seed workflows away from the live home-directory database.
```

- [ ] **Step 2: Check that the wording is consistent**

Run:

```bash
rg -n "~/.timetracker|\\.timetracker-dev|npm run dev" README.md docs
```

Expected: hits show both locations only where the distinction is intentional, with no stale statement that says development always writes to `~/.timetracker`.

- [ ] **Step 3: Make any final wording corrections**

Prefer concise statements like these:

```md
- `npm run dev` uses `.timetracker-dev/` in the repo
- packaged or non-debug builds use `~/.timetracker/`
```

Do not add UI instructions or configuration knobs that do not exist.

- [ ] **Step 4: Commit**

```bash
git add README.md docs/getting-started.md docs/configuration.md docs/architecture.md docs/data-and-backup.md docs/development-notes.md
git commit -m "Document isolated dev runtime data"
```

### Task 4: Run the full verification pass

**Files:**
- Modify: none expected
- Verify: `src-tauri/src/runtime_paths.rs`
- Verify: `src-tauri/src/lib.rs`
- Verify: `README.md`
- Verify: `docs/getting-started.md`
- Verify: `docs/configuration.md`
- Verify: `docs/architecture.md`
- Verify: `docs/data-and-backup.md`
- Verify: `docs/development-notes.md`

- [ ] **Step 1: Run the frontend checks**

Run:

```bash
npm run check
npm run lint
```

Expected: both commands exit successfully without introducing frontend regressions.

- [ ] **Step 2: Run the Rust quality checks**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all -- -D warnings
cargo fmt --manifest-path src-tauri/Cargo.toml --check
```

Expected: all Rust tests pass, Clippy reports no warnings, and formatting is already correct.

- [ ] **Step 3: Confirm the working tree only contains the intended changes**

Run:

```bash
git --no-pager status --short
```

Expected: only the runtime-path helper, startup wiring, gitignore entry, docs updates, and plan/spec files related to this work appear.

- [ ] **Step 4: Commit the verification-complete state**

```bash
git add src-tauri/src/runtime_paths.rs src-tauri/src/lib.rs .gitignore README.md docs/getting-started.md docs/configuration.md docs/architecture.md docs/data-and-backup.md docs/development-notes.md
git commit -m "Finish dev database isolation"
```
