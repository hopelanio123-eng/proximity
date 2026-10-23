use winapi::shared::minwindef::{DWORD, WORD};
use winapi::shared::windef::{HWND, POINT};
use winapi::um::winuser::*;

use tracing::warn;

use crate::input::autopilot_device::AutoPilotDevice;
use crate::input::device::{InputDevice, InputDeviceType};
use crate::protocol::{
    Button, ClickButton, ClickEvent, ClickEventType, KeyboardEvent, KeyboardEventType,
    PointerEvent, PointerEventType, PointerType, TextEdit, TextEvent, WheelEvent,
};

use crate::capturable::{Capturable, Geometry};

/// Ported from the remote_control keyboard backend: navigation keys share
/// scan codes with the numeric keypad and must be flagged as extended, or the
/// system presses both the arrow and its keypad equivalent.
fn extended_key(vk: u16) -> bool {
    const EXTENDED: [u16; 13] = [
        VK_LWIN as u16,
        VK_MENU as u16,
        VK_DELETE as u16,
        VK_INSERT as u16,
        VK_HOME as u16,
        VK_END as u16,
        VK_PRIOR as u16,
        VK_NEXT as u16,
        VK_LEFT as u16,
        VK_RIGHT as u16,
        VK_UP as u16,
        VK_DOWN as u16,
        VK_SNAPSHOT as u16,
    ];
    EXTENDED.contains(&vk)
}

/// KeyboardEvent/shortcut name to virtual key code, ported from
/// remote_control's input/keycodes.rs.
fn string_to_vk(key: &str) -> Option<u16> {
    match key.to_lowercase().as_str() {
        "enter" | "return" => Some(VK_RETURN as u16),
        "backspace" => Some(VK_BACK as u16),
        "tab" => Some(VK_TAB as u16),
        "escape" | "esc" => Some(VK_ESCAPE as u16),
        "space" => Some(VK_SPACE as u16),
        "delete" | "del" => Some(VK_DELETE as u16),
        "insert" | "ins" => Some(VK_INSERT as u16),
        "printscreen" | "prntscr" | "prtsc" => Some(VK_SNAPSHOT as u16),
        "home" => Some(VK_HOME as u16),
        "end" => Some(VK_END as u16),
        "pageup" | "prior" => Some(VK_PRIOR as u16),
        "pagedown" | "next" => Some(VK_NEXT as u16),
        "arrowup" | "up" => Some(VK_UP as u16),
        "arrowdown" | "down" => Some(VK_DOWN as u16),
        "arrowleft" | "left" => Some(VK_LEFT as u16),
        "arrowright" | "right" => Some(VK_RIGHT as u16),
        "control" | "ctrl" => Some(VK_CONTROL as u16),
        "alt" | "menu" => Some(VK_MENU as u16),
        "shift" => Some(VK_SHIFT as u16),
        "meta" | "win" | "windows" => Some(VK_LWIN as u16),
        "f1" => Some(VK_F1 as u16),
        "f2" => Some(VK_F2 as u16),
        "f3" => Some(VK_F3 as u16),
        "f4" => Some(VK_F4 as u16),
        "f5" => Some(VK_F5 as u16),
        "f6" => Some(VK_F6 as u16),
        "f7" => Some(VK_F7 as u16),
        "f8" => Some(VK_F8 as u16),
        "f9" => Some(VK_F9 as u16),
        "f10" => Some(VK_F10 as u16),
        "f11" => Some(VK_F11 as u16),
        "f12" => Some(VK_F12 as u16),
        // Letter and digit keys
        s if s.len() == 1 => {
            let c = s.chars().next().unwrap().to_ascii_uppercase();
            if c.is_ascii_alphanumeric() {
                Some(c as u16)
            } else {
                None
            }
        }
        _ => None,
    }
}

pub struct WindowsInput {
    capturable: Box<dyn Capturable>,
    autopilot_device: AutoPilotDevice,
    pointer_device_handle: *mut HSYNTHETICPOINTERDEVICE__,
    touch_device_handle: *mut HSYNTHETICPOINTERDEVICE__,
    multitouch_map: std::collections::HashMap<i64, POINTER_TYPE_INFO>,
}

impl WindowsInput {
    /// One full press and release of a virtual key, honoring the extended-key
    /// flag (port of remote_control's key_press).
    fn key_press(&mut self, key: &str) {
        let Some(vk) = string_to_vk(key) else { return };
        unsafe {
            let scan = MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) as u8;
            let mut flags: DWORD = 0;
            if extended_key(vk) {
                flags |= KEYEVENTF_EXTENDEDKEY;
            }
            keybd_event(vk as u8, scan, flags, 0);
            keybd_event(vk as u8, scan, flags | KEYEVENTF_KEYUP, 0);
        }
    }

    /// Inject unicode text using one SendInput call with down/up pairs
    /// interleaved, ported from remote_control's type_text.
    fn type_text(&mut self, text: &str) {
        let mut inputs = Vec::<INPUT>::with_capacity(2 * text.encode_utf16().count());
        for code_point in text.encode_utf16() {
            let mut down: INPUT = unsafe { std::mem::zeroed() };
            down.type_ = INPUT_KEYBOARD;
            unsafe {
                down.u.ki_mut().wVk = 0;
                down.u.ki_mut().wScan = code_point;
                down.u.ki_mut().dwFlags = KEYEVENTF_UNICODE;
            }
            inputs.push(down);
            let mut up: INPUT = unsafe { std::mem::zeroed() };
            up.type_ = INPUT_KEYBOARD;
            unsafe {
                up.u.ki_mut().wVk = 0;
                up.u.ki_mut().wScan = code_point;
                up.u.ki_mut().dwFlags = KEYEVENTF_UNICODE | KEYEVENTF_KEYUP;
            }
            inputs.push(up);
        }
        if inputs.is_empty() {
            return;
        }
        let sent = unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_mut_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            )
        };
        if sent != inputs.len() as u32 {
            warn!(
                "SendInput was blocked from injecting all key events ({}/{}), \
                buffer may not have been typed fully",
                sent,
                inputs.len()
            );
        }
    }

    pub fn new(capturable: Box<dyn Capturable>) -> Self {
        unsafe {
            InitializeTouchInjection(5, TOUCH_FEEDBACK_DEFAULT);
            Self {
                capturable: capturable.clone(),
                autopilot_device: AutoPilotDevice::new(capturable),
                pointer_device_handle: CreateSyntheticPointerDevice(PT_PEN, 1, 1),
                touch_device_handle: CreateSyntheticPointerDevice(PT_TOUCH, 5, 1),
                multitouch_map: std::collections::HashMap::new(),
            }
        }
    }

    fn send_print_screen(&mut self, event: &KeyboardEvent) {
        if let KeyboardEventType::REPEAT = event.event_type {
            return;
        }
        if let Err(err) = self.capturable.before_input() {
            warn!("Failed to activate window, sending no input ({})", err);
            return;
        }
        // Windows takes the screenshot as soon as PrintScreen is pressed, so
        // react only to down events and inject one complete press+release of
        // VK_SNAPSHOT.
        if let KeyboardEventType::DOWN = event.event_type {
            let mut down: INPUT = unsafe { std::mem::zeroed() };
            down.type_ = INPUT_KEYBOARD;
            unsafe {
                down.u.ki_mut().wVk = VK_SNAPSHOT as WORD;
            }
            let mut up: INPUT = unsafe { std::mem::zeroed() };
            up.type_ = INPUT_KEYBOARD;
            unsafe {
                up.u.ki_mut().wVk = VK_SNAPSHOT as WORD;
                up.u.ki_mut().dwFlags = KEYEVENTF_KEYUP;
            }
            let mut keys = [down, up];
            let sent =
                unsafe { SendInput(2, keys.as_mut_ptr(), std::mem::size_of::<INPUT>() as i32) };
            if sent != 2 {
                warn!("SendInput was blocked from injecting the PrintScreen key");
            }
        }
    }
}

impl InputDevice for WindowsInput {
    fn send_wheel_event(&mut self, event: &WheelEvent) {
        unsafe { mouse_event(MOUSEEVENTF_WHEEL, 0, 0, event.dy as DWORD, 0) };
    }

    fn send_pointer_event(&mut self, event: &PointerEvent) {
        if let Err(err) = self.capturable.before_input() {
            warn!("Failed to activate window, sending no input ({})", err);
            return;
        }
        let Geometry::VirtualScreen(offset_x, offset_y, width, height, left, top) =
            self.capturable.geometry().unwrap()
        else {
            unreachable!()
        };

        let (x, y) = (
            (event.x * width as f64) as i32 + offset_x,
            (event.y * height as f64) as i32 + offset_y,
        );
        let mut pointer_flags = match event.event_type {
            PointerEventType::DOWN => {
                POINTER_FLAG_INRANGE | POINTER_FLAG_INCONTACT | POINTER_FLAG_DOWN
            }
            PointerEventType::MOVE | PointerEventType::OVER | PointerEventType::ENTER => {
                POINTER_FLAG_INRANGE | POINTER_FLAG_UPDATE
            }
            PointerEventType::UP => POINTER_FLAG_UP,
            PointerEventType::CANCEL | PointerEventType::LEAVE | PointerEventType::OUT => {
                POINTER_FLAG_INRANGE | POINTER_FLAG_UPDATE | POINTER_FLAG_CANCELED
            }
        };
        let button_change_type = match event.buttons {
            Button::PRIMARY => {
                pointer_flags |= POINTER_FLAG_INCONTACT;
                POINTER_CHANGE_FIRSTBUTTON_DOWN
            }
            Button::SECONDARY => POINTER_CHANGE_SECONDBUTTON_DOWN,
            Button::AUXILARY => POINTER_CHANGE_THIRDBUTTON_DOWN,
            Button::NONE => POINTER_CHANGE_NONE,
            _ => POINTER_CHANGE_NONE,
        };
        if event.is_primary {
            pointer_flags |= POINTER_FLAG_PRIMARY;
        }
        match event.pointer_type {
            PointerType::Pen => {
                unsafe {
                    let mut pointer_type_info = POINTER_TYPE_INFO {
                        type_: PT_PEN,
                        u: std::mem::zeroed(),
                    };
                    *pointer_type_info.u.penInfo_mut() = POINTER_PEN_INFO {
                        pointerInfo: POINTER_INFO {
                            pointerType: PT_PEN,
                            pointerId: event.pointer_id as u32,
                            frameId: 0,
                            pointerFlags: pointer_flags,
                            sourceDevice: 0 as *mut winapi::ctypes::c_void, //maybe use syntheticPointerDeviceHandle here but works with 0
                            hwndTarget: 0 as HWND,
                            ptPixelLocation: POINT { x: x, y: y },
                            ptHimetricLocation: POINT { x: 0, y: 0 },
                            ptPixelLocationRaw: POINT { x: x, y: y },
                            ptHimetricLocationRaw: POINT { x: 0, y: 0 },
                            dwTime: 0,
                            historyCount: 1,
                            InputData: 0,
                            dwKeyStates: 0,
                            PerformanceCount: 0,
                            ButtonChangeType: button_change_type,
                        },
                        penFlags: PEN_FLAG_NONE,
                        penMask: PEN_MASK_PRESSURE
                            | PEN_MASK_ROTATION
                            | PEN_MASK_TILT_X
                            | PEN_MASK_TILT_Y,
                        pressure: (event.pressure * 1024f64) as u32,
                        rotation: event.twist as u32,
                        tiltX: event.tilt_x,
                        tiltY: event.tilt_y,
                    };
                    InjectSyntheticPointerInput(self.pointer_device_handle, &pointer_type_info, 1);
                }
            }
            PointerType::Touch => {
                unsafe {
                    let mut pointer_type_info = POINTER_TYPE_INFO {
                        type_: PT_TOUCH,
                        u: std::mem::zeroed(),
                    };

                    let mut pointer_touch_info: POINTER_TOUCH_INFO = std::mem::zeroed();
                    pointer_touch_info.pointerInfo = std::mem::zeroed();
                    pointer_touch_info.pointerInfo.pointerType = PT_TOUCH;
                    pointer_touch_info.pointerInfo.pointerFlags = pointer_flags;
                    pointer_touch_info.pointerInfo.pointerId = event.pointer_id as u32; //event.pointer_id as u32; Using the actual pointer id causes errors in the touch injection
                    pointer_touch_info.pointerInfo.ptPixelLocation = POINT { x, y };
                    pointer_touch_info.touchFlags = TOUCH_FLAG_NONE;
                    pointer_touch_info.touchMask = TOUCH_MASK_PRESSURE;
                    pointer_touch_info.pressure = (event.pressure * 1024f64) as u32;

                    pointer_touch_info.pointerInfo.ButtonChangeType = button_change_type;

                    *pointer_type_info.u.touchInfo_mut() = pointer_touch_info;
                    self.multitouch_map
                        .insert(event.pointer_id, pointer_type_info);
                    let len = self.multitouch_map.len();

                    let mut pointer_type_info_vec: Vec<POINTER_TYPE_INFO> = Vec::new();
                    for (_i, info) in self.multitouch_map.iter().enumerate() {
                        pointer_type_info_vec.push(*info.1);
                    }
                    let b: Box<[POINTER_TYPE_INFO]> = pointer_type_info_vec.into_boxed_slice();
                    let m: *mut POINTER_TYPE_INFO = Box::into_raw(b) as _;

                    InjectSyntheticPointerInput(self.touch_device_handle, m, len as u32);

                    match event.event_type {
                        PointerEventType::DOWN
                        | PointerEventType::MOVE
                        | PointerEventType::OVER
                        | PointerEventType::ENTER => {}

                        PointerEventType::UP
                        | PointerEventType::CANCEL
                        | PointerEventType::LEAVE
                        | PointerEventType::OUT => {
                            self.multitouch_map.remove(&event.pointer_id);
                        }
                    }
                }
            }
            PointerType::Mouse => {
                let mut dw_flags = 0;

                let (screen_x, screen_y) = (
                    (event.x * width as f64) as i32 + left,
                    (event.y * height as f64) as i32 + top,
                );

                match event.event_type {
                    PointerEventType::DOWN => match event.buttons {
                        Button::PRIMARY => {
                            dw_flags |= MOUSEEVENTF_LEFTDOWN;
                        }
                        Button::SECONDARY => {
                            dw_flags |= MOUSEEVENTF_RIGHTDOWN;
                        }
                        Button::AUXILARY => {
                            dw_flags |= MOUSEEVENTF_MIDDLEDOWN;
                        }
                        _ => {}
                    },
                    PointerEventType::MOVE | PointerEventType::OVER | PointerEventType::ENTER => {
                        unsafe { SetCursorPos(screen_x, screen_y) };
                    }
                    PointerEventType::UP => match event.button {
                        Button::PRIMARY => {
                            dw_flags |= MOUSEEVENTF_LEFTUP;
                        }
                        Button::SECONDARY => {
                            dw_flags |= MOUSEEVENTF_RIGHTUP;
                        }
                        Button::AUXILARY => {
                            dw_flags |= MOUSEEVENTF_MIDDLEUP;
                        }
                        _ => {}
                    },
                    PointerEventType::CANCEL | PointerEventType::LEAVE | PointerEventType::OUT => {
                        dw_flags |= MOUSEEVENTF_LEFTUP;
                    }
                }
                unsafe { mouse_event(dw_flags, 0 as u32, 0 as u32, 0, 0) };
            }
            PointerType::Unknown => todo!(),
        }
    }

    fn send_keyboard_event(&mut self, event: &KeyboardEvent) {
        if event.code == "PrintScreen" {
            self.send_print_screen(event);
            return;
        }
        self.autopilot_device.send_keyboard_event(event);
    }

    fn send_click_event(&mut self, event: &ClickEvent) {
        if let Err(err) = self.capturable.before_input() {
            warn!("Failed to activate window, sending no input ({})", err);
            return;
        }
        // Injecting a button event without moving the cursor acts at its
        // current position.
        let flags = match event.button {
            ClickButton::LEFT => match event.action {
                ClickEventType::DOWN => MOUSEEVENTF_LEFTDOWN,
                _ => MOUSEEVENTF_LEFTUP,
            },
            ClickButton::MIDDLE => match event.action {
                ClickEventType::DOWN => MOUSEEVENTF_MIDDLEDOWN,
                _ => MOUSEEVENTF_MIDDLEUP,
            },
            ClickButton::RIGHT => match event.action {
                ClickEventType::DOWN => MOUSEEVENTF_RIGHTDOWN,
                _ => MOUSEEVENTF_RIGHTUP,
            },
        };
        unsafe { mouse_event(flags, 0 as u32, 0 as u32, 0, 0) };
    }

    fn send_text_event(&mut self, event: &TextEvent) {
        if let Err(err) = self.capturable.before_input() {
            warn!("Failed to activate window, sending no input ({})", err);
            return;
        }
        self.type_text(&event.text);
    }

    // Port of remote_control's edit_text: apply the replacement of the
    // previous insertion as caret movements, deletions and new text instead of
    // retyping everything, keeping IME buffers and autocorrect consistent
    // with what the phone shows.
    fn send_text_edit_event(&mut self, event: &TextEdit) {
        if let Err(err) = self.capturable.before_input() {
            warn!("Failed to activate window, sending no input ({})", err);
            return;
        }
        const MAX: u32 = 4096;
        for _ in 0..event.left.min(MAX) {
            self.key_press("ArrowLeft");
        }
        for _ in 0..event.backspace.min(MAX) {
            self.key_press("Backspace");
        }
        self.type_text(&event.text);
        for _ in 0..event.right.min(MAX) {
            self.key_press("ArrowRight");
        }
    }

    fn set_capturable(&mut self, capturable: Box<dyn Capturable>) {
        self.capturable = capturable;
    }

    fn device_type(&self) -> InputDeviceType {
        InputDeviceType::WindowsInput
    }
}
