//! Powerballs (soccer), as the client sees them. Ported from nullspace (MIT).
//!
//! The server reports each ball's position, velocity, carrier or last shooter and the
//! server time of that state (S2C `0x2E`; time 0 means carried). Between reports the
//! client moves free balls itself, tick by tick: integer positions in 1/1000 pixel,
//! velocity in pixels per 10 s (so one unit per tick), reversing on walls and slowing
//! by the shooter's ship's `SoccerBallFriction`. A carried ball sits just in front of
//! its carrier.
//!
//! Your own ship asks to pick a ball up when within `SoccerBallProximity` (not while
//! the ball is "phased": just fired, within `PassDelay`, or just touched), fires it
//! with Ctrl/Tab (unless `Soccer:AllowGuns`/`AllowBombs` lets those fire weapons),
//! throws it backwards when `SoccerThrowTime` runs out, and reports a goal when a ball
//! it last carried crosses an enemy goal tile (by `Soccer:Mode`).

use oss_session::BallPosition;

/// Ball ids run 0..8 (ASSS's `Soccer:BallCount` limit).
pub const MAX_BALLS: usize = 8;
/// After touching a ball, wait this long (ticks) before touching it again.
const TOUCH_DELAY: i32 = 100;
/// A ball you touched is hidden this long (ticks) while the server decides.
const TOUCH_HIDE: i32 = 40;
/// Longest catch-up simulated for a stale report (ticks).
const MAX_CATCH_UP: i32 = 6000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BallState {
    World,
    Carried,
    /// Reported as a goal; waiting for the server.
    Goal,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Ball {
    pub id: u8,
    /// 1/1000 pixel.
    x: i64,
    y: i64,
    /// Pixels per 10 s.
    vx: i32,
    vy: i32,
    friction: i32,
    friction_delta: i32,
    /// Team whose bricks it passes (the carrier's), or 0xFFFF.
    freq: u16,
    /// Carrier, or the last player to fire it.
    pub carrier: Option<i16>,
    pub state: BallState,
    /// Server time of the last report (0 while carried).
    timestamp: u32,
    last_fire: u32,
    last_touch: u32,
    /// When the last report arrived (for `BallBlankDelay`).
    last_report: u32,
}

impl Ball {
    /// Pixel position of a free ball.
    pub fn pixels(&self) -> (i32, i32) {
        ((self.x / 1000) as i32, (self.y / 1000) as i32)
    }
}

/// Per-ship and arena rules, from the arena settings.
#[derive(Debug, Clone, Default)]
pub struct BallRules {
    /// Per ship: friction, pickup distance (pixels), fire speed (pixels per 10 s),
    /// carry time before an automatic throw (ticks, 0 = none).
    pub friction: [i32; 8],
    pub proximity: [i32; 8],
    pub speed: [i32; 8],
    pub throw_time: [u32; 8],
    /// Ticks after a shot before anyone can catch the ball.
    pub pass_delay: u32,
    /// `Soccer:Mode`: which goals are your own.
    pub mode: u8,
    pub allow_guns: bool,
    pub allow_bombs: bool,
    /// `Soccer:BallBounce`: balls bounce off walls (otherwise they stop there).
    pub bounce: bool,
    /// `Soccer:BallBlankDelay`: ticks without a report before a ball can't be
    /// picked up (0 = no limit).
    pub blank_delay: u32,
    /// `Misc:DisableBallThroughWalls`: no pickups with a wall in between.
    pub no_wall_pass: bool,
}

impl BallRules {
    /// The rules from the arena settings (`Soccer:*` and each ship's ball settings).
    pub fn from_arena(a: &oss_sim::ArenaSettings) -> Self {
        let mut r = Self {
            pass_delay: a.soccer.pass_delay,
            mode: a.soccer.mode,
            allow_guns: a.soccer.allow_guns,
            allow_bombs: a.soccer.allow_bombs,
            bounce: a.soccer.ball_bounce,
            blank_delay: a.soccer.ball_blank_delay,
            no_wall_pass: a.soccer.no_wall_pass,
            ..Self::default()
        };
        for (i, s) in a.ships.iter().enumerate() {
            r.friction[i] = s.soccer_ball_friction as i32;
            r.proximity[i] = s.soccer_ball_proximity as i32;
            r.speed[i] = s.soccer_ball_speed as i32;
            r.throw_time[i] = s.soccer_throw_time;
        }
        r
    }
}

/// Your ship, for pickups, firing and goals.
#[derive(Debug, Clone, Copy)]
pub struct BallPlayer {
    pub pid: i16,
    /// 0..8; 8 = spectating.
    pub ship: u8,
    pub freq: u16,
    /// Centre, pixels.
    pub x: i32,
    pub y: i32,
    /// Pixels per 10 s.
    pub vx: i32,
    pub vy: i32,
    /// 0..40, 0 = up, clockwise.
    pub direction: u8,
    /// Alive, not attached to another ship.
    pub can_play: bool,
}

/// What the client should tell the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BallAction {
    Pickup {
        id: u8,
        time: u32,
    },
    Fire {
        id: u8,
        pos: (i16, i16),
        vel: (i16, i16),
    },
    Goal {
        id: u8,
        x: i16,
        y: i16,
    },
}

/// Unit heading for 40-step `direction` (0 = up, clockwise).
fn heading(direction: u8) -> (f32, f32) {
    let a = (direction % 40) as f32 / 40.0 * std::f32::consts::TAU;
    (a.sin(), -a.cos())
}

fn after(now: u32, then: u32) -> i32 {
    now.wrapping_sub(then) as i32
}

#[derive(Debug, Clone, Default)]
pub struct Balls {
    pub balls: [Option<Ball>; MAX_BALLS],
    /// The ball you carry and the tick its carry time runs out (if timed).
    carrying: Option<(u8, Option<u32>)>,
    last_pickup_request: u32,
}

impl Balls {
    pub fn carrying(&self) -> Option<u8> {
        self.carrying.map(|c| c.0)
    }

    /// Applies S2C `0x2E` at server tick `now`. `carrier` gives a player's team and
    /// ship; `solid` says whether a tile blocks the ball for a team.
    #[allow(clippy::too_many_arguments)]
    pub fn report(
        &mut self,
        p: &BallPosition,
        now: u32,
        me: &BallPlayer,
        rules: &BallRules,
        carrier: impl Fn(i16) -> Option<(u16, u8)>,
        solid: &impl Fn(i32, i32, u16) -> bool,
    ) {
        let id = p.ball_id as usize;
        if id >= MAX_BALLS {
            return;
        }
        let time = p.time & 0x7FFF_FFFF;
        let owner = (p.pid >= 0).then_some(p.pid);
        let fresh = match &self.balls[id] {
            None => true,
            Some(b) => {
                after(time, b.timestamp) > 0
                    || b.state == BallState::Goal
                    || (b.state == BallState::Carried && time != 0)
            }
        };
        let ball = self.balls[id].get_or_insert(Ball {
            id: p.ball_id,
            x: 0,
            y: 0,
            vx: 0,
            vy: 0,
            friction: 0,
            friction_delta: 0,
            freq: 0xFFFF,
            carrier: None,
            state: BallState::World,
            timestamp: 0,
            last_fire: now,
            last_touch: now.wrapping_sub(TOUCH_DELAY as u32),
            last_report: now,
        });
        ball.last_report = now;
        if fresh {
            ball.x = p.x as i64 * 1000;
            ball.y = p.y as i64 * 1000;
            (ball.vx, ball.vy) = (p.x_speed as i32, p.y_speed as i32);
            ball.freq = 0xFFFF;
            ball.state = BallState::World;
            ball.last_fire = time;
            if self.carrying.is_some_and(|c| c.0 == p.ball_id) {
                self.carrying = None;
            }
            let mut ship = 0;
            if let Some(pid) = owner {
                if let Some((freq, s)) = carrier(pid) {
                    ball.freq = freq;
                    ship = if s >= 8 { 0 } else { s };
                }
                if pid == me.pid {
                    self.last_pickup_request = now;
                }
                ball.last_touch = now.wrapping_sub(TOUCH_DELAY as u32);
            }
            if ball.vx != 0 || ball.vy != 0 {
                ball.friction_delta = rules.friction[ship as usize];
                ball.friction = 1_000_000;
            } else {
                ball.friction = 0;
            }
            ball.carrier = owner;
            let catch_up = if time == 0 {
                0
            } else {
                after(now, time).clamp(0, MAX_CATCH_UP)
            };
            for _ in 0..catch_up {
                step_ball(ball, rules.bounce, solid);
            }
            ball.timestamp = time;
        } else if time == 0 {
            match owner {
                // Carried.
                Some(pid) => {
                    ball.timestamp = 0;
                    ball.carrier = Some(pid);
                    (ball.vx, ball.vy) = (0, 0);
                    let in_ship = carrier(pid).is_some_and(|(_, s)| s < 8);
                    if ball.state != BallState::Carried && in_ship {
                        ball.state = BallState::Carried;
                        if pid == me.pid {
                            let t = rules.throw_time[(me.ship % 8) as usize];
                            self.carrying = Some((p.ball_id, (t > 0).then(|| now.wrapping_add(t))));
                        }
                    }
                }
                // Gone.
                None => {
                    if self.carrying.is_some_and(|c| c.0 == p.ball_id) {
                        self.carrying = None;
                    }
                    self.balls[id] = None;
                }
            }
        }
    }

    /// One tick at server time `now`: moves free balls, and returns what your ship
    /// should send (pickups, goals, the automatic throw). `goal_tile` says whether a
    /// tile is a goal.
    pub fn step(
        &mut self,
        now: u32,
        me: &BallPlayer,
        rules: &BallRules,
        solid: &impl Fn(i32, i32, u16) -> bool,
        goal_tile: &impl Fn(i32, i32) -> bool,
    ) -> Vec<BallAction> {
        let mut out = Vec::new();
        let playing = me.ship < 8 && me.can_play;
        for ball in self.balls.iter_mut().flatten() {
            if ball.state == BallState::World {
                step_ball(ball, rules.bounce, solid);
                // A goal by a ball you last carried.
                if ball.carrier == Some(me.pid) {
                    let (tx, ty) = (
                        ball.x.div_euclid(16_000) as i32,
                        ball.y.div_euclid(16_000) as i32,
                    );
                    if goal_tile(tx, ty) && !is_team_goal(rules.mode, me.freq, tx, ty) {
                        ball.state = BallState::Goal;
                        out.push(BallAction::Goal {
                            id: ball.id,
                            x: tx as i16,
                            y: ty as i16,
                        });
                    }
                }
            }
            // Pickup.
            let fresh =
                rules.blank_delay == 0 || after(now, ball.last_report) < rules.blank_delay as i32;
            let ready = ball.state == BallState::World
                && fresh
                && after(now, ball.last_fire) >= rules.pass_delay as i32
                && after(now, ball.last_touch) >= TOUCH_DELAY;
            if ready
                && playing
                && self.carrying.is_none()
                && after(now, self.last_pickup_request) >= TOUCH_DELAY
            {
                let reach = rules.proximity[(me.ship % 8) as usize];
                let (bx, by) = ball.pixels();
                let clear =
                    !rules.no_wall_pass || clear_line((me.x, me.y), (bx, by), me.freq, solid);
                if (me.x - bx).abs() < reach && (me.y - by).abs() < reach && clear {
                    out.push(BallAction::Pickup {
                        id: ball.id,
                        time: ball.timestamp,
                    });
                    self.last_pickup_request = now;
                    ball.last_touch = now;
                }
            }
        }
        // Carry time over: throw it backwards, as Continuum does.
        if let Some((id, Some(deadline))) = self.carrying
            && after(now, deadline) >= 0
            && playing
        {
            out.push(self.fire_action(id, me, rules, true));
            self.carrying = None;
        }
        out
    }

    /// Ctrl/Tab while carrying: fires the ball unless `AllowGuns`/`AllowBombs` lets
    /// that key fire weapons instead. Returns the shot, and the key should then not
    /// fire a weapon.
    pub fn fire(
        &mut self,
        me: &BallPlayer,
        rules: &BallRules,
        gun: bool,
        bomb: bool,
        now: u32,
    ) -> Option<BallAction> {
        let id = self.carrying?;
        let id = id.0;
        let by_gun = gun && !rules.allow_guns;
        let by_bomb = bomb && !rules.allow_bombs;
        if !(by_gun || by_bomb) {
            return None;
        }
        self.carrying = None;
        if let Some(b) = self.balls.get_mut(id as usize).and_then(Option::as_mut) {
            b.last_fire = now;
        }
        Some(self.fire_action(id, me, rules, false))
    }

    fn fire_action(
        &self,
        id: u8,
        me: &BallPlayer,
        rules: &BallRules,
        backwards: bool,
    ) -> BallAction {
        let speed = rules.speed[(me.ship % 8) as usize] as f32;
        let (hx, hy) = heading(me.direction);
        let sign = if backwards { -1.0 } else { 1.0 };
        let vx = me.vx as f32 + hx * speed * sign;
        let vy = me.vy as f32 + hy * speed * sign;
        let clamp = |v: f32| v.clamp(i16::MIN as f32, i16::MAX as f32) as i16;
        BallAction::Fire {
            id,
            pos: (me.x as i16, me.y as i16),
            vel: (clamp(vx), clamp(vy)),
        }
    }

    /// Where to draw each ball: (ball, pixel position, phased). Carried balls sit in
    /// front of their carrier (`ship_at` gives a player's centre, direction and
    /// radius); balls you just touched are hidden while the server decides.
    pub fn visible(
        &self,
        now: u32,
        rules: &BallRules,
        ship_at: impl Fn(i16) -> Option<(f32, f32, u8, i32)>,
    ) -> Vec<(u8, (f32, f32), bool)> {
        let mut out = Vec::new();
        for b in self.balls.iter().flatten() {
            if b.state == BallState::Carried {
                if let Some((x, y, dir, radius)) = b.carrier.and_then(&ship_at) {
                    let (hx, hy) = heading(dir);
                    let ext = (radius - 4).max(0) as f32;
                    out.push((b.id, (x + hx * ext, y + hy * ext), false));
                }
                continue;
            }
            let touched = after(now, b.last_touch);
            if touched > 0 && touched < TOUCH_HIDE {
                continue;
            }
            let phased = after(now, b.last_fire) < rules.pass_delay as i32
                || (touched > 0 && touched < TOUCH_DELAY);
            let (x, y) = b.pixels();
            out.push((b.id, (x as f32, y as f32), phased));
        }
        out
    }
}

/// Whether no wall lies between two points (sampled every 4 pixels).
fn clear_line(
    a: (i32, i32),
    b: (i32, i32),
    freq: u16,
    solid: &impl Fn(i32, i32, u16) -> bool,
) -> bool {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let steps = (dx.abs().max(dy.abs()) / 4).max(1);
    (0..=steps).all(|i| {
        let (x, y) = (a.0 + dx * i / steps, a.1 + dy * i / steps);
        !solid(x.div_euclid(16), y.div_euclid(16), freq)
    })
}

/// One tick of a free ball: each axis moves, and reverses on a wall (or stops, when
/// balls don't bounce); then friction.
fn step_ball(b: &mut Ball, bounce: bool, solid: &impl Fn(i32, i32, u16) -> bool) {
    if b.friction <= 0 {
        return;
    }
    for axis in 0..2 {
        let previous = (b.x, b.y);
        if axis == 0 {
            b.x += b.vx as i64;
        } else {
            b.y += b.vy as i64;
        }
        let (tx, ty) = (b.x.div_euclid(16_000) as i32, b.y.div_euclid(16_000) as i32);
        if solid(tx, ty, b.freq) {
            (b.x, b.y) = previous;
            if !bounce {
                (b.vx, b.vy) = (0, 0);
                break;
            }
            if axis == 0 {
                b.vx = -b.vx;
            } else {
                b.vy = -b.vy;
            }
        }
    }
    let f = b.friction / 1000;
    b.vx = b.vx * f / 1000;
    b.vy = b.vy * f / 1000;
    b.friction -= b.friction_delta;
}

/// Whether goal tile (x, y) belongs to team `freq` under `Soccer:Mode` (nullspace's
/// table: 0 any goal scores; 1 left/right halves; 2 top/bottom; 3/4 corners; 5/6
/// diagonal quarters).
pub fn is_team_goal(mode: u8, freq: u16, x: i32, y: i32) -> bool {
    let corner = |freq: u16| match freq % 4 {
        0 => x < 512 && y < 512,
        1 => x >= 512 && y < 512,
        2 => x < 512 && y >= 512,
        _ => x >= 512 && y >= 512,
    };
    let quarter = |freq: u16| match freq % 4 {
        0 if y < 512 => x < y,
        0 => x + y < 1024,
        1 if x < 512 => x + y >= 1024,
        1 => x < y,
        2 if x < 512 => x >= y,
        2 => x + y < 1024,
        _ if y <= 512 => x + y >= 1024,
        _ => x >= y,
    };
    match mode {
        0 => false,
        1 => (freq & 1 == 1) == (x >= 512),
        2 => (freq & 1 == 1) == (y >= 512),
        3 => corner(freq),
        4 => !corner(freq),
        5 => quarter(freq),
        6 => !quarter(freq),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> BallRules {
        BallRules {
            friction: [10; 8],
            proximity: [20; 8],
            speed: [2000; 8],
            throw_time: [300; 8],
            pass_delay: 50,
            mode: 1,
            allow_guns: false,
            allow_bombs: true,
            bounce: true,
            blank_delay: 0,
            no_wall_pass: false,
        }
    }

    fn me(x: i32, y: i32) -> BallPlayer {
        BallPlayer {
            pid: 1,
            ship: 0,
            freq: 0,
            x,
            y,
            vx: 0,
            vy: 0,
            direction: 10,
            can_play: true,
        }
    }

    fn report(id: u8, x: i16, y: i16, vx: i16, pid: i16, time: u32) -> BallPosition {
        BallPosition {
            ball_id: id,
            x,
            y,
            x_speed: vx,
            y_speed: 0,
            pid,
            time,
        }
    }

    const OPEN: fn(i32, i32, u16) -> bool = |_, _, _| false;
    const NO_GOAL: fn(i32, i32) -> bool = |_, _| false;

    #[test]
    fn free_ball_moves_slows_and_bounces() {
        let mut balls = Balls::default();
        let r = rules();
        // A wall at tile x = 101.
        let wall = |x: i32, _: i32, _: u16| x == 101;
        balls.report(
            &report(0, 1600, 800, 1000, 7, 100),
            100,
            &me(0, 0),
            &r,
            |_| Some((1, 2)),
            &wall,
        );
        let mut last = 1600;
        for t in 0..20 {
            balls.step(101 + t, &me(0, 0), &r, &wall, &NO_GOAL);
            let (x, _) = balls.balls[0].as_ref().unwrap().pixels();
            assert!(x <= 1615, "never enters the wall: {x}");
            last = x;
        }
        let b = balls.balls[0].as_ref().unwrap();
        assert!(b.vx < 0, "bounced back");
        assert!(b.vx.abs() < 1000, "slowed by friction");
        assert!(last > 1600);
    }

    #[test]
    fn pickup_waits_for_pass_delay_and_carry_then_fire() {
        let mut balls = Balls::default();
        let r = rules();
        let at = me(500, 500);
        balls.report(
            &report(2, 505, 505, 0, -1, 1000),
            1000,
            &at,
            &r,
            |_| None,
            &OPEN,
        );
        // Just fired (PassDelay 50): no pickup yet.
        assert!(balls.step(1010, &at, &r, &OPEN, &NO_GOAL).is_empty());
        let acts = balls.step(1060, &at, &r, &OPEN, &NO_GOAL);
        assert_eq!(acts, [BallAction::Pickup { id: 2, time: 1000 }]);
        assert!(
            balls.visible(1065, &r, |_| None).is_empty(),
            "hidden while asked"
        );
        // The server says we carry it.
        balls.report(
            &report(2, 0, 0, 0, 1, 0),
            1070,
            &at,
            &r,
            |_| Some((0, 0)),
            &OPEN,
        );
        assert_eq!(balls.carrying(), Some(2));
        // Carried ball is drawn in front (facing right: direction 10).
        let shown = balls.visible(1071, &r, |_| Some((500.0, 500.0, 10, 14)));
        assert_eq!(shown.len(), 1);
        assert!((shown[0].1.0 - 510.0).abs() < 0.5);
        // Bombs are allowed to fire normally; guns fire the ball.
        assert_eq!(balls.fire(&at, &r, false, true, 1080), None);
        let shot = balls.fire(&at, &r, true, false, 1080).unwrap();
        assert!(matches!(
            shot,
            BallAction::Fire {
                id: 2,
                vel: (2000, 0),
                ..
            }
        ));
        assert_eq!(balls.carrying(), None);
    }

    #[test]
    fn no_bounce_stops_and_stale_balls_cant_be_taken() {
        let wall = |x: i32, _: i32, _: u16| x == 101;
        let mut r = rules();
        r.bounce = false;
        let mut balls = Balls::default();
        balls.report(
            &report(0, 1600, 800, 1000, 7, 100),
            100,
            &me(0, 0),
            &r,
            |_| Some((1, 2)),
            &wall,
        );
        for t in 0..20 {
            balls.step(101 + t, &me(0, 0), &r, &wall, &NO_GOAL);
        }
        let b = balls.balls[0].as_ref().unwrap();
        assert_eq!((b.vx, b.vy), (0, 0), "stopped at the wall");

        // BallBlankDelay: no news for too long, no pickup.
        let mut r = rules();
        r.blank_delay = 200;
        let at = me(500, 500);
        let mut balls = Balls::default();
        balls.report(
            &report(1, 505, 505, 0, -1, 1000),
            1000,
            &at,
            &r,
            |_| None,
            &OPEN,
        );
        assert!(balls.step(1300, &at, &r, &OPEN, &NO_GOAL).is_empty());
        balls.report(
            &report(1, 505, 505, 0, -1, 1300),
            1300,
            &at,
            &r,
            |_| None,
            &OPEN,
        );
        assert_eq!(balls.step(1360, &at, &r, &OPEN, &NO_GOAL).len(), 1);

        // DisableBallThroughWalls: a wall tile between ship and ball blocks it.
        let mut r = rules();
        r.no_wall_pass = true;
        let wall_between = |x: i32, _: i32, _: u16| x == 31; // pixels 496..512
        let at = me(490, 500);
        let mut balls = Balls::default();
        balls.report(
            &report(2, 505, 500, 0, -1, 1000),
            1000,
            &at,
            &r,
            |_| None,
            &wall_between,
        );
        assert!(
            balls
                .step(1100, &at, &r, &wall_between, &NO_GOAL)
                .is_empty()
        );
        r.no_wall_pass = false;
        assert_eq!(balls.step(1300, &at, &r, &wall_between, &NO_GOAL).len(), 1);
    }

    #[test]
    fn rules_come_from_the_arena_settings() {
        let mut a = oss_sim::ArenaSettings::default();
        a.soccer.pass_delay = 33;
        a.soccer.ball_bounce = false;
        a.ships[3].soccer_ball_speed = 1234;
        let r = BallRules::from_arena(&a);
        assert_eq!((r.pass_delay, r.bounce, r.speed[3]), (33, false, 1234));
    }

    #[test]
    fn throw_timer_fires_backwards() {
        let mut balls = Balls::default();
        let r = rules();
        let at = me(0, 0);
        balls.report(
            &report(0, 0, 0, 0, 1, 0),
            100,
            &at,
            &r,
            |_| Some((0, 0)),
            &OPEN,
        );
        balls.report(
            &report(0, 0, 0, 0, 1, 0),
            100,
            &at,
            &r,
            |_| Some((0, 0)),
            &OPEN,
        );
        assert!(balls.step(399, &at, &r, &OPEN, &NO_GOAL).is_empty());
        let acts = balls.step(400, &at, &r, &OPEN, &NO_GOAL);
        assert!(matches!(
            acts[..],
            [BallAction::Fire {
                vel: (-2000, 0),
                ..
            }]
        ));
    }

    #[test]
    fn goals_only_on_enemy_side() {
        // Mode 1: team 0 owns the left half, team 1 the right.
        assert!(is_team_goal(1, 0, 100, 500));
        assert!(!is_team_goal(1, 0, 900, 500));
        assert!(is_team_goal(1, 1, 900, 500));
        assert!(!is_team_goal(0, 0, 100, 500), "mode 0: every goal scores");
        let mut balls = Balls::default();
        let r = rules();
        let at = me(0, 0);
        let goal = |x: i32, y: i32| (x, y) == (901, 50);
        // Our last shot rolls right into tile 901 (enemy goal for team 0).
        balls.report(
            &report(0, 900 * 16 + 8, 50 * 16 + 8, 3000, 1, 50),
            50,
            &at,
            &r,
            |_| Some((0, 0)),
            &OPEN,
        );
        let mut acts = Vec::new();
        for t in 51..120 {
            acts.extend(balls.step(t, &at, &r, &OPEN, &goal));
        }
        assert_eq!(
            acts,
            [BallAction::Goal {
                id: 0,
                x: 901,
                y: 50
            }]
        );
    }
}
