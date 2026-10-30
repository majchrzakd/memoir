# Memoir — Storage format, version 1

This document is enough to decrypt a Memoir vault without the app (that is
what `memoir decrypt` in `crates/memoir-cli` does). Any incompatible change requires a new
format version, and readers for older versions must be kept.

Notation: `||` is concatenation, `u32le`/`u32be` are 4-byte little/big-endian
integers, `b64` is unpadded base64url (RFC 4648 §5), strings are UTF-8.

## Primitives

| Purpose | Algorithm |
|---|---|
| Authenticated encryption | XChaCha20-Poly1305 (24-byte nonce, 16-byte tag appended) |
| Passphrase hashing | Argon2id, version 0x13, 32-byte output |
| Key derivation | HKDF-SHA256, no salt |
| Attachment names | HMAC-SHA256 |
| Compression | zstd (single frame) |

## Key hierarchy

```
master key (MK)        32 random bytes, never stored unwrapped
object key             HKDF(MK, info = "memoir/v1/object")
blob key               HKDF(MK, info = "memoir/v1/blob")
blob-id key            HKDF(MK, info = "memoir/v1/blob-id")
local database key     HKDF(MK, info = "memoir/v1/local-db")
```

## `vault.json`

```json
{
  "format": "memoir-vault",
  "version": 1,
  "vault_id": "<b64 of 16 random bytes>",
  "key_epoch": 1,
  "passphrase": {
    "kdf": "argon2id",
    "m_kib": 262144, "t": 3, "p": 1,
    "salt": "<b64 of 16 random bytes>",
    "wrapped_key": "<b64>"
  },
  "recovery": {
    "kdf": "hkdf-sha256",
    "wrapped_key": "<b64>"
  },
  "updated_at": 1793216558257,
  "mac": "<b64>"
}
```

`wrapped_key` = b64( nonce[24] || XChaCha20-Poly1305(KEK, nonce, MK, AAD) ).

**Passphrase slot**
- `passphrase'` = Unicode NFC normalization of the passphrase.
- KEK = Argon2id(password = passphrase' as UTF-8, salt = decoded `salt`,
  memory = `m_kib` KiB, iterations = `t`, lanes = `p`, output = 32 bytes).
- AAD = `"memoir/v1/wrap/passphrase|{vault_id}|{key_epoch}|argon2id|{m_kib}|{t}|{p}|{salt}"`,
  using the exact strings and decimal numbers from the JSON.

**Recovery slot**
- The recovery key is 32 random bytes `RK`, shown to the user as text (below).
- KEK = HKDF(RK, info = `"memoir/v1/recovery-kek"`).
- AAD = `"memoir/v1/wrap/recovery|{vault_id}|{key_epoch}|hkdf-sha256"`.

Readers must reject parameters outside: `m_kib` ≤ 1 048 576, 1 ≤ `t` ≤ 16,
1 ≤ `p` ≤ 8.

**Changes across devices** (`updated_at`, `mac`; optional, absent in files
from before they existed)
- `updated_at`: when the passphrase or recovery key last changed, unix ms.
- `mac` = b64( HMAC-SHA256(key = HKDF(MK, info = `"memoir/v1/vault-mac"`),
  data = the file serialized as compact JSON without `mac`, fields in the
  order shown above) ).
- A device holding MK replaces its `vault.json` with another copy only if
  that copy has the same `vault_id` and `key_epoch`, a valid `mac`, and a
  larger `updated_at`. A copy planted or rolled back by anyone without MK is
  ignored. Readers that only unlock may ignore both fields.

### Recovery key text
Crockford base32 (alphabet `0123456789ABCDEFGHJKMNPQRSTVWXYZ`) of
`RK || SHA-256(RK)[0..2]`: 34 bytes, read most-significant bit first in 5-bit
groups, the final group zero-padded → 55 symbols, printed in groups of 5
separated by `-`. Readers ignore case, whitespace and `-`, read `O` as `0`
and `I`/`L` as `1`, require the 3 padding bits to be zero, and verify the
2-byte checksum.

## Object files (notes, folders)

```
offset  size  field
0       4     magic "MMR1"
4       1     format version = 1
5       1     kind = 1 (object)
6       4     key epoch, u32le
10      24    nonce (random)
34      ...   ciphertext || tag[16]
```

- Key: the object key.
- AAD = bytes 0..34 of the file || context, where context =
  `"object:" || object id || ":" || revision id` (ASCII).
- Plaintext = `u32le(n) || zstd_frame[n] || zero bytes`, padded to a multiple
  of 256 bytes. Readers must reject non-zero padding and must cap the
  decompressed size (64 MiB).

In the cloud folder each file is one **revision** of one object, named
`objects/<object id>.<revision id>.mmr` (both ULIDs, Crockford base32). Files
are never overwritten: every change is a new revision. The decompressed
payload is JSON:

```json
{
  "v": 1,
  "parents": ["<revision id>", "..."],
  "object": {
    "id": "<object id>",
    "kind": "folder" | "note",
    "deleted": false,
    "parent_id": "<object id>" | null,
    "sort_key": "a0",
    "title": "…",
    "body": "Markdown (notes and replies)",
    "tags": ["…"],
    "created_at": 1792958800000,
    "updated_at": 1792958800000,
    "trashed_at": null,
    "later_at": null,
    "due_date": "YYYY-MM-DD" | null,
    "attachments": [{ "id": "<blob id>", "size": 123, "mime": "image/png" }],
    "lock": { "kdf": "argon2id", "params": { "m_kib": 19456, "t": 2, "p": 1 },
              "salt": "<base64url>", "hash": "<base64url>" },
    "clocks": { "title": { "at": 0, "d": "<device id>" }, "...": "…" }
  }
}
```

- `lock` (folders only, left out when unset): the folder's password as an
  Argon2id hash. It only hides the folder in the app; the contents are
  encrypted like everything else, so readers can ignore it.

- Times are unix milliseconds. A **reply** is a note whose `parent_id` is a
  note. `deleted: true` is a tombstone: the object was deleted for good.
- `parents` are the revisions this one replaced (several after a merge). The
  current state of an object is its *heads*: revisions no other revision
  lists as a parent. With one head (the usual case) that is the object.
  With several, they were made concurrently. Merging them the way the app
  does (field by field by `clocks`, the body with a 3-way merge) is
  described in docs/ARCHITECTURE.md §6. A simple reader can take the head
  with the latest `updated_at`.
- Replaced revisions are deleted after a while, so a parent may be missing.

## Blob files (attachments)

```
offset  size  field
0       4     magic "MMR1"
4       1     format version = 1
5       1     kind = 2 (blob)
6       4     key epoch, u32le
10      19    nonce prefix (random)
29      4     chunk size C, u32le (1 KiB ≤ C ≤ 16 MiB; the writer uses 64 KiB)
33      ...   chunks
```

- Key: the blob key. AAD for every chunk = bytes 0..33 || context.
- Chunk `i` (from 0) = XChaCha20-Poly1305 with nonce
  `prefix[19] || u32be(i) || last[1]`, where `last` is 1 for the final chunk
  and 0 otherwise.
- Every chunk except the last encrypts exactly C plaintext bytes (C + 16 on
  disk); the last encrypts 0..=C bytes. An empty file is one empty final
  chunk (16 bytes).
- A reader that reaches the end of the data without decrypting a chunk with
  `last = 1` must fail: the file is truncated.

**Blob id** (the attachment's name) = lowercase hex of
HMAC-SHA256(blob-id key, plaintext).

**Context** for attachments = `"blob:" || blob id` (ASCII). Locally they are
stored as `blobs/<blob id>.mmr`. Notes reference them in Markdown as
`attachment:<blob id>`.

## Local database

SQLCipher 4 database keyed with the raw local database key
(`PRAGMA key = "x'<64 hex>'"`), WAL mode, `synchronous = FULL`. Schema:
`crates/memoir-core/src/store/schema.rs`. It never leaves the device.
