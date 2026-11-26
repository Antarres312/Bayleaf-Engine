//! The first connected gamepad, read with gilrs, in the form scripts see it.

use bayleaf_engine::script::{PAD_BUTTONS, Pad};
use gilrs::{Axis, Button};

/// Gamepad buttons in the order of [`PAD_BUTTONS`].
const BUTTONS: [Button; 16] = [
    Button::South,
    Button::East,
    Button::West,
    Button::North,
    Button::LeftTrigger,
    Button::RightTrigger,
    Button::LeftTrigger2,
    Button::RightTrigger2,
    Button::Select,
    Button::Start,
    Button::LeftThumb,
    Button::RightThumb,
    Button::DPadUp,
    Button::DPadDown,
    Button::DPadLeft,
    Button::DPadRight,
];

/// Reads gamepads. The system is opened the first time one is asked for, so a session that never
/// plays never opens it; when it cannot be opened, there is no gamepad.
#[derive(Default)]
pub struct Gamepads {
    gilrs: Option<Option<gilrs::Gilrs>>,
}

impl Gamepads {
    /// The first connected gamepad now, or `None`.
    pub fn read(&mut self) -> Option<Pad> {
        let gilrs = self.gilrs.get_or_insert_with(|| gilrs::Gilrs::new().ok()).as_mut()?;
        // Events must be taken for the state to stay current.
        while gilrs.next_event().is_some() {}
        let (_, pad) = gilrs.gamepads().find(|(_, g)| g.is_connected())?;
        let trigger = |b: Button| pad.button_data(b).map_or(0.0, gilrs::ev::state::ButtonData::value);
        let axes = [
            pad.value(Axis::LeftStickX),
            pad.value(Axis::LeftStickY),
            pad.value(Axis::RightStickX),
            pad.value(Axis::RightStickY),
            trigger(Button::LeftTrigger2),
            trigger(Button::RightTrigger2),
        ];
        let buttons = BUTTONS.iter().enumerate().filter(|(_, b)| pad.is_pressed(**b)).fold(0u32, |bits, (i, _)| bits | (1 << i));
        debug_assert_eq!(BUTTONS.len(), PAD_BUTTONS.len());
        Some(Pad { axes, buttons })
    }
}
