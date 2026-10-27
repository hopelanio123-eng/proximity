use crate::capturable::Capturable;
use crate::protocol::{
    ClickEvent, KeyboardEvent, KeyboardEventType, KeyboardLocation, MouseMoveEvent, PointerEvent,
    TextEdit, TextEvent, WheelEvent,
};

#[derive(PartialEq, Eq)]
pub enum InputDeviceType {
    AutoPilotDevice,
    UInputDevice,
    #[cfg(target_os = "windows")]
    WindowsInput,
}

pub trait InputDevice {
    fn send_wheel_event(&mut self, event: &WheelEvent);
    fn send_pointer_event(&mut self, event: &PointerEvent);
    fn send_keyboard_event(&mut self, event: &KeyboardEvent);
    fn send_text_event(&mut self, event: &TextEvent);
    fn send_click_event(&mut self, event: &ClickEvent);
    fn send_mouse_move_event(&mut self, event: &MouseMoveEvent);
    fn set_capturable(&mut self, capturable: Box<dyn Capturable>);
    fn device_type(&self) -> InputDeviceType;

    /// Apply a TextEdit produced by the client's text funnel. Both the arrows
    /// and the insertion are expressed through the primitive events every
    /// backend can generate, so this fallback works everywhere. Backends with
    /// a native unicode injection path (like the Windows SendInput one)
    /// override this method.
    fn send_text_edit_event(&mut self, event: &TextEdit) {
        const MAX: u32 = 4096;
        for _ in 0..event.left.min(MAX) {
            self.send_key_press_event("ArrowLeft");
        }
        for _ in 0..event.backspace.min(MAX) {
            self.send_key_press_event("Backspace");
        }
        if !event.text.is_empty() {
            self.send_text_event(&TextEvent {
                text: event.text.clone(),
            });
        }
        for _ in 0..event.right.min(MAX) {
            self.send_key_press_event("ArrowRight");
        }
    }

    /// Helper for send_text_edit_event: one full press and release of a
    /// modifier-less key, addressed by its JavaScript KeyboardEvent code.
    fn send_key_press_event(&mut self, code: &str) {
        let down = KeyboardEvent {
            event_type: KeyboardEventType::DOWN,
            code: code.to_string(),
            key: code.to_string(),
            location: KeyboardLocation::STANDARD,
            alt: false,
            ctrl: false,
            shift: false,
            meta: false,
        };
        self.send_keyboard_event(&down);
        self.send_keyboard_event(&KeyboardEvent {
            event_type: KeyboardEventType::UP,
            ..down
        });
    }
}
