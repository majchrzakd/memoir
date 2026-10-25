# Memoir — Storage format, version 1

This document is enough to decrypt a Memoir vault without the app (that is
what `memoir-cli` will do in M7). Any incompatible change requires a new
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
  }
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
- AAD = bytes 0..34 of the file || context. The context identifies the
  object revision; it is defined by the sync layer (M6) and documented there.
- Plaintext = `u32le(n) || zstd_frame[n] || zero bytes`, padded to a multiple
  of 256 bytes. Readers must reject non-zero padding and must cap the
  decompressed size (64 MiB).
- The decompressed payload's encoding is defined by the store layer (M2).

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
