# Encryption and identity

## Which cipher is used

Every security mechanism the client can speak is described once, in a registry, and
everything else is read from it: the start-up lines, the order the client tries them in, and
the words `--security` and the settings file accept. A build of this repository logs, at start:

```text
Mechanisms supported: None, Subspace, OpenSecurity
Automatic order: OpenSecurity, then Subspace
```

By default (`auto`) a connection tries the **automatic order**: every mechanism that may be
tried on its own, strongest first, stepping down to the next when a server has not answered
within 1.5 seconds. Here that is **OpenSec**, then **VIE** for servers that do not speak it.
The order comes from the strength each mechanism declares, never from a list written out.

`--security <name>`, or `[network] security = <name>` in the settings file, uses **one
mechanism alone** instead: `opensec` refuses to fall back, `vie` never offers OpenSec, `none`
is below. `--help`, and the comment the client writes above that setting, list exactly the
words this build has. A word in the settings file that this build does not know is reported,
`auto` is used for that run, and the word is left as it is: saving the settings writes it
back unchanged.

The `oss-bot` program negotiates exactly the same way, with `auto` as its default too.

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
beside the settings file. From then on `auto` uses OpenSec alone for that server, and it must
prove the same key, with no fallback to VIE. A server that changes its key, or an attacker
trying to downgrade you, fails the connection rather than quietly continuing. A mechanism
named on purpose (`--security vie`, `--security none`) still wins over a remembered key: a
person asking is a decision, and the remembered key is not.

## Identity keys

Over OpenSec the client also offers the zone an identity key, which zones can use for bans
that survive a name change. **Each zone gets its own key**, made on first use, so zones
cannot link your sessions to each other. A zone that does not take identities is unaffected.

The keys are kept in `identities.txt` beside the settings file. Keep it private: anyone with
a copy can present your identity at those zones. `[network] identity = false` turns the
offer off entirely.

## Remembered passwords

The start-up screen has a **Remember** line under the password. It starts unticked. Tick it,
and once the zone has accepted the password it is kept in your operating system's own
password store: **Windows Credential Manager** on Windows, the **Keychain** on macOS. Each
entry is filed under `Open SubSpace` as `name@zone:port`, so you can see it and delete it there
with the tools you already have. Next time, the same name and zone fill the password in for
you. Untick the line to delete the saved password straight away. The password is never written
to the settings file, `login.ini` or any file of the client's own, and never to a log.

Play pressed before the saved password has been looked up waits for it, for a second or so,
rather than sending an empty one. A zone named with `--server` and no `--password` uses the
saved password for that name and zone; with none saved, the start-up screen opens filled in
and waits for you to type it. `--password ""` goes straight in with no password, for a zone
that wants none.

**On Linux** the password goes to your desktop's own keyring through the Secret Service (GNOME
Keyring, or KWallet), encrypted on its way there. A system with no such keyring, such as a
server or a minimal desktop, has nowhere safe to keep it, so the password is not stored at
all and the Remember line is not shown.

## Continuum's encryption

Continuum's key expansion is not public and this repository does not carry it. That is
deliberate rather than an oversight, and it is not a gap waiting to be filled.

What that means in practice:

- A client built from this repository cannot join a zone that requires Continuum encryption.
- Zones that accept VIE — which is most zones that run ASSS, and the original server — are
  reachable, as are OpenSec zones.
- The session layer's security path is pluggable, so a build that supplies its own cipher can
  plug it in through `oss_net::Acceptor` / `oss_net::Cipher` without changing
  anything here. That is how a real Continuum 0.40 client came to play on `oss-server`.
- That seam reaches the graphical client as well as a headless one, and it is not specific
  to Continuum: it is how *any* scheme outside this repository plugs in, by registering a
  mechanism. See [Supplying your own scheme](#supplying-your-own-scheme) below. Nothing here
  implements one, and nothing here knows what one does.

The *shape* of the Continuum handshake (`00 10`–`00 14`) is documented in
[oss-core's docs/protocol/encryption.md](https://github.com/DangerMouseUK/oss-core/blob/main/docs/protocol/encryption.md),
read off the client by running it. **The algorithm is not in these repositories and will not
be.**

## Supplying your own scheme

`oss-client` is a library beside its binary, so a build outside this repository can add its
own mechanisms, and its own client identity, without changing anything here. It registers
each mechanism in an `oss_client::Supplied`, one line each, and hands that to
`options_with` **before the arguments are read**, because the mechanisms decide what
`--security` and the settings file accept:

```rust
let mut supplied = Supplied { client: Some(Arc::new(MyClient)), ..Supplied::default() };
supplied.mechanisms.register(my_mechanism())?;
let options = oss_client::options_with(std::env::args().skip(1), supplied)?.unwrap();
oss_client::run(options)?;
```

That one line is all a mechanism needs. It is named in the start-up line, it takes its place
in the automatic order by the strength it declares, it becomes a word `--security` and the
settings file accept, and `oss_client::usage` lists it. **A mechanism is added, never swapped
in**: the built-in ones stay, and `--security vie` still means VIE.

| Field | Type | What it decides |
|---|---|---|
| `mechanisms` | `oss_client::MechanismRegistry` (from `oss_net`) | What the client can speak. Starts as the built-in mechanisms. Each `oss_net::Mechanism` has an identifier (`name@your-domain` for anyone outside this project), a display name, a strength, an era, and a client half that builds an `oss_net::Handshake` for each connection, which this repository neither inspects nor knows anything about |
| `client` | `oss_client::ClientProfile` (from `oss_session`) | Which client the session presents itself as: the login form it sends, and how it answers a security challenge |

### The login form must match the era

**This is the one that will cost you a day.** A zone picks which login form it expects from
the *encryption in use*, not from the packet's type byte. So a session whose login form does
not match its encryption is **dropped at login in silence**: no error, no rejection, nothing
on the wire.

Each mechanism declares its **era**: `SubSpace`, `Continuum` or `Modern` (what the zone should
treat the connection as; OpenSec is `Modern`, and its hello is classified as a SubSpace
client's). The session checks the login form against the era of the mechanism that won,
whether it was reached by the automatic order or named, and logs
`... the zone will drop this login` when they disagree. If a connection appears to come up and
then simply stops, look for that line first. A client profile that serves more than one era
has to know which mechanism won before it chooses the form.
