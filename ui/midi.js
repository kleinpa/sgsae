// Web MIDI, handed straight through to the console.
//
// Nothing here interprets a message. The browser gives us bytes off the wire
// and the emulator's MIDI port takes the same bytes — a note, a clock pulse and
// a pitch bend all travel the same path, and the cartridge decides what any of
// them mean.
//
// Messages arrive on their own schedule, not on the frame boundary, so they are
// delivered as they land rather than being batched into the frame loop. That
// matters for clock: a sequencer following an external device needs the pulses
// to arrive when they were sent.

/**
 * Ask for MIDI access and route every input to `demo`.
 *
 * Resolves to a function reporting the current port count. Access is a
 * permission prompt in most browsers and is simply absent in some, so a
 * failure here is normal and leaves the console running without MIDI.
 */
export async function attachMidi(demo) {
  if (!navigator.requestMIDIAccess) {
    console.info("Web MIDI is unavailable in this browser");
    return () => 0;
  }

  let access;
  try {
    access = await navigator.requestMIDIAccess({ sysex: false });
  } catch (error) {
    console.info("MIDI access was refused:", error);
    return () => 0;
  }

  const listen = (port) => {
    // Reassigning is idempotent, so re-running on a state change is safe.
    port.onmidimessage = (event) => demo.midiPacket(event.data);
  };

  const refresh = () => {
    let count = 0;
    for (const port of access.inputs.values()) {
      listen(port);
      count += 1;
    }
    demo.setMidiConnected(count > 0);
    return count;
  };

  // Devices come and go while the page is open.
  access.onstatechange = refresh;
  refresh();

  return () => access.inputs.size;
}
