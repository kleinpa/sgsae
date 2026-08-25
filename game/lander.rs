//! Lunar Lander.
//!
//! A vector-graphics lander over randomly generated terrain. Rotate the ship
//! to aim its engine, burn to kill your descent, and set down flat and slow on
//! one of the two pads. The narrow one is worth more.
//!
//! ```text
//!   up / W / A / pad up      main engine, along whichever way the nose points
//!   left / right / A / D     rotate
//!   enter / start            play again
//!   escape / select          back to the loader
//! ```
//!
//! Everything is fixed point and integer trigonometry from [`console::fixed`]:
//! the ship's heading is an [`Angle`], and thrust is that angle's unit vector
//! scaled by the engine's power, so there is no special case for "which way is
//! up". Drawing is [`console::gfx`]: each model is a `const` list of points in
//! ROM, placed by a [`Transform`].

#![no_std]
#![no_main]

use console::audio::Duty;
use console::fixed::{Angle, Fx, Vec2};
use console::gfx::{self, Shape, Transform};
use console::input::{Button, Key, Stick};
use console::video::{Rgb, Video, HEIGHT, WIDTH};
use console::{entry, Console};

// Monochrome: one ink, one ground, and a dim shade for what is only context.
// Keeping the whole game to three pens is what makes it read as a vector
// display rather than as a painting with outlines.
const BACKGROUND: u8 = 0;
const INK: u8 = 1;
const DIM: u8 = 2;
const WARN: u8 = 3;
/// The exhaust plume. The same hue as [`WARN`] but its own entry, so that what
/// the engine is doing can be told apart from what the instruments are saying —
/// by a test reading the screen, and by a player glancing at it.
const EXHAUST: u8 = 4;

const PALETTE: [Rgb; 5] = [
    Rgb::new(0, 1, 1),
    Rgb::new(24, 31, 26),
    Rgb::new(8, 13, 10),
    Rgb::new(31, 14, 6),
    Rgb::new(31, 20, 2),
];

/// Downward pull, per frame, in fixed point.
const GRAVITY: Fx = Fx::ratio(3, 1000);
/// Engine power, per frame.
const THRUST: Fx = Fx::ratio(11, 1000);
/// How fast the ship turns, in angle units per frame — about 1.3 degrees, so
/// a quarter turn takes a little over a second. Fast enough to correct a bad
/// approach, slow enough that pointing the engine is a decision.
const TURN_RATE: i32 = 240;

const START_FUEL: i32 = 1500;

/// The gentlest touchdown that still counts as a crash.
const SAFE_SPEED: Fx = Fx::ratio(52, 100);
/// How far off vertical the ship may be and still land, either way.
const SAFE_TILT: i32 = Angle::from_degrees(17).0 as i32;

/// The ship, nose up, about the point it rotates around.
///
/// An octagonal crew module over a narrower engine housing: eight points is
/// enough to read as a capsule rather than as a letter, and the waist is what
/// separates the two halves at this size.
const SHIP: Shape = Shape::closed(&[
    Vec2::int(-2, -6),
    Vec2::int(2, -6),
    Vec2::int(4, -3),
    Vec2::int(4, 0),
    Vec2::int(2, 2),
    Vec2::int(-2, 2),
    Vec2::int(-4, 0),
    Vec2::int(-4, -3),
]);

/// The engine bell and the legs, which are separate strokes from the hull.
///
/// The legs start at the hull's widest point rather than at its base, so the
/// strut is visible along its whole length instead of disappearing behind the
/// bell — at this size a leg that is only its own footpad reads as a stray
/// pixel.
const LEGS: [(Vec2, Vec2); 7] = [
    // The bell, under the waist.
    (Vec2::int(-2, 2), Vec2::int(-2, 4)),
    (Vec2::int(2, 2), Vec2::int(2, 4)),
    (Vec2::int(-2, 4), Vec2::int(2, 4)),
    // The struts, from the hull's shoulders out to the footpads.
    (Vec2::int(-4, 0), Vec2::int(-6, 6)),
    (Vec2::int(4, 0), Vec2::int(6, 6)),
    // The footpads themselves.
    (Vec2::int(-7, 6), Vec2::int(-5, 6)),
    (Vec2::int(5, 6), Vec2::int(7, 6)),
];

/// The exhaust plume, pointing out of the engine bell.
const FLAME: Shape = Shape::open(&[Vec2::int(-2, 5), Vec2::int(0, 11), Vec2::int(2, 5)]);

/// A shorter plume, alternated with the long one so the flame flickers.
const FLAME_SHORT: Shape = Shape::open(&[Vec2::int(-2, 5), Vec2::int(0, 8), Vec2::int(2, 5)]);

/// Debris, drawn scattering from where the ship stopped.
const DEBRIS: [(Vec2, Vec2); 4] = [
    (Vec2::int(-6, 2), Vec2::int(-2, 4)),
    (Vec2::int(2, 4), Vec2::int(7, 1)),
    (Vec2::int(-3, 5), Vec2::int(1, 6)),
    (Vec2::int(0, 3), Vec2::int(3, -2)),
];

/// How far the footpads sit either side of the ship's centre, and how far
/// below it they reach. Taken from [`LEGS`]: the physics and the picture have
/// to agree about where the craft actually touches, or it lands on air.
const FOOT_X: i32 = 6;
const FOOT_Y: i32 = 6;

/// Terrain is stored as the height of every column, which is what makes
/// collision a lookup rather than a search.
const COLUMNS: usize = WIDTH as usize;

/// The two pads, as a column range and what landing there is worth.
#[derive(Clone, Copy)]
struct Pad {
    left: i32,
    width: i32,
    /// The score multiplier, shown on the pad itself.
    bonus: u32,
}

impl Pad {
    fn right(&self) -> i32 {
        self.left + self.width
    }

    fn contains(&self, x: i32) -> bool {
        x >= self.left && x < self.right()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Flying,
    Landed,
    Crashed,
}

struct Game {
    /// The first solid row of each column, so smaller is higher up.
    terrain: [u8; COLUMNS],
    stars: [(i32, i32); 20],
    /// The wide, forgiving pad and the narrow one worth more.
    pads: [Pad; 2],
    position: Vec2,
    velocity: Vec2,
    heading: Angle,
    fuel: i32,
    state: State,
    /// Frames since the round ended, so the result has a moment to be read.
    settled: u32,
    score: u32,
    /// Which pad was earned, for the result line.
    landed_bonus: u32,
    rng: Rng,
}

/// A small linear congruential generator. Seeded from the frame counter at
/// startup, so the world differs run to run, but reproducible once seeded.
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        self.0 >> 16
    }

    /// A number in `0..bound`.
    fn below(&mut self, bound: u32) -> u32 {
        if bound == 0 { 0 } else { self.next() % bound }
    }

    /// A number in `low..=high`.
    fn between(&mut self, low: i32, high: i32) -> i32 {
        low + self.below((high - low + 1).max(1) as u32) as i32
    }
}

impl Game {
    fn new(seed: u32) -> Game {
        let mut game = Game {
            terrain: [0; COLUMNS],
            stars: [(0, 0); 20],
            pads: [Pad { left: 0, width: 0, bonus: 1 }; 2],
            position: Vec2::ZERO,
            velocity: Vec2::ZERO,
            heading: Angle::ZERO,
            fuel: 0,
            state: State::Flying,
            settled: 0,
            score: 0,
            landed_bonus: 0,
            rng: Rng(seed | 1),
        };
        game.generate();
        game.reset();
        game
    }

    /// Build a fresh world: a ridge line with two level shelves cut into it.
    ///
    /// The profile is straight runs between randomly placed peaks rather than
    /// smooth noise, because that angular silhouette is what a vector cabinet
    /// of this era actually drew.
    fn generate(&mut self) {
        for i in 0..self.stars.len() {
            let x = self.rng.below(WIDTH as u32) as i32;
            let y = self.rng.below(70) as i32 + 4;
            self.stars[i] = (x, y);
        }

        // Peaks every SPAN columns, joined by straight runs.
        const SEGMENTS: usize = 14;
        const SPAN: i32 = WIDTH / SEGMENTS as i32;
        let mut peaks = [0i32; SEGMENTS + 2];
        for peak in peaks.iter_mut() {
            *peak = self.rng.between(96, 150);
        }
        for x in 0..WIDTH {
            let segment = (x / SPAN) as usize;
            let t = x - segment as i32 * SPAN;
            let (a, b) = (peaks[segment], peaks[segment + 1]);
            self.terrain[x as usize] = (a + (b - a) * t / SPAN) as u8;
        }

        // Two pads: a wide one and a narrow one worth more. Placing them in
        // separate halves and keeping a gap between guarantees they never
        // overlap however the widths fall out, which is the only way a
        // generated world can be trusted not to need a retry.
        // The narrow pad has to be wider than the ship's stance with something
        // to spare, or it is not a hard landing but an impossible one.
        let wide_width = self.rng.between(26, 34);
        let narrow_width = self.rng.between(FOOT_X * 2 + 4, FOOT_X * 2 + 8);
        let wide_left = self.rng.between(12, WIDTH / 2 - wide_width - 12);
        let narrow_left = self.rng.between(WIDTH / 2 + 10, WIDTH - narrow_width - 12);

        // Which half each lands in is itself a coin toss, so the small pad is
        // not always on the right.
        let (wide, narrow) = if self.rng.below(2) == 0 {
            (wide_left, narrow_left)
        } else {
            // Mirror both, keeping each inside its own half.
            (WIDTH - wide_left - wide_width, WIDTH - narrow_left - narrow_width)
        };

        self.pads = [
            Pad { left: wide, width: wide_width, bonus: 1 },
            Pad { left: narrow, width: narrow_width, bonus: 5 },
        ];

        for index in 0..self.pads.len() {
            let pad = self.pads[index];
            // Sit the shelf at the average height of the ground it replaces,
            // so it belongs to the landscape rather than being stamped on top
            // of it.
            let mut total = 0;
            for x in pad.left..pad.right() {
                total += self.terrain[x as usize] as i32;
            }
            let shelf = (total / pad.width).clamp(104, 148);
            for x in pad.left..pad.right() {
                self.terrain[x as usize] = shelf as u8;
            }

            // Ramp the ground back to the ridge either side. Without this the
            // clamp can leave the shelf metres above what it adjoins, and the
            // pad ends up on a pillar with its posts hanging in mid-air.
            const RAMP: i32 = 10;
            for step in 1..=RAMP {
                let blend = |ground: i32| (shelf * (RAMP - step) + ground * step) / RAMP;
                let before = pad.left - step;
                if before >= 0 {
                    self.terrain[before as usize] = blend(self.terrain[before as usize] as i32) as u8;
                }
                let after = pad.right() - 1 + step;
                if after < WIDTH {
                    self.terrain[after as usize] = blend(self.terrain[after as usize] as i32) as u8;
                }
            }
        }
    }

    /// Put the ship back at the top with a fresh tank, over open ground.
    fn reset(&mut self) {
        // Start high but clear of the HUD, drifting gently, so the player
        // always has to fly somewhere to land.
        let x = self.rng.between(40, WIDTH - 40);
        self.position = Vec2::new(Fx::from_int(x), Fx::from_int(34));
        self.velocity = Vec2::new(Fx::ratio(self.rng.between(-30, 30), 100), Fx::ratio(6, 100));
        self.heading = Angle::ZERO;
        self.fuel = START_FUEL;
        self.state = State::Flying;
        self.settled = 0;
        self.landed_bonus = 0;
    }

    /// A whole new world, and a fresh ship on it.
    fn regenerate(&mut self) {
        self.generate();
        self.reset();
    }

    /// Fold what the player is doing into the generator's state.
    ///
    /// A console with no clock boots identically every time, so this is where
    /// unpredictability actually comes from: how long someone held the engine
    /// on, when they turned. Only advancing on input keeps a run that touches
    /// nothing reproducible, which is what the tests rely on.
    fn stir(&mut self, controls: &Controls) {
        if controls.thrust || controls.left || controls.right {
            self.rng.next();
        }
    }

    fn ground_at(&self, x: i32) -> i32 {
        self.terrain[x.clamp(0, WIDTH - 1) as usize] as i32
    }

    /// The pad under `x`, if any.
    fn pad_at(&self, x: i32) -> Option<&Pad> {
        self.pads.iter().find(|pad| pad.contains(x))
    }

    /// How far from upright the ship is, in angle units, either way.
    fn tilt(&self) -> i32 {
        self.heading.delta(Angle::ZERO).abs()
    }

    fn update(&mut self, controls: Controls, console: &mut Console) {
        if self.state != State::Flying {
            self.settled += 1;
            if self.settled > 40 && controls.restart {
                // A crash costs you the world; a landing earns a new one.
                self.regenerate();
            }
            return;
        }

        if controls.left {
            self.heading = self.heading.wrapping_add(-TURN_RATE);
        }
        if controls.right {
            self.heading = self.heading.wrapping_add(TURN_RATE);
        }

        // Thrust goes wherever the nose points. `Angle::UP` is the model's
        // nose direction, so rotating it by the heading is the whole of it —
        // no cases, no sign fixing.
        let burning = controls.thrust && self.fuel > 0;
        if burning {
            let along = (Angle::UP + self.heading).unit();
            self.velocity += along.scale(THRUST);
            self.fuel -= 1;
        }
        console.audio.channel(3).set_gate(burning);

        self.velocity.y += GRAVITY;
        self.position += self.velocity;

        // The sides are walls: bleed off the speed rather than wrapping, so
        // the playfield is somewhere with edges rather than a torus.
        let limit = Fx::from_int(WIDTH - 1);
        if self.position.x < Fx::ZERO {
            self.position.x = Fx::ZERO;
            self.velocity.x = -self.velocity.x / 2;
        } else if self.position.x > limit {
            self.position.x = limit;
            self.velocity.x = -self.velocity.x / 2;
        }
        if self.position.y < Fx::ZERO {
            self.position.y = Fx::ZERO;
            self.velocity.y = Fx::ZERO;
        }

        self.settle(console);
    }

    /// Test the legs against the ground and decide the round if they touch.
    fn settle(&mut self, console: &mut Console) {
        let x = self.position.x.round();
        let feet = self.position.y.round() + FOOT_Y;

        // The ground under either leg, whichever is higher: landing across a
        // slope should catch on the upslope side.
        let ground = self
            .ground_at(x - FOOT_X)
            .min(self.ground_at(x + FOOT_X))
            .min(self.ground_at(x));
        if feet < ground {
            return;
        }

        self.position.y = Fx::from_int(ground - FOOT_Y);
        console.audio.channel(3).set_gate(false);

        let speed = self.velocity.length();
        let gentle = speed < SAFE_SPEED && self.tilt() < SAFE_TILT;
        // Both legs have to be over the same pad, or it is not a landing.
        let pad = match (self.pad_at(x - FOOT_X), self.pad_at(x + FOOT_X)) {
            (Some(left), Some(right)) if left.left == right.left => Some(*left),
            _ => None,
        };

        match pad {
            Some(pad) if gentle => {
                self.state = State::Landed;
                self.landed_bonus = pad.bonus;
                self.score += pad.bonus;
                console.audio.channel(0).play(523);
                console.print("landed\n");
            }
            _ => {
                self.state = State::Crashed;
                let mut noise = console.audio.channel(3);
                noise.set_volume(15).set_decay(3);
                noise.play(200);
                console.print("crashed\n");
            }
        }
        self.velocity = Vec2::ZERO;
    }

    fn render(&self, controls: Controls, console: &mut Console) {
        let video = &mut console.video;
        video.clear(BACKGROUND);

        for (x, y) in self.stars {
            video.pixel(x, y, DIM);
        }

        self.draw_terrain(video);
        self.draw_ship(controls, video);
        self.draw_hud(video);
    }

    /// The surface, as the single stroke a vector display would draw.
    ///
    /// Only the outline is drawn — nothing is filled — so the ground reads as
    /// a horizon rather than as a solid mass, which is the whole look.
    fn draw_terrain(&self, video: &mut Video) {
        let mut previous = (0, self.ground_at(0));
        for x in 1..WIDTH {
            let next = (x, self.ground_at(x));
            video.line(previous.0, previous.1, next.0, next.1, INK);
            previous = next;
        }

        // The pads, marked with a bar and end posts so they stand out from the
        // ridge they are cut into, and their bonus written beside them.
        for pad in &self.pads {
            let y = self.ground_at(pad.left);
            video.hline(pad.left, y, pad.width, INK);
            video.hline(pad.left, y + 1, pad.width, INK);
            video.vline(pad.left, y - 4, 4, INK);
            video.vline(pad.right() - 1, y - 4, 4, INK);
            if pad.bonus > 1 {
                // Only the pad that pays extra is labelled; the wide one needs
                // no explanation. Above the surface, where there is sky rather
                // than ground to draw over.
                video.text(pad.left + pad.width / 2 - 3, y - 11, "X5", DIM);
            }
        }
    }

    fn draw_ship(&self, controls: Controls, video: &mut Video) {
        let placed = Transform::at(self.position).turned(self.heading);

        if self.state == State::Crashed {
            // The wreck, scattered where it stopped, unrotated: whatever
            // attitude it hit at, the debris lies on the ground.
            gfx::segments(video, &Transform::at(self.position), &DEBRIS, WARN);
            return;
        }

        SHIP.draw(video, &placed, INK);
        gfx::segments(video, &placed, &LEGS, INK);

        // The plume, drawn in the ship's own space so it always leaves the
        // engine bell whichever way the ship is facing.
        if self.state == State::Flying && controls.thrust && self.fuel > 0 {
            let plume = if self.fuel & 2 == 0 { FLAME } else { FLAME_SHORT };
            plume.draw(video, &placed, EXHAUST);
        }
    }

    fn draw_hud(&self, video: &mut Video) {
        // Fuel, as a bar that drains.
        const FULL: i32 = 52;
        let remaining = self.fuel * FULL / START_FUEL;
        let low = self.fuel * 4 < START_FUEL;
        video.outline(4, 4, FULL + 2, 7, if low { WARN } else { DIM });
        video.rect(5, 5, remaining, 5, if low { WARN } else { INK });

        // Speed and tilt, the two numbers that decide a landing. Both are
        // drawn in the warning colour once they are outside what will survive,
        // which is the whole instrument panel: you can fly by the colour.
        let speed = self.velocity.length();
        let speed_shown = (speed.raw() * 100 >> 16).clamp(0, 99) as u32;
        let unsafe_speed = speed >= SAFE_SPEED;
        video.text(4, 14, "VEL", DIM);
        video.number(20, 14, speed_shown, 2, if unsafe_speed { WARN } else { INK });

        let tilted = self.tilt() >= SAFE_TILT;
        video.text(4, 21, "TLT", DIM);
        let degrees = (self.tilt() * 360 / 65536).clamp(0, 99) as u32;
        video.number(20, 21, degrees, 2, if tilted { WARN } else { INK });

        // A horizon indicator: a short bar rolled the way the ship is, so the
        // attitude is readable without looking away from the ship.
        let centre = Vec2::int(WIDTH - 20, 10);
        let arm = Vec2::int(7, 0);
        let level = Transform::at(centre).turned(self.heading);
        let (lx, ly) = level.pixel(-arm);
        let (rx, ry) = level.pixel(arm);
        video.line(lx, ly, rx, ry, if tilted { WARN } else { INK });
        video.pixel(WIDTH - 20, 10, DIM);

        video.text(WIDTH / 2 - 14, 4, "SCORE", DIM);
        video.number(WIDTH / 2 + 8, 4, self.score.min(999), 3, INK);

        if self.state != State::Flying {
            self.draw_result(video);
        }
    }

    /// The result, in words, once the round is over.
    fn draw_result(&self, video: &mut Video) {
        let (message, colour) = match self.state {
            State::Landed if self.landed_bonus > 1 => ("PERFECT LANDING", INK),
            State::Landed => ("LANDED", INK),
            _ => ("CRASHED", WARN),
        };
        let width = Video::text_width(message);
        let x = WIDTH / 2 - width / 2;
        let y = HEIGHT / 2 - 12;

        // A ruled box, so the text sits on the picture rather than in it.
        video.rect(x - 6, y - 5, width + 12, 24, BACKGROUND);
        video.outline(x - 6, y - 5, width + 12, 24, colour);
        video.text(x, y, message, colour);

        if self.settled > 40 {
            const AGAIN: &str = "PRESS START";
            video.text(WIDTH / 2 - Video::text_width(AGAIN) / 2, y + 10, AGAIN, DIM);
        }
    }
}

/// Below this the left stick reads as centred.
const STICK_DEADZONE: i8 = 40;

/// What the player is asking for this frame, however they asked for it.
#[derive(Debug, Default, Clone, Copy)]
struct Controls {
    thrust: bool,
    left: bool,
    right: bool,
    restart: bool,
    quit: bool,
}

impl Controls {
    fn read(input: &mut console::input::Input) -> Controls {
        let pad = input.gamepad(0);
        let stick: Stick = pad.left.with_deadzone(STICK_DEADZONE);
        // One snapshot: reading the latch clears it, and Enter and Escape
        // share a byte.
        let pressed = input.keys_pressed();

        Controls {
            thrust: pad.is_held(Button::Up)
                || pad.is_held(Button::A)
                || pad.triggers.1 > 96
                || stick.y < 0
                || input.is_key_held(Key::Up)
                || input.is_key_held(Key::W)
                || input.is_key_held(Key::Space),
            left: pad.is_held(Button::Left)
                || stick.x < 0
                || input.is_key_held(Key::Left)
                || input.is_key_held(Key::A),
            right: pad.is_held(Button::Right)
                || stick.x > 0
                || input.is_key_held(Key::Right)
                || input.is_key_held(Key::D),
            restart: pad.was_pressed(Button::Start)
                || pad.was_pressed(Button::A)
                || pressed.contains(Key::Enter),
            quit: pad.was_pressed(Button::Select) || pressed.contains(Key::Escape),
        }
    }
}

fn main(mut console: Console) -> ! {
    console.print("lunar lander\n");

    console.video.set_palettes(0, &PALETTE);

    console.audio.enable(12);
    // Channel 3 is the noise voice: the engine holds its gate open, so it
    // sustains rather than decaying.
    console.audio.channel(3).set_volume(7).set_tonal_noise(false).set_decay(0).set_pitch(1400);
    console.audio.channel(0).set_volume(13).set_duty(Duty::Half).set_decay(4);

    // The console has no clock and boots into a known state, so the first
    // world is necessarily the same every time — which is what makes the game
    // testable. Every world after it is stirred by how the player flew the
    // last one, which is where the variety comes from.
    let mut game = Game::new(0x5EED_1EA5);
    loop {
        let controls = Controls::read(&mut console.input);
        if controls.quit {
            console.exit();
        }
        game.stir(&controls);
        game.update(controls, &mut console);
        game.render(controls, &mut console);
        console.wait_frame();
    }
}

entry!(main);
