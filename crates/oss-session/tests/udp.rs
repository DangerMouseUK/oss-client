//! Two sessions over real UDP sockets join the test zone and see each other.

use std::time::{Duration, Instant};

use oss_session::test_zone::TestZone;
use oss_session::udp::UdpDriver;
use oss_session::{OwnPosition, Session, SessionConfig, SessionEvent, State, chat_type};

fn map() -> Vec<u8> {
    let mut v = Vec::new();
    for x in 500u32..520 {
        v.extend_from_slice(&(x | (530 << 12) | (1 << 24)).to_le_bytes());
    }
    v
}

#[test]
fn two_clients_see_each_other_over_udp() {
    let settings = oss_sim::ArenaSettings::default().to_client_settings();
    let mut zone = TestZone::bind("127.0.0.1:0", "test.lvl", &map(), &settings).unwrap();
    let addr = zone.local_addr().unwrap().to_string();

    let mut clients: Vec<(Session, UdpDriver, Vec<SessionEvent>)> = ["alpha", "beta"]
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let d = UdpDriver::connect(&addr).unwrap();
            let cfg = SessionConfig {
                name: name.to_string(),
                ship: i as u8,
                ..Default::default()
            };
            (Session::new(cfg, d.now_ms(), 100 + i as u32), d, Vec::new())
        })
        .collect();

    let start = Instant::now();
    let mut chatted = false;
    while start.elapsed() < Duration::from_secs(10) {
        zone.pump();
        for (s, d, events) in &mut clients {
            d.pump(s).unwrap();
            while let Some(ev) = s.poll_event() {
                if let SessionEvent::MapNeeded { .. } = ev {
                    s.download_map();
                }
                events.push(ev);
            }
            let now = d.now_ms();
            if s.state() == State::Playing && s.position_due(now) {
                let x = 8000 + (now / 10 % 100) as i16;
                s.send_position(
                    now,
                    &OwnPosition {
                        x,
                        y: 8200,
                        rotation: 5,
                        energy: 1000,
                        ..Default::default()
                    },
                );
            }
            d.send_pending(s).unwrap();
        }
        if !chatted
            && clients
                .iter()
                .all(|(s, _, _)| s.state() == State::Playing && s.players().count() == 2)
        {
            clients[0].0.send_chat(chat_type::PUBLIC, 0, "hello beta");
            chatted = true;
        }
        let beta_sees_alpha = clients[1]
            .0
            .players()
            .any(|p| p.name == "alpha" && p.position.is_some());
        let alpha_sees_beta = clients[0]
            .0
            .players()
            .any(|p| p.name == "beta" && p.position.is_some());
        let beta_heard = clients[1]
            .2
            .iter()
            .any(|e| matches!(e, SessionEvent::Chat { text, .. } if text == "hello beta"));
        if beta_sees_alpha && alpha_sees_beta && beta_heard {
            let alpha = clients[1].0.players().find(|p| p.name == "alpha").unwrap();
            assert_eq!(alpha.ship, 0);
            assert_eq!(alpha.position.unwrap().y, 8200);
            return;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    for (s, _, events) in &clients {
        eprintln!("{}: {:?} {:?}", s.config().name, s.state(), events);
    }
    panic!("clients did not see each other within 10 s");
}
