use crate::capturable::Capturable;
use crate::protocol::{ClickEvent, KeyboardEvent, PointerEvent, TextEvent, WheelEvent};

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
    fn set_capturable(&mut self, capturable: Box<dyn Capturable>);
    fn device_type(&self) -> InputDeviceType;
}
