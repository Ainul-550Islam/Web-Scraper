# Web-Scraper — কোড রিভিউ, সংশোধন ও যাচাই রিপোর্ট

**রিপোজিটরি:** https://github.com/Ainul-550Islam/Web-Scraper
**Patch-এর বেস commit:** `856717b46c746d73041c11364da20119733e96c5`
**রিভিউ/পরীক্ষার তারিখ:** ৮ অক্টোবর ২০২৬
**পরিধি:** Rust crawler, scheduler, CLI ও admin API; Redis/PostgreSQL/SQLite integration; storage/export/download; React/TypeScript console; lockfile ও security audit; CI, Docker/Compose/Kubernetes/systemd উদাহরণ; docs ও tests।

## সারাংশ

রিপোজিটরির tracked source, নতুন/untracked source, configuration, tests, deployment files এবং documentation পর্যালোচনা করে পাওয়া সমস্যাগুলো সংশোধন করা হয়েছে। পূর্ণ source project ZIP, বেস commit থেকে full patch এবং SHA-256 manifest তৈরি করা হয়েছে। Rust-এর default ও all-feature test suites, feature-isolated checks, Clippy, formatting, npm audit, web tests/build, buyer-flow E2E, release binary smoke এবং Docker image/healthcheck smoke চালানো হয়েছে।

এই ফলাফল পরীক্ষিত repository snapshot-এর জন্য; এটি সব সম্ভাব্য bug শেষ হওয়ার বা production deployment যাচাইয়ের নিশ্চয়তা নয়। যে service/platform বাস্তবে চালানো হয়নি, সেটিকে verified বলা হয়নি—সীমাবদ্ধতা নিচে আছে।

## উল্লেখযোগ্য সমস্যা ও সংশোধন

| ক্ষেত্র | পাওয়া সমস্যা / করা সংশোধন |
|---|---|
| হারানো test source | `src/dist/tests.rs` working tree-তে অনুপস্থিত ছিল। Git baseline থেকে ফিরিয়ে এনে distributed target/URL validation এবং DLQ redaction/retryability regression test যোগ করা হয়েছে। |
| HTTP response resource bound | `max_response_bytes`-এর নির্দিষ্ট সর্বোচ্চ সীমা ছিল না। এখন প্রতি response সর্বোচ্চ 64 MiB; `cap × effective workers` সর্বোচ্চ 1 GiB। Effective worker count engine-এর মতো 1–64 clamp করা হয়, overflow-এ saturating arithmetic ব্যবহৃত হয়। 256 MiB-এর বেশি nominal response-buffer reservation production audit-এ soft warning। Config validation, tests, configuration docs, README, CHANGELOG ও SECURITY docs একসঙ্গে আপডেট করা হয়েছে। এটি response body-র সীমা; সম্পূর্ণ process RSS-এর নিশ্চয়তা নয়। |
| Target page budget | Per-target `max_pages = 0` গ্রহণ করে অনির্দিষ্ট/শূন্য page budget তৈরি হতে পারত। এখন startup validation তা প্রত্যাখ্যান করে; global ও per-target zero-budget regression test/docs যোগ। |
| Queue/requeue test | Requeue regression test-এ `Url` serialization-এর প্রত্যাশিত path ভুলভাবে trailing slash-সহ লেখা ছিল। Canonical serialization অনুযায়ী test ঠিক করা হয়েছে। Drain lifecycle test এখন terminal event অথবা `job_submitted`-এর `requeued: true` transition—দুটোকেই গণনা করে, যাতে cancellation-এ pending job-কে হারিয়ে গেছে বলে ভুল না দেখায়। Engine API-র stop behavior-ও pending/checkpoint semantics অনুযায়ী ব্যাখ্যা করা হয়েছে। |
| Distributed DLQ retry target | DLQ entry-র `target`-এ বর্তমান configured target ID থাকলে সেটি resolve হয়; পুরোনো queue-prefix রাখা entry-তে approved URL scope থেকে target fallback হয়। Replay-তে approved-scope check বজায় আছে। Credential-bearing URL/reason redaction হলে entry `retryable:false`; পরিচ্ছন্ন entry retryable থাকে। |
| Docker healthcheck address | Healthcheck আগে `http://127.0.0.1:9678` দিত, অথচ `harvest health --url`/`AdminClient`-এর interface `HOST:PORT` নেয়। Scheme-সহ ঠিকানাটি ভুলভাবে parse হয়ে health probe ব্যর্থ হতো। Dockerfile-এ `127.0.0.1:9678` করা হয়েছে এবং container-এর ভিতরে CLI ও image healthcheck দিয়ে healthy status যাচাই করা হয়েছে। |
| Run-output snapshot cancellation | Snapshot copy-তে cancellation binding, capped/no-follow create path, loop-level cancellation checks এবং cancellation/copy/register error-এ partial snapshot cleanup যোগ করা হয়েছে। |
| Control-plane request limit | Server-এর 65,536-byte command-body সীমার সঙ্গে web command composer-এর serialized UTF-8 body validation মিলিয়ে দেওয়া হয়েছে; বড় inline AI body-র UI regression test যোগ। |
| Filesystem/state safety | Shared filesystem helpers-এ bounded no-follow reads, descriptor-rooted Unix traversal, private atomic state-file publication, no-follow delete এবং hard-link/symlink refusal শক্ত করা হয়েছে। Checkpoint, export progress, state, history/events, sink, download, source inputs ও run snapshots-এর সঙ্গে সংশ্লিষ্ট regression coverage রয়েছে। Non-Unix fallback race-free বলে দাবি করা হচ্ছে না। |
| Export/import bounds | Export source/record/column/member limits, no-follow source read, atomic manifest/progress handling এবং ZIP path/size constraints পরীক্ষা করা হয়েছে। Input/output overlap এবং unsafe file adoption-এর ক্ষেত্রে fail-closed আচরণ রাখা হয়েছে। |
| Web console safety/UX | Route boundary/error handling, command/model validation, redaction, polling/cancellation, generated API flow ও job state-এর regression coverage বাড়ানো হয়েছে। `PageErrorBoundary`, shared command/state models এবং নতুন route-safety tests যোগ হয়েছে। |
| Dependencies | `source-map-js`-এর vulnerable transitive version lockfile-এ নিরাপদ version-এ উন্নীত করা হয়েছে; Rust lockfile-ও audit-এ পরীক্ষা করা হয়েছে। |
| Docs/deployment examples | Response limits, DLQ/replay, state safety, admin/health command syntax, secret handling, examples ও deployment documentation সামঞ্জস্য করা হয়েছে। |

## পরীক্ষার ফল

| পরীক্ষা | ফল |
|---|---|
| Rust formatting | `cargo fmt --all -- --check` — সফল। |
| Rust Clippy | `cargo clippy --locked --all-targets --all-features -- -D warnings` — সফল; warning নেই। |
| Rust feature matrix | `cargo check --locked --all-targets` এবং আলাদাভাবে `--features sqlite`, `--features postgres`, `--features redis` — চারটিই সফল। |
| Rust default suite | `cargo test --locked --all-targets -j1 --quiet` — **536 unit tests সফল**, সঙ্গে সব default integration targets সফল; bench target-ও সম্পন্ন। |
| Rust all-feature suite | `cargo test --locked --all-features -j1` — **553 unit tests সফল** এবং সব সক্রিয় integration/doc-test targets সফল। Live external Redis/PostgreSQL service-gated case এই পরিবেশে opt-in করা হয়নি। |
| Rust advisory audit | `cargo audit --deny warnings` — 304 crate dependency scan; কোনো advisory/warning নেই। |
| Rust release binary | `cargo build --locked --release --all-features -j1` — সফল। `harvest --version`, `harvest validate --config config.example.json` এবং test-only env token দিয়ে `ops/harvest/config.example.json` validation সফল; example config-এর version migration warning প্রত্যাশিত। |
| Web dependency/security | Node `v24.21.0`; `npm ci` সফল; `npm audit --audit-level=low` — **0 vulnerability**। |
| Web checks | `npm run typecheck`, `npm run lint`, `npm test`, `npm run build` — সব সফল। Vitest: **126/126 files, 373/373 tests**। Negative boundary tests jsdom-এ ইচ্ছাকৃত error log করতে পারে, কিন্তু Vitest exit code 0। |
| Buyer-flow E2E | Built console, local demo site, release `harvest serve`, export/verify ও graceful stop—`npm run e2e:buyer` **18/18 checks সফল**। |
| Docker | Docker 26.1.5 local VFS engine-এ `docker build --network=host --pull=false -t harvest-fixed:verify .` সফল। Built image `--version` smoke এবং container healthcheck—`health=healthy`—যাচাই করা হয়েছে। `--network=host` এই sandbox-এর bridge DNS সমস্যার workaround; production network/deployment পরীক্ষা নয়। |
| Package/config syntax | `cargo package --locked --allow-dirty --list` — 879 package entries; shell `bash -n`, 37 JSON file ও 10 YAML file syntax checks — সফল। |
| Patch hygiene | Final artifact তৈরির আগে whitespace/diff checks চালানো হয়েছে; patch-এ সম্পূর্ণ নতুন file content অন্তর্ভুক্ত। |

**পরীক্ষা-সংক্রান্ত সীমাবদ্ধতা:** প্রথমে Rust all-feature test compilation ডিফল্ট debug info-সহ চালালে sandbox memory limit-এ rustc `SIGKILL` হয়েছিল। Test profile debug info বন্ধ করে `-j1`-এ একই full suite পুনরায় চালানো হয়েছে; সব পরীক্ষা পাস করেছে, কোনো test বাদ দেওয়া হয়নি।

## যা এই পরিবেশে যাচাই করা হয়নি

1. Live Redis/PostgreSQL service integration, PostgreSQL trusted-CA/hostname TLS integration এবং production database/queue credentials দিয়ে connection যাচাই করা হয়নি। Opt-in `HARVEST_TEST_REDIS`/`HARVEST_TEST_PG` পরিবেশ দেওয়া হয়নি; gated test pass-কে live service pass হিসেবে ধরা হয়নি।
2. Windows/macOS build/runtime, symlink/reparse-point behavior এবং non-Unix filesystem race পরীক্ষা করা হয়নি। Unix-specific tests এই Linux runner-এ চালানো হয়েছে।
3. Kubernetes cluster, remote GitHub Actions, production DNS/private CA/firewall, real source sites, proxy fleet, persistent volume quota, backup/restore বা rollout/rollback পরীক্ষা করা হয়নি। Docker image smoke স্থানীয় sandbox-এ সীমিত।
4. Chromium/বাস্তব browser automation চালানো হয়নি। Web tests jsdom/Vitest; buyer-flow E2E local HTTP/API/CLI path যাচাই করে, বাস্তব end-user browser নয়।
5. `max_response_bytes` ও concurrency product response-body budget; DOM/parser এবং অন্যান্য runtime allocation-সহ মোট RSS cap নয়। Low-memory production container-এ উপযুক্ত cap/concurrency নির্বাচন করুন।
6. URL userinfo request-এর জন্য configuration-এ থাকতে পারে; log/DLQ observations-এ তা redact করা হয়। Redis-এ distributed work item সংরক্ষণ হলে deployment-এর TLS/auth/access policy নিজ পরিবেশে যাচাই করতে হবে। SQLite VFS sidecar-এর concurrent filesystem race সম্পূর্ণ বন্ধ বলে দাবি করা হচ্ছে না।
7. কোনো Git commit/push করা হয়নি এবং কোনো production credential deliverable-এ রাখা হয়নি।

## পুনরায় চালানোর নির্দেশনা

Rust toolchain `rust-toolchain.toml`-এ pin করা সংস্করণে:

```bash
cargo fmt --all -- --check
cargo check --locked --all-targets
cargo check --locked --all-targets --features sqlite
cargo check --locked --all-targets --features postgres
cargo check --locked --all-targets --features redis
cargo clippy --locked --all-targets --all-features -- -D warnings
CARGO_BUILD_JOBS=1 CARGO_PROFILE_TEST_DEBUG=0 cargo test --locked --all-targets -j1
CARGO_BUILD_JOBS=1 CARGO_PROFILE_TEST_DEBUG=0 cargo test --locked --all-features -j1
cargo audit --deny warnings
cargo build --locked --release --all-features -j1
```

Web console:

```bash
cd web
npm ci
npm audit --audit-level=low
npm run typecheck
npm run lint
npm test
npm run build
HARVEST_BIN="../target/release/harvest" npm run e2e:buyer
```

Docker image ও container health smoke:

```bash
docker build --network=host --pull=false -t harvest-fixed:verify .
docker run --rm harvest-fixed:verify --version
```

Live Redis/PostgreSQL integration চাইলে আগে test-only service চালিয়ে প্রয়োজনীয় `HARVEST_TEST_REDIS`, `HARVEST_TEST_REDIS_URL`, `HARVEST_TEST_PG` এবং `HARVEST_PG_DSN` দিন; production secret ব্যবহার করবেন না।

## Deliverables

- `Web-Scraper-fixed.zip` — সম্পূর্ণ project source ও configuration; build outputs, dependency caches, local secrets এবং পরীক্ষার অস্থায়ী config বাদ।
- `Web-Scraper-fixes.patch` — বেস commit `856717b46c746d73041c11364da20119733e96c5` থেকে tracked ও নতুন source/docs-সহ সম্পূর্ণ full patch।
- `Web-Scraper-review-bn.md` — এই পূর্ণ বাংলা রিপোর্টের আলাদা কপি।
- `SHA256SUMS.txt` — ZIP, patch ও রিপোর্টের যাচাইযোগ্য SHA-256 checksum।
