// rules_rust_wasm_bindgen emits its outputs into a directory named after the
// target, so the module lives at app_bindgen/app_bindgen.js.
import init, { Demo } from "./app_bindgen/app_bindgen.js";
import { createRenderer } from "./render.js";
import { attachInput } from "./input.js";
import { attachMidi } from "./midi.js";

/// The largest whole-number magnification that fits the window, so the picture
/// stays on the pixel grid rather than being resampled onto it.
function fitScale(demo) {
  const fit = Math.min(
    Math.floor(window.innerWidth / demo.width()),
    Math.floor(window.innerHeight / demo.height()),
  );
  return Math.max(1, Math.min(fit, 8));
}

/// Where programs live when an address does not say. One day there might be
/// others; today everything ships together.
const DEFAULT_PREFIX = "system";

function build(demo) {
  const screen = document.createElement("canvas");
  screen.id = "screen";
  screen.style.display = "block";
  screen.style.background = "#000";
  document.body.appendChild(screen);

  const renderer = createRenderer(screen, demo.width(), demo.height(), { scale: fitScale(demo) });
  screen.dataset.renderer = renderer.kind;

  const output = document.createElement("pre");
  output.id = "emulator-output";
  document.body.appendChild(output);

  return { screen, renderer, output };
}

/**
 * Play frames of audio back to back, and say when more is needed.
 *
 * The guest produces one frame's worth of samples at a time and never stops,
 * so rather than collecting a buffer we schedule each frame at the moment the
 * previous one ends. The lead keeps a little slack so a late frame does not
 * leave an audible hole; it is also the whole of the latency between a note
 * being played and being heard, so it is kept short.
 *
 * Browsers will not start an AudioContext until the user has interacted with
 * the page, so the context is built up front and resumed on the first gesture
 * of any kind. Any kind matters: these cartridges are played with the
 * keyboard, and listening only for a click means a player who never reaches
 * for the mouse never hears anything.
 */
function speaker(sampleRate, onState) {
  const LEAD = 0.02;
  // Suspended until a gesture, but existing already means the first note does
  // not have to wait for the device to open.
  const context = new AudioContext({ sampleRate, latencyHint: "interactive" });
  let next = 0;

  const unlock = () => {
    if (context.state === "running") return;
    // While the policy is blocking, resume()'s promise never settles at all —
    // not resolved, not rejected. Fire it and watch statechange instead.
    context.resume().catch(() => {});
  };

  // Not `once`: switching tabs suspends the context again, and the next
  // keystroke should bring it back without the player noticing.
  for (const type of ["keydown", "pointerdown", "touchstart"]) {
    addEventListener(type, unlock, { capture: true, passive: true });
  }

  context.addEventListener("statechange", () => {
    // A suspended context's clock is frozen, so the schedule can only be
    // anchored once it is really running.
    if (context.state === "running") next = context.currentTime + LEAD;
    onState?.(context.state);
  });
  onState?.(context.state);

  const play = (samples) => {
    if (context.state !== "running" || samples.length === 0) return;
    // If we have fallen behind, skip forward rather than queueing the past.
    if (next < context.currentTime) next = context.currentTime + LEAD;

    const buffer = context.createBuffer(1, samples.length, sampleRate);
    buffer.copyToChannel(samples, 0);
    const source = context.createBufferSource();
    source.buffer = buffer;
    source.connect(context.destination);
    source.start(next);
    next += samples.length / sampleRate;
  };
  // Whether playback is about to run out of scheduled sound. While the
  // context is not running there is no clock to be behind, so one frame per
  // call keeps the picture moving.
  play.starved = () => context.state !== "running" || next - context.currentTime < LEAD;
  // How many frames to run at once when catching up.
  play.burst = () => (context.state === "running" ? 4 : 1);
  return play;
}

async function main() {
  const wasm = await init();

  // An address is a path: `/system/seq`. A bare name gets the default prefix,
  // so the supervisor can say `seq` and never think about where programs live.
  // The page shows the address as its own URL, and fetches the same path with
  // `.elf` on the end — which is why the address itself has no extension, and
  // so cannot collide with the file it names.
  const address = (name) => {
    if (!name || name === "/") return null;
    const path = name.startsWith("/") ? name : `/${DEFAULT_PREFIX}/${name}`;
    return path.split("/").length > 2 ? path : `/${DEFAULT_PREFIX}${path}`;
  };

  const fetchProgram = async (path) =>
    new Uint8Array(await fetch(`${path}.elf`).then((r) => {
      if (!r.ok) throw new Error(`${path}: ${r.status}`);
      return r.arrayBuffer();
    }));

  // The supervisor is the only thing the page loads directly. Everything else
  // is its business, including where a deep link should go: the address goes
  // into the slot and the supervisor finds it already there.
  const demo = new Demo(await fetchProgram(`/${DEFAULT_PREFIX}/loader`));
  // The page's own path is the deep link: opening /system/seq goes straight
  // there, and the server serves this page for any path it has no file for.
  const deepLink = address(location.pathname);
  if (deepLink) demo.setBootUrl(deepLink);
  const { renderer, output } = build(demo);
  const pump = attachInput();
  const play = speaker(demo.sampleRate(), (state) => {
    document.body.dataset.audio = state;
  });
  // MIDI is optional: a browser without it, or a refused permission, just
  // leaves the console running without an external clock.
  attachMidi(demo).catch((error) => console.info("MIDI unavailable:", error));

  // Keep the page's address bar and the console's slot saying the same thing,
  // so reload and the back button land where you were.
  const showAddress = (path, { push = false } = {}) => {
    const url = path ?? "/";
    const state = { cart: path ?? null };
    if (push) history.pushState(state, "", url);
    else history.replaceState(state, "", url);
    document.body.dataset.cart = path ?? "";
  };
  showAddress(deepLink);

  const open = (path) => {
    output.textContent = `loading ${path}\u2026`;
    return fetchProgram(path).then((bytes) => {
      demo.loadProgram(bytes);
      output.textContent = "";
    });
  };

  // The back button leaves a program the way closing a tab would: the program
  // does not get a say, any more than a web page does.
  addEventListener("popstate", (event) => {
    const wanted = event.state?.cart ?? address(location.pathname);
    const current = document.body.dataset.cart || null;
    if (wanted === current) return;
    if (wanted === null) {
      demo.returnToSupervisor();
      showAddress(null);
    } else {
      open(wanted).then(() => showAddress(wanted)).catch((error) => {
        console.warn(error);
        demo.reportLoadFailure();
        showAddress(null);
      });
    }
  });

  // Alt+Enter toggles full screen. Handled in the capture phase and stopped
  // there, so the guest never sees the keystroke that resized its window.
  addEventListener(
    "keydown",
    (event) => {
      if (!event.altKey || event.code !== "Enter") return;
      event.preventDefault();
      event.stopPropagation();
      if (document.fullscreenElement) document.exitFullscreen();
      else document.documentElement.requestFullscreen().catch(() => {});
    },
    { capture: true },
  );

  const rescale = () => renderer.setScale(fitScale(demo));
  addEventListener("resize", rescale);
  addEventListener("fullscreenchange", rescale);

  const tick = () => {
    pump(demo);
    // The audio clock decides how fast the guest runs, not the display's
    // refresh rate: run frames until playback has enough sound in hand. A 120
    // Hz screen then shows each frame twice rather than running the guest at
    // double speed, and a slow one catches up in bursts.
    let running = true;
    for (let n = 0; running && n < play.burst() && play.starved(); n += 1) {
      running = demo.frame();
      play(demo.takeAudio());
    }

    // A fresh view every frame: growing the wasm heap detaches the old one.
    // WebGL uploads straight from here, with no copy in between.
    const pixels = new Uint8Array(
      wasm.memory.buffer,
      demo.framebufferPtr(),
      demo.framebufferLen(),
    );
    renderer.draw(pixels);

    // A program stopped and the supervisor has the machine back. Replace the
    // entry rather than popping one: a deep-linked page has nothing of ours
    // behind it, and going back would leave the app altogether.
    if (demo.takeReturned()) {
      showAddress(null);
    }

    if (running) {
      requestAnimationFrame(tick);
      return;
    }

    // The supervisor asked for a program. Go and get it, then carry on.
    const asked = demo.takeLoadRequest();
    if (asked !== undefined) {
      const path = address(asked);
      open(path)
        .then(() => {
          // A new entry, so the back button comes back here.
          showAddress(path, { push: true });
          requestAnimationFrame(tick);
        })
        .catch((error) => {
          console.warn(error);
          // Hand the failure back to the supervisor rather than stopping dead.
          demo.reportLoadFailure();
          output.textContent = `${error}`;
          showAddress(null);
          requestAnimationFrame(tick);
        });
      return;
    }

    output.textContent = demo.report();
  };

  requestAnimationFrame(tick);
}

main();
