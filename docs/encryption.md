# Encryption and identity

## Which cipher is used

Connections try **OpenSec** first and fall back to **VIE** for servers that do not speak it.
`--security auto|opensec|vie`, or `[network] security` in the settings file, changes the
policy: `opensec` refuses to fall back, `vie` never offers OpenSec.

- **VIE** is the original SubSpace cipher. It is public, it is what ASSS implements, and it
  is what every existing zone speaks. It is not cryptography by any modern standard; treat a
  VIE session as readable by anyone on the path.
- **OpenSec** is this project's optional modern transport: X25519 key agreement,
  ChaCha20-Poly1305 records, HKDF-SHA256. It is specified in
  [oss-core's docs/protocol/opensec.md](https://github.com/DangerMouseUK/oss-core/blob/main/docs/protocol/opensec.md),
  its test vectors are reproduced by an independent implementation written from the RFCs with
  no shared code, and it is ready for external cryptographic review — which has not happened
  yet. Read that as "designed carefully and checked twice", not "audited".

### Key pinning

The first time a server proves an OpenSec key, the key is remembered in `known_servers.txt`
beside the settings file. From then on that server must prove the same key over OpenSec, with
no fallback to VIE. A server that changes its key, or an attacker trying to downgrade you,
fails the connection rather than quietly continuing.

## Identity keys

Over OpenSec the client also offers the zone an identity key, which zones can use for bans
that survive a name change. **Each zone gets its own key**, made on first use, so zones
cannot link your sessions to each other. A zone that does not take identities is unaffected.

The keys are kept in `identities.txt` beside the settings file. Keep it private: anyone with
a copy can present your identity at those zones. `[network] identity = false` turns the
offer off entirely.

## Continuum's encryption

Zones that require Continuum's own encryption **cannot be joined from this client**, and that
is not a gap waiting to be filled. Continuum's key expansion is not public; this project does
not reverse-engineer it and does not carry it, which is a deliberate licensing and community
decision rather than an oversight.

What that means in practice:

- Zones that accept VIE — which is most zones that run ASSS, and the original server — are
  reachable.
- Zones configured to require Continuum are not reachable at all.
- The session layer's security path is a trait, so an operator or a private build can
  register its own cipher through `oss_net::Acceptor` / `oss_net::Cipher` without changing
  anything here. That is how a real Continuum 0.40 client came to play on `oss-server`.

The *shape* of the Continuum handshake (`00 10`–`00 14`) is documented in
[oss-core's docs/protocol/encryption.md](https://github.com/DangerMouseUK/oss-core/blob/main/docs/protocol/encryption.md),
read off the client by running it. The algorithm is not there and will not be.
