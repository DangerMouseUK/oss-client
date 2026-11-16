# Encryption and identity

## Which cipher is used

Connections try **OpenSec** first and fall back to **VIE** for servers that do not speak it.
`--security auto|opensec|vie|none`, or `[network] security` in the settings file, changes the
policy: `opensec` refuses to fall back, `vie` never offers OpenSec, `none` is below.

**VIE is the floor, and nothing automatic goes below it.** Each step down is triggered by an
attempt that failed, and anyone able to disturb the traffic can make every stronger attempt
look as though it failed. A ladder whose bottom rung is cleartext therefore hands that
attacker cleartext for the price of being a nuisance, without breaking any cipher. Keeping
VIE as the bottom rung costs nothing — it is the path every existing zone speaks anyway —
and it means the worst an attacker can arrange by interfering is a VIE session rather than a
readable one.

So the client also refuses the two answers that used to put it in the clear. A server saying
"I will not encrypt" — which on the wire is just your own connection key sent back to you —
ends the connection instead of starting an unencrypted one, because nothing in that exchange
is signed and anything on the network path can send it. You will see
`Couldn't connect to <zone>: the server's 00 02 refused encryption...`.

**`--security none` is the way past that, and it is for development.** It sends your password
in the clear. It is never selected for you: the automatic policy cannot reach it, a server
cannot ask for it, and a key remembered for the zone does not override it. The server has to
have been set to allow it as well (`Security:AllowUnencrypted` in its `global.conf`), so an
unencrypted session takes a decision at each end. The zone-list lookup is the one connection
this client makes that still accepts an unencrypted answer, because it carries only public
information in both directions and refusing would risk the zone browser to buy nothing.

Three schemes have been used over the life of this game — VIE, Continuum's own, and OpenSec.
[oss-core's docs/protocol/encryption.md](https://github.com/DangerMouseUK/oss-core/blob/main/docs/protocol/encryption.md#the-three-eras)
says what each one is; the two this client implements are below.

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

Continuum's key expansion is not public and this repository does not carry it. That is
deliberate rather than an oversight, and it is not a gap waiting to be filled.

What that means in practice:

- A client built from this repository cannot join a zone that requires Continuum encryption.
- Zones that accept VIE — which is most zones that run ASSS, and the original server — are
  reachable, as are OpenSec zones.
- The session layer's security path is a trait, so a build that supplies its own cipher can
  register it through `oss_net::Acceptor` / `oss_net::Cipher` without changing
  anything here. That is how a real Continuum 0.40 client came to play on `oss-server`.
- That seam reaches the graphical client as well as a headless one, and it is not specific
  to Continuum: it is how *any* scheme outside this repository plugs in. See
  [Supplying your own scheme](#supplying-your-own-scheme) below. Nothing here implements
  either half, and nothing here knows what one returns.

The *shape* of the Continuum handshake (`00 10`–`00 14`) is documented in
[oss-core's docs/protocol/encryption.md](https://github.com/DangerMouseUK/oss-core/blob/main/docs/protocol/encryption.md),
read off the client by running it. **The algorithm is not in these repositories and will not
be.**

## Supplying your own scheme

`oss-client` is a library beside its binary, so a build outside this repository can give a
session its own cipher and its own client identity without changing anything here. There is
no plugin registry and no init hook: you fill in one field and call `run`.

```rust
let mut options = oss_client::options(std::env::args().skip(1))?.unwrap();
options.supplied = Supplied { security: Some(profile.clone()), client: Some(profile) };
oss_client::run(options)?;
```

`Options::supplied` is an `oss_client::Supplied`, and its two fields take one trait each:

| Field | Trait | What it decides |
|---|---|---|
| `security` | `oss_client::SecurityProfile` (from `oss_session::security`) | The handshake the connection opens with. It returns an `oss_net::Handshake`, which this repository neither inspects nor knows anything about |
| `client` | `oss_client::ClientProfile` (from `oss_session`) | Which client the session presents itself as: the login form it sends, and how it answers a security challenge |

A supplied profile is **always** the one used, whatever `--security` asked for, so no new
mode name is needed and none has to be registered.

### Both fields, always

**This is the one that will cost you a day.** A zone picks which login form it expects from
the *encryption in use*, not from the packet's type byte. So a session given a handshake
without a matching client profile is **dropped at login in silence**: no error, no
rejection, nothing on the wire. If a connection appears to come up and then simply stops,
check both fields before looking anywhere else.

The cheapest way to make that mistake impossible is to implement both traits on one type and
hand the same value to both fields. Supplying them separately is the shape of the mistake.
