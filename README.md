# sgsae

An RV32IMC emulator core and a small fantasy console, with a handful of
cartridges and a way to move between them.

`bazel run //ui:vite` boots the supervisor, an index of the other programs.
Pick one and the console loads it — which makes this, in a small way, a browser
whose pages are programs. Press Escape in any of them to come back.

```
   arrows / WASD / gamepad     play
   enter / start               open the highlighted program
   escape / select             back to the launcher
   alt + enter                 full screen
   any key                     start sound
```

Rotate the lander with left and right, burn with up, and set it down flat and
slow on a pad. The narrow one pays five times the wide one.

```
bazel test //...                        # everything, including the real cartridges
cargo run --bin cli -- --cartridge bazel-bin/game/lander.elf --frames 600
bazel run //ui:vite                     # serve the wasm build
```

`//ui:e2e_test` drives the built page in headless Chrome. The dev container
installs Chrome for Testing and a matching chromedriver for it; outside the
container, put a Chrome on the `PATH` or set `CHROME_BIN` and `CHROMEDRIVER_BIN`
at specific binaries, which `.bazelrc` forwards into the test sandbox.

## Layout

| file | what it is |
|---|---|
| [`sgsae/instructions.rs`](sgsae/instructions.rs) | RV32IM decode/encode |
| [`sgsae/machine.rs`](sgsae/machine.rs) | the interpreter |
| [`sgsae/memory.rs`](sgsae/memory.rs) | the bus: ROM, RAM, and MMIO devices |
| [`sgsae/devices.rs`](sgsae/devices.rs) | text output |
| [`sgsae/input.rs`](sgsae/input.rs) | four gamepads and a keyboard |
| [`sgsae/loader.rs`](sgsae/loader.rs) | the cartridge slot |
| [`sgsae/midi.rs`](sgsae/midi.rs) | MIDI input, notes and clock |
| [`sgsae/compressed.rs`](sgsae/compressed.rs) | RV32C expansion |
| [`sgsae/elf.rs`](sgsae/elf.rs) | cartridge loader |
| [`sgsae/ppu.rs`](sgsae/ppu.rs) | paletted bitmap display |
| [`sgsae/apu.rs`](sgsae/apu.rs) | the sound chip: a four-channel PSG and sixteen FM voices |
| [`sgsae/console.rs`](sgsae/console.rs) | the console: one memory map, shared by the CLI and the web build |
| [`sdk/`](sdk/) | the SDK cartridges link against |

## The bus

The address space is carved into 16 MiB **slots** indexed by the top byte of an
address, so a load or store dispatches in one array lookup no matter how many
regions are mapped. A region may span several slots, but must start on a slot
boundary.

```rust
let mut bus = Bus::new();
bus.map_rom(0x0000_0000, cartridge)?;   // read-only; writes fault
bus.map_ram(0x0200_0000, 64 * 1024)?;
bus.map_device(0x0400_0000, console.clone())?;
```

Anything that is not plain memory implements `Device`. A device thinks in
bytes; the bus splits wider accesses for it, and a register that has to be
taken whole overrides `read` for that one case:

```rust
pub trait Device {
    fn size(&self) -> u32;
    fn read_byte(&mut self, offset: u32) -> u8;
    fn write_byte(&mut self, offset: u32, value: u8);
    // `read` and `write` are provided.
}
```

`read_byte` takes `&mut self`, so registers may have read side effects —
draining a FIFO, or clearing a latch of button edges as `PRESSED` does. Devices
never fault: as on real hardware, a write to a read-only register is ignored
and a gap reads as zero. The bus enforces the one limit that matters, the
device's declared size.

### Keeping a handle from the host

Devices are mapped as `Rc<RefCell<T>>`, which is how the host pushes input in
and drains output out without downcasting:

```rust
let input = Rc::new(RefCell::new(Input::default()));
bus.map_device(0x0500_0000, input.clone())?;

// …from a WebHID `inputreport` handler, or a keydown listener:
input.borrow_mut().set_key(code, true);
```

For bulk data — video RAM handed to the display — `Bus::bytes_at` borrows
memory from an address directly, with no copy. Instruction fetch reads through
the same path.

## Display

[`Ppu`](sgsae/ppu.rs) is a 240x160 paletted bitmap. Video RAM is plain memory
in its own slot, holding a palette and one byte per pixel indexing into it, and
`render` resolves the two into RGBA8888 — the layout a canvas `ImageData`
wants. The display is always on; it shows black until something is drawn.

```
video RAM at 0x0900_0000:
  0x0000 palette   256 x RGB555
  0x0200 VRAM      240 x 160 palette indices, row-major

registers at 0x0700_0000:
  0x0000 FRAME     u32, frames rendered (read-only)
```

Indirecting through a palette is what makes the cheap tricks of the era work:
a guest animates the whole screen by rewriting sixteen palette entries per
frame instead of forty thousand pixels.

Video RAM being memory rather than a device is what makes drawing cheap: a
store to it is the bus's fastest path, with no dispatch, and a game clears the
screen with word writes at four pixels a store.

The web build drives the machine from `requestAnimationFrame`, one guest frame
per animation frame. [`ui/render.js`](ui/render.js) uploads the framebuffer
straight from wasm memory to a WebGL texture — `texSubImage2D` takes the view
directly, so nothing is copied between Rust and JS — and a fullscreen triangle
magnifies it with `NEAREST` filtering. Canvas 2D is the fallback if WebGL is
missing.

At 240x160 this is not a speed decision: the whole blit costs about 0.01 ms
either way, against a 16.7 ms frame. What the shader buys is somewhere to put a
CRT treatment. Add `?crt` to the URL for scanlines and a vignette; without it
the fragment shader is a straight passthrough and the screen holds exactly the
palette entries the PPU drew, which the end-to-end test asserts.

## MIDI

[`sgsae/midi.rs`](sgsae/midi.rs) is an input port. Channel messages — notes,
controllers, pitch bend — queue in a FIFO the guest drains. Realtime messages do
not: a clock pulse arrives twenty-four times a beat and would swamp any queue,
so pulses are counted in a register and `TICKS` reports how many landed since it
was last read. A sequencer polls that once a frame and advances by the
difference, which locks it to an external device without needing interrupts.

```rust
let ticks = console.midi.take_ticks();          // pulses since last frame
for message in console.midi.messages() {
    if let Some((note, velocity)) = message.note_on() { /* … */ }
}
```

`note_on` folds away the note-on-with-zero-velocity convention, which is how
most keyboards actually release a note.

In the browser, [`ui/midi.js`](ui/midi.js) hands packets straight through — a
note, a clock pulse and a pitch bend all travel the same path, and the cartridge
decides what any of them mean. Messages are delivered as they land rather than
batched into the frame loop, because a clock that arrives late is a clock that
drifts. Web MIDI is a permission prompt and is absent in some browsers; failing
to get it just leaves the console running without an external clock.

## Keeping up

The emulator has to produce a frame of audio faster than a frame takes to play,
or the sound gaps. That is not a scheduling problem, it is a throughput one, and
it has exactly one cause worth knowing about: **an unoptimised interpreter is
about ten times slower**, which is not fast enough.

`//sgsae` and `//ui:app` therefore pin `-Copt-level=2` rather than following the
build's compilation mode, so a plain `bazel run //ui:vite` is playable. Measured
in the browser, on the three cartridges here:

| | unoptimised | optimised |
|---|---|---|
| lander | 48.8 fps, 18.2 ms/frame | 60.1 fps, 3.2 ms |
| fm seq | 36.8 fps, 24.6 ms | 60.1 fps, 3.7 ms |
| io test | 31.7 fps, 29.1 ms | 59.9 fps, 3.7 ms |

Drawing is not the cost — the blit is about 0.05 ms. It is all guest execution.

## Sound and the autoplay policy

Browsers will not start an `AudioContext` until the page has been interacted
with, so the console is silent until you press something. Two details of that
policy are worth knowing, because both are easy to get wrong:

- **Any gesture counts, so listen for any gesture.** These cartridges are
  played with the keyboard; waiting for a click means a player who never
  reaches for the mouse never hears anything. `keydown`, `pointerdown` and
  `touchstart` all resume the context, and the listeners stay attached, because
  switching tabs suspends it again.
- **`resume()` returns a promise that never settles while blocked** — not
  resolved, not rejected. Awaiting it hangs. Fire it and watch `statechange`.

A suspended context's `currentTime` is frozen too, so the playback schedule is
anchored when the context actually starts rather than when it was created.

YouTube is not exempt from any of this. Muted media may always autoplay, and
Chrome keeps a per-site [Media Engagement
Index](chrome://media-engagement) — watch enough sound on a site and it earns
the right to autoplay without a gesture. YouTube sits at the top of that list
for almost everyone, so it looks like an exception when it is really just a
site you have clicked on ten thousand times. A page nobody has visited gets
blocked exactly like this one.

For local development, `--autoplay-policy=no-user-gesture-required` turns it
off; `--autoplay-policy=document-user-activation-required` forces the strict
behaviour, which is what the end-to-end test runs against.

## Sound

[`Apu`](sgsae/apu.rs) is one sound chip with twenty voices behind one register
map. Voices 0..4 are a PSG — two pulse voices with selectable duty, a stepped
triangle, and an LFSR noise channel. Voices 4..20 are two-operator FM, each one
modulator driving one carrier, the arrangement the 16-bit consoles used where a
square-wave chip did the bleeps and an FM chip did the instruments:

```
output = sin(carrier + depth * sin(carrier * ratio))
```

The ratio decides the character and the depth decides how much of it you hear.
A whole-number ratio lands its sidebands on harmonics and sounds like an
instrument; anything else sounds like a bell. The depth follows the envelope,
so a note opens and closes rather than merely switching on.

Every voice has the same registers and the same attack/decay envelope; only
what `WAVE` and `DEPTH` mean differs by kind:

```
0x00 CTRL     bit 0: master enable
0x01 VOLUME   master volume, 0..=15
0x04 STATUS   u32, bit n: voice n is sounding (read-only)

0x08 + n*8    per voice n:
  +0 FREQ     u16, pitch in Hz (0 is silent)
  +2 VOL      voice volume, 0..=15
  +3 CTRL     bit 0: gate; bit 1: retrigger (strobe)
  +4 ATTACK   0 instant, larger swells in more slowly
  +5 DECAY    0 sustains, larger fades faster
  +6 WAVE     pulse: duty 0..=3; noise: 0 white, 1 tonal; FM: ratio in eighths
  +7 DEPTH    FM modulation depth; the PSG ignores it
```

Striking a note means writing `FREQ` then setting gate+retrigger, which restarts
the envelope. `Apu::render` fills a buffer with mono `f32` samples, the shape
Web Audio's `AudioWorkletProcessor` wants. To hear a cartridge:

```sh
cargo run --bin cli -- --cartridge bazel-bin/game/seq.elf --frames 600 --wav tune.wav
```

## Running the machine

`Machine::run` takes a cycle budget and returns why it stopped, so a host can
drive it from `requestAnimationFrame` without blocking the page:

```rust
match machine.run(CYCLES_PER_FRAME) {
    Ok(()) => {}                                       // budget spent, yield
    Err(Trap::EnvironmentCall { .. }) => service(&mut machine), // guest syscall
    Err(trap) => show_fault_screen(trap),
}
```

Guest misbehaviour never panics. A bad address, a misaligned jump, or an
unrecognised opcode all come back as a `Trap`, with `pc` left on the
instruction responsible. `ECALL` is a call rather than a fault, so it is the
one trap that retires: the host services it and simply runs again.

## The SDK

Cartridges link [`//sdk:console`](sdk/), which carries the reset vector, the
panic handler, and the linker script, and wraps the hardware in types. A game
takes the peripherals once at startup, the way embedded Rust does:

```rust
#![no_std]
#![no_main]
use console::{entry, input::Button, video::Rgb, Console};

fn main(mut console: Console) -> ! {
    console.video.set_palette(1, Rgb::new(31, 20, 0));
    console.audio.enable(12);
    loop {
        let pad = console.input.gamepad(0);
        if pad.is_held(Button::A) {
            console.video.clear(1);
        }
        console.wait_frame();
    }
}

entry!(main);
```

`video` has the palette, drawing primitives and a 3x5 digit font; `audio` hands
out [`Voice`](sdk/audio.rs) borrows — `channel(n)` for the PSG, `fm(n)` for FM —
with `play`, `set_gate` and envelope control; `input` covers gamepads and the
keyboard. Nothing in a game needs a raw
pointer or an address.

## Input

Four gamepads, each with a d-pad, six face and shoulder buttons, two analog
sticks and two analog triggers, plus a 256-key keyboard with modifiers. Both
report *held* state and a *pressed* latch that records rising edges and clears
when read, so a tap between two polls is still seen exactly once.

The page sends everything it can see: [`ui/input.js`](ui/input.js) maps
`event.code` onto the SDK's `Key` codes and polls the Gamepad API each frame.
Nothing in the browser decides what an input means — the game does:

```rust
thrust: pad.is_held(Button::Up)
    || pad.is_held(Button::A)
    || pad.triggers.1 > 96
    || stick.y < 0
    || input.is_key_held(Key::Up)
    || input.is_key_held(Key::W)
```

## The operating system

There is one, and it is three things.

**A supervisor.** [`game/loader.rs`](game/loader.rs) is an ordinary program —
same SDK, same linker script — that happens to be the one the console starts
with. The host keeps a copy, so when something else stops, the supervisor comes
back without anything being fetched.

**One register to start a program.** Write an address into the slot, poke
`CTRL`, and the host fetches it and restarts the machine with it in ROM:

```rust
console.loader.run("seq");   // never returns
```

**`ebreak` to end one.** That is the entire exit protocol, and it is why a
program pays nothing for any of this: no runtime to link, no API to call, one
instruction. A program that panics ends the same way, so a broken cartridge
returns to the menu instead of taking the console down.

```rust
console.exit();   // one instruction
```

The supervisor keeps no state between runs. Everything it needs on entry is in
the address slot and its status: a fresh boot with an address is a deep link, a
return means something stopped and the address says which, so the cursor goes
back to it. That slot is also how the page and the guest stay in step — the
host writes the boot address before anything runs, and reads it back to keep the
browser's address bar honest.

### Addresses

An address is a path with an optional prefix, defaulting to `system`, and no
extension:

```
seq            ->  /system/seq        page URL
                   /system/seq.elf    what the host fetches
```

Leaving the extension off the address is what lets the page live at
`/system/seq` without colliding with the file it names; any path the server has
no file for serves the app, which then reads the path. The supervisor matches
links on the last segment, so a prefix the host added does not confuse it.

The host decides what an address means. In the browser it is a `fetch`; in the
CLI, `--cartridge-dir` maps the last component onto a file:

```sh
bazel build //game:cartridges
cargo run --bin cli -- --supervisor bazel-bin/game/loader.elf \
                       --cartridge-dir bazel-bin/game --url /system/seq --frames 600
```

Programs are served from vite's `public/system/` rather than bundled as hashed
assets, so an address resolves to the same path after a build as in dev.

## The cartridges

| cartridge | what it is |
|---|---|
| [`loader.rs`](game/loader.rs) | the supervisor: an index of the rest |
| [`lander.rs`](game/lander.rs) | Lunar Lander |
| [`seq.rs`](game/seq.rs) | a sixteen-step sequencer driving the FM chip |
| [`io.rs`](game/io.rs) | every input the console can see, with DTMF tones |

[`io.rs`](game/io.rs) draws all four gamepads — buttons, sticks, triggers — a
256-cell grid of the keyboard, and a MIDI monitor, and sounds a touch-tone pair
for each press.

The monitor prints the raw status and data bytes of the last few messages
rather than an interpretation of them, which is the point: if a controller
sends something under a button and you do not know what, this is where you find
out. Clock pulses are deliberately kept out of the log — twenty-four a beat
would push everything else off it within a frame — and counted in `CLK` and
`BEAT` instead.
The tones are the real thing: a keypad sends two frequencies at once, one for
the row and one for the column, and there are sixteen codes — exactly as many as
this console has gamepad buttons. Two pulse channels stand in for the sines.

## The game

[`game/lander.rs`](game/lander.rs) is a complete cartridge: fixed-point physics
in Q8 (one pixel is 256 units, so no floating point anywhere), a generated ridge
line with a landing shelf cut into it, and noise-channel thrust. It runs about
70k instructions a frame — roughly a seventh of the budget at 60 fps.

The page fetches `cartridge.elf` and hands the bytes to the emulator, the way a
console reads a cart rather than having the game baked in.

## Building a cartridge

Guest code lives in [`game/`](game/) and is built by Bazel for a different
platform than everything else:

```sh
bazel build //game:cartridge     # -> bazel-bin/game/cartridge.elf
bazel test //game:cartridge_test # build it, then run it on the emulator
bazel run //ui:vite              # play it

# Or run it headless and look at a frame:
cargo run --bin cli -- --cartridge bazel-bin/game/cartridge.elf \
                       --frames 240 --screenshot frame.ppm
```

The `cartridge` rule in [`game/cartridge.bzl`](game/cartridge.bzl) carries an
outgoing [transition](https://bazel.build/extending/config) to
[`//platforms:rv32`](platforms/BUILD.bazel), so nobody has to remember
`--platforms` on the command line — the emulator, CLI and web bundle still build
for the host in the same invocation.

The reset vector and the panic handler live in the SDK, so a cartridge is just
`#![no_std] #![no_main]` plus `entry!`. [`sdk/console.ld`](sdk/console.ld)
places the sections, and its `MEMORY` block must agree with `ROM_SIZE` and
`RAM_SIZE` in `sgsae/console.rs`.

### On the compressed extension

The only bare-metal RISC-V triples `rules_rust` offers are `riscv32imc` and
`riscv32imac` — both include `c`, and `-Ctarget-feature=-c` does not remove it,
because `c` is part of those targets' ISA spec. A toolchain-built cartridge is
roughly two thirds compressed instructions, so the machine implements RV32C in
[`sgsae/compressed.rs`](sgsae/compressed.rs) rather than fighting the toolchain.

Note that `llvm-objdump` prints compressed instructions under their expanded
mnemonics, so grepping for `c.` will tell you there are none. Compare encoding
widths instead.

### C and C++

The same linker script works with clang, which *does* let you pick a plain
`rv32im`:

```sh
clang --target=riscv32-unknown-elf -march=rv32im -mabi=ilp32 \
      -ffreestanding -nostdlib -c game.c -o game.o
ld.lld -T game/console.ld game.o -o game.elf
```

For C++ add `-fno-exceptions -fno-rtti` and supply `memcpy`, `memset`, and
`__cxa_pure_virtual`.

## Loading

[`elf::load`](sgsae/elf.rs) places `PT_LOAD` segments and returns the entry
point. It writes through [`Bus::load`](sgsae/memory.rs), which bypasses write
protection — a cartridge's code belongs in ROM and something has to put it
there. There is no dynamic linking or relocation processing.

## Not implemented

No A, F, or D extensions. No CSRs (any access traps). No interrupts, no MMU.
The PPU is a bitmap only — no tiles, tilemaps, scrolling, or sprites, so the
guest draws every pixel itself. No save RAM.
