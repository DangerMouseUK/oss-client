//! Which client the session presents itself as on the wire.
//!
//! Two packets differ between a SubSpace/VIE client and a Continuum one: the login
//! (`0x09`, 101 bytes, version 134 — against `0x24`, 165 bytes, version 40) and the
//! answer to a security challenge (C2S `0x1A`, which a Continuum client sends longer,
//! carrying its own executable checksum).
//!
//! **An ASSS zone decides which form it expects from the encryption, not from the
//! packet's type byte.** `core.c` registers `PLogin` for both `C2S_LOGIN` and
//! `C2S_CONTLOGIN` and then drops the packet unless its length matches `p->type`:
//!
//! ```text
//! else if ( (p->type == T_VIE  && l != LEN_LOGINPACKET_VIE) ||
//!           (p->type == T_CONT && l != LEN_LOGINPACKET_CONT) )
//!         lm->Log(L_MALICIOUS, "<core> [pid=%d] bad login packet length (%d)", ...);
//! ```
//!
//! and `p->type` is set in `net.c`'s `NewConnection`, which the *encryption module*
//! calls — `enc_vie.c` passes `T_VIE`, a Continuum cipher passes `T_CONT`. Compiling
//! ASSS with `CFG_RELAX_LENGTH_CHECKS` only relaxes the test to `l < 165`, which 101
//! still fails. So a session that negotiates a Continuum cipher and then sends the
//! 101-byte login is dropped before authentication, with nothing on the wire to say
//! why. (Verified in `asss-1.4.4/src/core/core.c:476` and `core/net.c:1835`.
//! **subgame does not reproduce it** — it accepts the 101-byte form over Continuum
//! encryption, so a passing subgame test says nothing about ASSS.)
//!
//! # What is deliberately not here
//!
//! The Continuum half of both packets — the executable checksum, the client id, the
//! key material behind them — **is not in this repository and will not be**. This
//! module is the seam a private implementation plugs into, the same promise
//! `oss_net::Handshake` and `oss_net::Cipher` make one layer down, which the
//! Continuum work found stops short of the session. Everything public here is the
//! SubSpace form, whose arithmetic has been public for twenty years.
//!
//! [`VieClient`] is the default and is what the tests in this repository exercise.

use std::fmt::Debug;
use std::sync::Arc;

use oss_proto::c2s::{self, Login};
use oss_proto::checksum;

/// The login packet a client sends: which form, and the fields that differ with it.
///
/// The lengths are [`Login::VIE_LEN`] (101) and [`Login::CONT_LEN`] (165); the
/// encoder picks between them from `continuum_id` being set, so the three fields
/// here have to agree with each other. [`LoginForm::vie`] and
/// [`LoginForm::continuum`] build consistent pairs.
#[derive(Debug, Clone)]
pub struct LoginForm {
    /// Login packet type byte: `0x09` (SubSpace) or `0x24` (Continuum).
    pub kind: u8,
    /// `client_version`: 134 for SubSpace, 40 for Continuum 0.40. ASSS rejects a
    /// `T_VIE` login whose version is not 134 (`CVERSION_VIE`) unless it was built
    /// with `CFG_RELAX_LENGTH_CHECKS`.
    pub version: u16,
    /// Continuum only: the 64 opaque bytes the zone forwards to its biller. `None`
    /// makes the packet the 101-byte SubSpace form.
    pub continuum_id: Option<[u8; 64]>,
}

impl LoginForm {
    /// The SubSpace form: `0x09`, version 134, no client id.
    pub fn vie() -> Self {
        Self {
            kind: c2s::kind::LOGIN,
            version: 134,
            continuum_id: None,
        }
    }

    /// The Continuum form: `0x24`, version 40, and the client id the private side
    /// supplies.
    pub fn continuum(version: u16, id: [u8; 64]) -> Self {
        Self {
            kind: c2s::kind::CONT_LOGIN,
            version,
            continuum_id: Some(id),
        }
    }

    /// The length this form encodes to, for callers that want to check it against
    /// what a zone will accept before sending.
    pub fn encoded_len(&self) -> usize {
        if self.continuum_id.is_some() {
            Login::CONT_LEN
        } else {
            Login::VIE_LEN
        }
    }
}

/// What the session knows when a security challenge arrives (S2C `0x18` with a
/// non-zero key), handed to [`ClientProfile::security`].
///
/// The session computes everything that is not secret — the settings and map
/// checksums are already in the response it builds — so a profile only has to supply
/// the parts that depend on the client binary.
#[derive(Debug, Clone, Copy)]
pub struct SecurityChallenge {
    /// The key from the challenge. Both checksums are keyed on it, so an answer
    /// cannot be precomputed.
    pub key: u32,
    /// CRC-32 of the arena's map ([`oss_proto::checksum::map_crc32`]), when a map is
    /// loaded. Later Continuum builds append it to the response; 0.40 does not.
    pub map_crc32: Option<u32>,
}

/// The parts of a C2S `0x1A` that depend on which client is being presented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecurityAnswer {
    /// Checksum of the client executable, keyed by [`SecurityChallenge::key`].
    pub exe_checksum: u32,
    /// Continuum only: the timer drift, and the map CRC-32 on builds that send one.
    /// `None` keeps the response at `SecurityResponse::VIE_LEN`; `Some` lengthens
    /// it to `CONT_LEN` or `CONT_CRC_LEN` (see [`oss_proto::c2s::SecurityResponse`]).
    pub continuum: Option<(i16, Option<u32>)>,
}

/// Which client a session presents itself as.
///
/// Implement this outside the repository to present a Continuum client; see the
/// module documentation for why the two forms are not interchangeable and why the
/// Continuum half is not here.
pub trait ClientProfile: Debug + Send + Sync {
    /// The login form to send.
    fn login(&self) -> LoginForm;

    /// Answer a security challenge.
    fn security(&self, challenge: SecurityChallenge) -> SecurityAnswer;
}

/// The SubSpace/VIE client: the default, and the only one in this repository.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VieClient;

impl ClientProfile for VieClient {
    fn login(&self) -> LoginForm {
        LoginForm::vie()
    }

    fn security(&self, challenge: SecurityChallenge) -> SecurityAnswer {
        SecurityAnswer {
            exe_checksum: checksum::vie_exe_checksum(challenge.key),
            continuum: None,
        }
    }
}

/// A handle to a [`ClientProfile`], so [`crate::SessionConfig`] stays `Clone`.
pub type Profile = Arc<dyn ClientProfile>;

/// The default profile: a SubSpace/VIE client.
pub fn vie() -> Profile {
    Arc::new(VieClient)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oss_proto::Packet;
    use oss_proto::c2s::SecurityResponse;

    /// The lengths ASSS checks `p->type` against. If these ever stop matching what
    /// the encoder produces, a Continuum zone drops our login with nothing on the
    /// wire to say why, which is the whole reason this module exists.
    #[test]
    fn the_two_login_forms_encode_to_the_lengths_asss_demands() {
        let mut out = Vec::new();
        let vie = LoginForm::vie();
        Login {
            kind: vie.kind,
            client_version: vie.version,
            cont_id: vie.continuum_id,
            ..Default::default()
        }
        .encode(&mut out);
        assert_eq!(out.len(), Login::VIE_LEN, "LEN_LOGINPACKET_VIE");
        assert_eq!(out.len(), vie.encoded_len());
        assert_eq!(out[0], 0x09);

        out.clear();
        let cont = LoginForm::continuum(40, [7; 64]);
        Login {
            kind: cont.kind,
            client_version: cont.version,
            cont_id: cont.continuum_id,
            ..Default::default()
        }
        .encode(&mut out);
        assert_eq!(out.len(), Login::CONT_LEN, "LEN_LOGINPACKET_CONT");
        assert_eq!(out.len(), cont.encoded_len());
        assert_eq!(out[0], 0x24);
        // 165 - 101: the client id is the whole difference.
        assert_eq!(Login::CONT_LEN - Login::VIE_LEN, 64);
    }

    #[test]
    fn the_default_profile_is_the_subspace_client() {
        let p = vie();
        let form = p.login();
        assert_eq!(form.kind, c2s::kind::LOGIN);
        assert_eq!(form.version, 134, "ASSS's CVERSION_VIE");
        assert_eq!(form.continuum_id, None);

        let answer = p.security(SecurityChallenge {
            key: 0x1234_5678,
            map_crc32: Some(0xDEAD_BEEF),
        });
        assert_eq!(
            answer.exe_checksum,
            checksum::vie_exe_checksum(0x1234_5678),
            "the public VIE arithmetic, keyed by the challenge"
        );
        assert_eq!(
            answer.continuum, None,
            "a VIE client sends no trailer, whatever it was told about the map"
        );
    }

    /// A stand-in for the private side: it answers in the Continuum shape without
    /// any Continuum arithmetic, so the session's plumbing can be tested here while
    /// the real implementation stays out of this repository.
    #[derive(Debug)]
    struct StubContinuum;

    impl ClientProfile for StubContinuum {
        fn login(&self) -> LoginForm {
            LoginForm::continuum(40, [0xAB; 64])
        }
        fn security(&self, challenge: SecurityChallenge) -> SecurityAnswer {
            SecurityAnswer {
                exe_checksum: challenge.key ^ 0x5555_5555,
                continuum: Some((0, challenge.map_crc32)),
            }
        }
    }

    #[test]
    fn a_continuum_shaped_profile_lengthens_both_packets() {
        let p: Profile = Arc::new(StubContinuum);
        assert_eq!(p.login().encoded_len(), Login::CONT_LEN);

        let with_crc = p.security(SecurityChallenge {
            key: 1,
            map_crc32: Some(9),
        });
        let mut out = Vec::new();
        SecurityResponse {
            exe_checksum: with_crc.exe_checksum,
            continuum: with_crc.continuum,
            ..Default::default()
        }
        .encode(&mut out);
        assert_eq!(out.len(), SecurityResponse::CONT_CRC_LEN);
        assert!(SecurityResponse::is_continuum_len(out.len()));

        // A 0.40 client sends the drift without the CRC.
        out.clear();
        SecurityResponse {
            continuum: Some((0, None)),
            ..Default::default()
        }
        .encode(&mut out);
        assert_eq!(out.len(), SecurityResponse::CONT_LEN);
        assert!(SecurityResponse::is_continuum_len(out.len()));
    }
}
