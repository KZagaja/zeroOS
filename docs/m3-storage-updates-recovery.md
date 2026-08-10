# M3 storage, updates, recovery, and release operations

`ROADMAP.md` defines the normative `ZEROOSB1` journal, `ZEROSLT1` container,
512 MiB GPT layout, API compatibility, and promotion gates. This document
records implementation behavior without changing those contracts.

## Update state machine

PID 1 accepts one update child at a time and stays in its event loop while the
child runs. `CHECK` ends in `current`, `available`, or `failed`; `INSTALL` ends
in `staged` or `failed`. The child emits only bounded `ZEROOS_UPDATE` records.
PID 1 accepts a sequence only from a successful `phase=complete` record, writes
the inactive slot journal transition, flushes it, and then performs an orderly
reboot. A failed child or journal write never changes boot selection.

The updater accepts only the fixed `zeroos-<arch>.slot` URL. It follows at most
five redirects itself so every destination is checked before a connection:
the fixed GitHub latest/versioned release paths and
`release-assets.githubusercontent.com` are production origins. Production
builds contain no alternate origin or CA. The `acceptance` Cargo feature may
embed `ZEROOS_ACCEPTANCE_ORIGIN` and `ZEROOS_ACCEPTANCE_CA_PEM` at compile time
for a disposable local HTTPS fixture.

Resume metadata contains the original URL, a strong ETag, total length, and
durable written length. A resume requires all four to agree with the partial
file, sends `Range` and `If-Range`, and accepts only the exact `206
Content-Range`, ETag, and remaining length. A `200` response truncates and
restarts safely. The partial file and metadata are flushed separately; the
metadata replacement and parent directory are also flushed.

After complete download, the updater strictly parses the container and
manifest, rejects non-increasing sequences and wrong architecture, verifies
RSA-PSS/SHA-256 over the exact manifest bytes with `ring`, and streams the
payload through SHA-256. `/etc/zeroos/release-keys` must contain one or two
regular `<signer>.der` PKCS#1 RSA public keys. The inactive partition label,
number, unique GUID, start sector, sector count, and capacity are validated
before the write. The raw slot is flushed, reread, and fully reverified before
PID 1 stages it.

## Encrypted data and recovery

PID 1 mounts procfs, sysfs, devtmpfs, and tmpfs, validates the `ZEROOS-DATA`
partition identity, and withholds `zeroOS init: READY` until data is unlocked
or first-boot provisioned and ext4 is mounted at `/var/lib/zeroos`. A blank
partition is provisionable; nonblank data without the LUKS2 magic fails closed.
Normal and recovery unlock use cryptsetup keyslot policy behind `zeroos-data`.

The data tool fixes LUKS2 to AES-XTS-plain64 with a 512-bit key and Argon2id,
adds the generated recovery credential as the second keyslot, and creates ext4.
Passphrases and recovery-code buffers are mlocked and zeroized. Terminal echo
is restored by RAII, including read failures. Secret memfds are CLOEXEC except
for the exact cryptsetup child spawn and are never placed in environment values
or command-line text.

`repair-boot` is recovery-only and rewrites both alternating journal records
from validated in-memory state. `repair-data` is reserved for explicit unmount,
`e2fsck -f`, and success-only remount. `factory-reset` remains recovery-only,
requires exact `ERASE-USER-DATA`, twice-confirmed new credentials, and exact
acknowledgement of the displayed recovery code. It targets only
`ZEROOS-DATA`; PID 1 clears pending/trial state only after success. System,
recovery, ESP, firmware variables, and Secure Boot material are never reset.

## Key hierarchy and release custody

PK and KEK remain offline. Production db signing and RSA release signing are
available only through a manually approved protected signer. The recovery db
certificate and recovery signer are independent and offline. Rotation adds the
next db under KEK, publishes an old-key transition release trusting both
manifest keys, switches signing, then removes or revokes the old db entry.

The release process has two human-approved stages: publish an unsigned recovery
payload from an exact source SHA to a draft release for offline signing; then
verify the returned asset hash and signer, rebuild the same SHA reproducibly,
sign selector/system artifacts and manifests through the protected interface,
and publish fixed architecture assets, hashes, and provenance to
`KZagaja/zeroOS-releases`. Devices have no GitHub credential. Private keys may
not enter GitHub arguments, environment variables, logs, or artifacts.

M3 promotion still requires the real offline ceremony, protected environment,
independent security review, public release, production-signed Secure Boot
installation on both architectures, exact-SHA native CI, and the evidence/tag
sequence in `ROADMAP.md`.
