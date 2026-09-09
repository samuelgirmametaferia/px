# The px upstream fallback registry

A deterministic, sharded index of software that isn't reliably available in
normal package managers and is officially installed through installer
scripts, release binaries, or custom upstream commands. Normal resolution
(apt/pacman/dnf/zypper → cargo/npm/pipx/go/gem → curated registry →
crates.io identity) runs FIRST; this registry is the verified fallback.

## Design

```
query → normalize → alias_key → alias shard (4096) → app_key → app shard → record
```

- `app_key   = BLAKE3("px-app-v1\0"   + canonical_id)`  (canonical: `github:owner/repo`)
- `alias_key = BLAKE3("px-alias-v1\0" + normalized_alias)`
- `shard     = first 12 bits of the key` → 4096 shards per index
- shards: CBOR arrays sorted by key, zstd-compressed, binary-searched
- root manifest (`root.cbor`) pins every shard's BLAKE3 + sha256 + URL

One uncached lookup = exactly two shard fetches. Shards are cached and
re-verified against the root on every load — a corrupted or tampered shard
fails verification and is never used.

## Scale (measured, not projected)

Built and verified with 1,000,000 synthetic applications:

- build: 48s, streaming (memory bounded by one shard, ~244 records)
- 276MB total (1.2GB JSONL in), 4096+4096 shards
- warm lookup: ~16ms including process startup
- corrupt shard (1 flipped byte): rejected with verification error
- alias forms ("Synth App 0000123" → `synth-app-0000123`) all resolve

## Trust model

- **Identity confidence dominates.** 100 curated, 95 official README
  installer, 90 docs, 85 canonical-repo + strong evidence, 70 probable.
  Below 70: never silently installed. Stars are only ever tie-breakers.
- **Installer content is pinned.** A registry record stores the sha256 of
  the exact installer the validator tested. If the live URL serves
  different bytes, px STOPS the install — changed installer content is
  treated as a serious signal, not an inconvenience.
- **Validation receipts are facts, not safety claims.** The validator runs
  the installer in a disposable bwrap sandbox (system read-only, temp HOME,
  no network, resource + time limits), verifies the expected binary exists,
  is executable, and answers `--version`, then records installer sha256,
  binary sha256, and the test list. Attestation happens in CI
  (GitHub artifact attestations).
- **Dead records are tombstones.** A dead project's identity is preserved
  so a namesake can't hijack resolution.

## Telemetry (opt-in endpoint)

Failures (404, TLS, hash mismatch, missing installer) enqueue a scrubbed
report: record id, method id, registry version, error class, HTTP status,
a BLAKE3 of the URL, and an hour-precision timestamp. No URLs, no queries,
no user data. Reports are batched and only sent when an endpoint is
configured (`telemetry_endpoint` in ~/.config/px/config.toml). The
Cloudflare Worker + D1 schema live in `registry-infra/`.

## Publishing

`px-registry build --input records.jsonl --out dir` streams records into
4096+4096 buckets, sorts each shard independently, CBOR+zstd encodes with
a round-trip check, and writes the root manifest. Publishing = upload
changed shards as immutable GitHub Release assets + atomically replace the
attested root. Old clients keep using the previous snapshot.

## Infra layout

- `.github/workflows/registry-build.yml` — scheduled discovery + validate + build + publish
- `.github/workflows/registry-repair.yml` — failure reports → repair/quarantine state machine
- `registry-infra/validator.sh` — the disposable-sandbox validation job
- `registry-infra/worker.js` + `schema.sql` — Cloudflare Worker + D1
- `registry-seed.jsonl` — the hand-curated seed records (installer hashes pinned)
- `src/bin/px-registry.rs` — the builder (`synth`, `build`, `check`)
