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
`ZEROOS-DATA`; the console command is exactly `factory-reset ERASE-USER-DATA`,
and PID 1 clears pending/trial state only after success. System,
recovery, ESP, firmware variables, and Secure Boot material are never reset.

## Key hierarchy and release custody

PK and KEK remain offline. Production db signing and RSA release signing are
available only through the protected signer. The recovery db
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

Under ADR 0002, only the repository owner may dispatch a release from `main`.
Dispatch accepts only a full source SHA identical to `main` that already has a
successful exact-SHA `main` push CI run. Both release environments permit only
`main`; unavailable reviewer/self-review rules are not required. Production
signing runs only on architecture-labelled
protected runners after the `zeroos-sign` version and executable hash match
protected configuration. Its provenance must expose public fingerprints and
must not contain private or secret material labels.

`zeroos-sign` reads root-owned `/etc/zeroos-sign/operator.conf`; it never accepts
key configuration or authentication on its command line. The exact keys are
`engine`, `selector-key`, `production-key`, `release-key`, `selector-cert`,
`production-cert`, `recovery-cert`, `release-signer`, and `fingerprints`.
Private-key values are PKCS#11 object-selector URIs without PIN or password
attributes. Connector, module, and authentication configuration remains
runner-owned. Workspace acceptance uses a disposable SoftHSM RSA-3072 object
to cross the same libp11/OpenSSL RSA-PSS path before production hardware.

The public trust ceremony commits `policy/m3-trust/pk.pem`, `kek.pem`,
`db.pem`, `next-db.pem`, `recovery.pem`, the PKCS#1 RSA public keys
`release-current.der` and `release-next.der`, and `fingerprints.sha256`. The
fingerprint manifest uses
`sha256sum`'s two-space format and covers every committed certificate and DER
key. It never contains a private key, backup location, credential, or recovery
material.

The initial production initramfs embeds only `release-current.der` as
`/etc/zeroos/release-keys/release-current.der`; committing the next public key
does not activate it. A separately tested transition release is responsible
for overlapping runtime release-key trust before signer rotation.

Production publishes a candidate as an immutable public prerelease. Native protected
runners then invoke `cargo xtask test-release --arch <arch> --sequence <n>
--url <public-tag-url>` without a download credential. That command verifies
the hash manifest, source/architecture/sequence provenance, committed public
fingerprints, PE signatures, and the slot manifest signature, installs a fresh
512 MiB image, enrolls the committed public certificates in a fresh variable
store, and boots both normal and recovery under Secure Boot. Only two passing
native jobs permit copying those exact candidate bytes to the immutable final
release and marking it latest; public assets are never replaced after
publication.

M3 promotion still requires the real offline ceremony, signed/timestamped
record and redacted HSM audit export, protected environments and runners, two
encrypted geographic backups with a verified restore, immutable public
release, production-signed Secure Boot installation on both architectures,
exact-SHA native CI, and the evidence/tag sequence in `ROADMAP.md`. ADR 0002
permanently removes independent review and second-person witnessing and records
the resulting single-operator compromise risk.
