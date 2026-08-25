// Collecting everything the browser will tell us about input, and handing it
// to the console once per frame.
//
// Nothing here decides what a key means. The keyboard goes across as a bitmap
// of all 256 key codes and gamepads as raw buttons, sticks and triggers; the
// game maps them to actions.

// Key codes, matching `Key` in sdk/input.rs. Anything absent from this table
// simply does not reach the guest.
const KEYS = {
  KeyA: 1, KeyB: 2, KeyC: 3, KeyD: 4, KeyE: 5, KeyF: 6, KeyG: 7,
  KeyH: 8, KeyI: 9, KeyJ: 10, KeyK: 11, KeyL: 12, KeyM: 13, KeyN: 14,
  KeyO: 15, KeyP: 16, KeyQ: 17, KeyR: 18, KeyS: 19, KeyT: 20, KeyU: 21,
  KeyV: 22, KeyW: 23, KeyX: 24, KeyY: 25, KeyZ: 26,

  Digit0: 30, Digit1: 31, Digit2: 32, Digit3: 33, Digit4: 34,
  Digit5: 35, Digit6: 36, Digit7: 37, Digit8: 38, Digit9: 39,

  Escape: 40, Enter: 41, Tab: 42, Space: 43, Backspace: 44, Delete: 45,

  Minus: 50, Equal: 51, BracketLeft: 52, BracketRight: 53, Backslash: 54,
  Semicolon: 55, Quote: 56, Backquote: 57, Comma: 58, Period: 59, Slash: 60,

  ArrowUp: 70, ArrowDown: 71, ArrowLeft: 72, ArrowRight: 73,

  Home: 80, End: 81, PageUp: 82, PageDown: 83, Insert: 84,

  ShiftLeft: 90, ShiftRight: 91, ControlLeft: 92, ControlRight: 93,
  AltLeft: 94, AltRight: 95, MetaLeft: 96, MetaRight: 97,

  F1: 100, F2: 101, F3: 102, F4: 103, F5: 104, F6: 105,
  F7: 106, F8: 107, F9: 108, F10: 109, F11: 110, F12: 111,

  // The numpad reports its own codes, but nothing on it means anything
  // different from its counterpart elsewhere on the keyboard, so it lands on
  // the same console keys. Without this, numpad Enter is simply dead.
  NumpadEnter: 41,
  Numpad0: 30, Numpad1: 31, Numpad2: 32, Numpad3: 33, Numpad4: 34,
  Numpad5: 35, Numpad6: 36, Numpad7: 37, Numpad8: 38, Numpad9: 39,
  NumpadAdd: 51, NumpadSubtract: 50, NumpadDivide: 60, NumpadDecimal: 59,
};

// Keys the browser would otherwise act on itself. Everything else keeps its
// normal behaviour, so reloading and switching tabs still work.
const SWALLOW = new Set([
  "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight",
  "Space", "Tab", "Backspace", "Enter", "NumpadEnter",
]);

const MODIFIER = { Shift: 1 << 0, Control: 1 << 1, Alt: 1 << 2, Meta: 1 << 3 };

/** The standard gamepad mapping's button order, as console button bits. */
const PAD_BUTTONS = [
  4, // 0  bottom face  -> A
  5, // 1  right face   -> B
  6, // 2  left face    -> X
  7, // 3  top face     -> Y
  8, // 4  L1
  9, // 5  R1
  10, // 6 L2 (also read as an analog trigger)
  11, // 7 R2
  12, // 8 Select
  13, // 9 Start
  14, // 10 L3
  15, // 11 R3
  0, // 12 d-pad up
  1, // 13 d-pad down
  2, // 14 d-pad left
  3, // 15 d-pad right
];

const AXIS = (value) => Math.max(-127, Math.min(127, Math.round((value ?? 0) * 127)));

/**
 * Start listening. Call the returned `pump(demo)` once per frame, before
 * stepping the machine.
 */
export function attachInput(target = window) {
  const held = new Uint8Array(32);
  // Keys that went down since the last pump. A tap that starts and ends between
  // two frames never appears in `held` at all, so without this it would be lost
  // entirely — and a menu you have to hold a key down for is a broken menu.
  const pressed = new Uint8Array(32);
  let modifiers = 0;

  const setKey = (event, down) => {
    const code = KEYS[event.code];
    if (code !== undefined) {
      if (down) {
        held[code >> 3] |= 1 << (code & 7);
        // Ignore auto-repeat: one press is one press.
        if (!event.repeat) pressed[code >> 3] |= 1 << (code & 7);
      } else {
        held[code >> 3] &= ~(1 << (code & 7));
      }
    }
    // Track modifiers even for keys we have no code for.
    for (const [name, bit] of Object.entries(MODIFIER)) {
      if (event.getModifierState?.(name)) modifiers |= bit;
      else modifiers &= ~bit;
    }
    if (SWALLOW.has(event.code)) event.preventDefault();
  };

  target.addEventListener("keydown", (e) => setKey(e, true));
  target.addEventListener("keyup", (e) => setKey(e, false));
  // A key released while the page is unfocused would otherwise stick down.
  target.addEventListener("blur", () => {
    held.fill(0);
    pressed.fill(0);
    modifiers = 0;
  });

  return function pump(demo) {
    demo.setKeys(held, pressed, modifiers);
    pressed.fill(0);

    const pads = navigator.getGamepads ? navigator.getGamepads() : [];
    let count = 0;
    for (const pad of pads) {
      if (!pad || count >= 4) continue;
      let buttons = 0;
      pad.buttons.forEach((button, i) => {
        const bit = PAD_BUTTONS[i];
        if (bit !== undefined && button.pressed) buttons |= 1 << bit;
      });
      demo.setGamepad(
        count,
        true,
        buttons,
        AXIS(pad.axes[0]),
        AXIS(pad.axes[1]),
        AXIS(pad.axes[2]),
        AXIS(pad.axes[3]),
        Math.round((pad.buttons[6]?.value ?? 0) * 255),
        Math.round((pad.buttons[7]?.value ?? 0) * 255),
      );
      count += 1;
    }
    // Anything unplugged since last frame must stop reading as held.
    demo.clearGamepadsFrom(count);
  };
}
