# Development notes

## Backend command pattern

The Rust backend exposes Tauri commands that return `Result<T, AppError>`. `AppError` is serialized as a display string, so frontend code receives user-visible error text instead of a custom error object graph.

This keeps command boundaries simple:

- frontend calls `invoke()`
- backend validates and queries
- success returns JSON data
- failure returns a serialized error string

## Shared test database fixture

Rust tests use a shared `test_db()` helper defined at crate root. It creates an in-memory SQLite pool and applies the schema before each test module uses it.

That pattern matters because it keeps backend tests:

- isolated from the real user database
- fast enough for unit-level command testing
- aligned with the production schema

## Development-only seed data

`src-tauri/src/dev_seed.rs` is compiled only in debug builds. The command is registered only when `debug_assertions` are enabled, which means:

- seed tooling is available during local development
- release builds do not expose the seed command

Debug builds also keep their runtime database and logs under repo-local `.timetracker-dev/`, which keeps seed workflows away from the live `~/.timetracker/` data used by non-debug builds.

The seed workflow populates sample customers, projects, issues, and time entries anchored to recent dates so reporting screens have realistic data to display.

Notes and folders are seeded separately: whenever the `notes` table is empty, the seed adds sample notes covering every live-preview element plus a `Projects / Archive` folder pair holding one note. This happens even if the customer seed is skipped because customers already exist, so older dev databases get notes too.

The Seed button (Settings › Dev) also adds 16 bilingual sample notes (100–200 words each) to a root `Samples` folder for trying semantic search: Swedish/English pairs on the same topic in different words (budget, release date, onboarding, invoice dispute, backups, accessibility, summer leave) plus two unrelated decoys. Titles that already exist are skipped, so it works on a dev database that already has notes and is safe to click again.

Reminders are seeded too, whenever the `reminders` table is empty (also when customers or notes already exist): six samples anchored to now, namely an overdue one (fired), one for in three hours linked to the "Weekly sync" note when that note exists, one tomorrow at 14:00, a weekday "Standup" at 09:00, a monthly "Pay rent" on the 25th, and a done one. The overdue row shows up in the Overdue group, the Today card, a toast, and the nav count straight away. Seeded times are relative to the moment you click Seed, so a row can already be overdue by the time you look.

## Semantic search tests

Unit tests never download or load the embedding model. Embedding sits behind the `Embedder` trait and tests use `FakeEmbedder` (deterministic vectors, a word-count token counter); `semantic::index::tests::ready_state` builds a ready `SemanticState` around one. Download tests run against a local fake source, not the network.

`src-tauri/src/semantic/real_model_tests.rs` holds `#[ignore]`d tests against the real model. They download nothing: enable semantic search in a debug build first (files land in `.timetracker-dev/models/`), or point `TIMETRACKER_MODEL_DIR` at a directory with `model_quantized.onnx` and `tokenizer.json`:

```bash
TIMETRACKER_MODEL_DIR=<dir> cargo test -p time-tracker real_model -- --ignored --nocapture --test-threads=1
```

`real_model_spike` also compares against the fp32 export when `model.onnx` is in the same directory, and prints the timing, int8-vs-fp32 overlap and the similarity-floor sweeps (fixed floor, then the relative rule's `MARGIN` and `ABSOLUTE_FLOOR` over topic and off-topic queries); rerun it when the model, chunker or prefixes change (see `similarity_floor` in `semantic/fusion.rs`). `real_model_sample_notes` checks the rule on the dev seed's 16 sample notes through hybrid search and prints every note's score, the median and the floor per query.

## Report and restore behavior worth knowing

- Weekly and monthly reports are computed in Rust rather than assembled entirely in the frontend
- Backup export/import also stays in Rust so file access and SQLite writes remain on the native side
- Backup/restore UI lives in the Settings page (Backup & Restore tab)

## Formatting and lint split

Frontend formatting and linting are handled by the Node toolchain, while Rust quality checks are handled by Cargo commands. `eslint.config.js` explicitly ignores generated directories and the Rust backend tree, so JavaScript linting expectations stop at the frontend boundary.
