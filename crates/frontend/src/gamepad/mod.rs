// SPDX-License-Identifier: Apache-2.0
//! Gamepads through `gilrs`, mapped to console buttons by position (#44). The mapping is pure:
//! [`PadEvent`] is what a `gilrs` event means to the console, [`Pad`] the buttons a pad holds, and
//! only [`Gamepads`] touches the library, as a thin adapter the loop polls once a frame.
//!
//! The face buttons go by position, not by label, so the layout feels like the console's whatever
//! the pad's own markings are: East is A, South B, North X, West Y. The shoulder buttons
//! (`LeftTrigger`, `RightTrigger`) are L and R; Start and Select are themselves. The D-pad buttons
//! and the left stick (past [`DEADZONE`]) are the D-pad. The pad's buttons are OR-ed with the
//! keyboard's (`App::graphics`).

use gilrs::{Axis, Button, EventType, Gilrs};
use ninge_core::Buttons;

/// How far the left stick must lean, 0 to 1, before it presses the D-pad: half way, so a worn
/// stick that rests off center does not press by itself, and a deliberate lean always does.
pub const DEADZONE: f32 = 0.5;

/// What a gamepad event means to the console.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PadEvent {
    /// A button went down (`true`) or up.
    Button(Button, bool),
    /// An axis moved to a value in -1 to 1.
    Axis(Axis, f32),
    /// A pad was unplugged: its buttons let go.
    Disconnected,
}

/// What `event` means to the console, `None` for what it ignores.
pub fn translate(event: &EventType) -> Option<PadEvent> {
    match *event {
        EventType::ButtonPressed(button, _) => Some(PadEvent::Button(button, true)),
        EventType::ButtonReleased(button, _) => Some(PadEvent::Button(button, false)),
        EventType::AxisChanged(axis, value, _) => Some(PadEvent::Axis(axis, value)),
        EventType::Disconnected => Some(PadEvent::Disconnected),
        _ => None,
    }
}

/// The console button a pad button stands for, by position; none for the rest.
pub const fn button_for(button: Button) -> Buttons {
    match button {
        Button::East => Buttons::A,
        Button::South => Buttons::B,
        Button::North => Buttons::X,
        Button::West => Buttons::Y,
        Button::LeftTrigger => Buttons::L,
        Button::RightTrigger => Buttons::R,
        Button::Start => Buttons::START,
        Button::Select => Buttons::SELECT,
        Button::DPadUp => Buttons::UP,
        Button::DPadDown => Buttons::DOWN,
        Button::DPadLeft => Buttons::LEFT,
        Button::DPadRight => Buttons::RIGHT,
        _ => Buttons::empty(),
    }
}

/// The buttons the pads hold now.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pad {
    /// The buttons held, as bits of [`Buttons`].
    buttons: u16,
    /// The left stick, each axis -1 to 1.
    stick: (f32, f32),
}

impl Pad {
    /// No button held.
    pub const fn new() -> Self {
        Self {
            buttons: 0,
            stick: (0.0, 0.0),
        }
    }

    /// Takes an event.
    pub fn handle(&mut self, event: PadEvent) {
        match event {
            PadEvent::Button(button, down) => {
                let bits = button_for(button).bits();
                self.buttons = if down {
                    self.buttons | bits
                } else {
                    self.buttons & !bits
                };
            }
            PadEvent::Axis(Axis::LeftStickX, value) => self.stick.0 = value,
            PadEvent::Axis(Axis::LeftStickY, value) => self.stick.1 = value,
            PadEvent::Axis(..) => {}
            // With several pads, one leaving lets go of what the others hold too: the state is
            // one set of buttons, and the next event of a pad that is still held re-presses.
            PadEvent::Disconnected => self.release_all(),
        }
    }

    /// Lets every button and the stick go, as when the terminal loses focus.
    pub fn release_all(&mut self) {
        self.buttons = 0;
        self.stick = (0.0, 0.0);
    }

    /// The console buttons held: the pad's buttons and the stick's leans. `gilrs` reports the
    /// stick's Y as positive going up.
    pub fn buttons(&self) -> Buttons {
        let (x, y) = self.stick;
        let lean = |on: bool, button: Buttons| if on { button } else { Buttons::empty() };
        Buttons::from_bits_truncate(self.buttons)
            | lean(x > DEADZONE, Buttons::RIGHT)
            | lean(x < -DEADZONE, Buttons::LEFT)
            | lean(y > DEADZONE, Buttons::UP)
            | lean(y < -DEADZONE, Buttons::DOWN)
    }
}

/// The `gilrs` library behind a thin adapter: `None` when it could not start (no input subsystem,
/// say), which is not an error: the keyboard and mouse still work.
pub struct Gamepads {
    gilrs: Option<Gilrs>,
}

impl Gamepads {
    /// Starts the library.
    pub fn open() -> Self {
        Self {
            gilrs: Gilrs::new().ok(),
        }
    }

    /// Whether the library started.
    pub const fn available(&self) -> bool {
        self.gilrs.is_some()
    }

    /// How many pads are plugged in now, asked of the library rather than counted from events.
    pub fn connected(&self) -> u32 {
        self.gilrs.as_ref().map_or(0, |g| {
            u32::try_from(g.gamepads().count()).unwrap_or(u32::MAX)
        })
    }

    /// The events since the last call, translated.
    pub fn poll(&mut self) -> Vec<PadEvent> {
        let mut events = Vec::new();
        if let Some(gilrs) = &mut self.gilrs {
            while let Some(event) = gilrs.next_event() {
                events.extend(translate(&event.event));
            }
        }
        events
    }
}

#[cfg(test)]
mod tests;
