use std::cmp::min;
use std::io::Cursor;
use std::iter::Iterator;
use std::net::{IpAddr, SocketAddr};
#[cfg(target_os = "windows")]
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};

use fltk::app;
use fltk::draw;
use fltk::enums::{Align, CallbackTrigger, Color, Font, FrameType, LabelType, UnmappedFrameType};
use fltk::image::PngImage;
use fltk::menu::Choice;
use std::sync::{mpsc, Arc, Mutex};
use tracing::{error, info, warn};

use fltk::{
    app::{awake_callback, App},
    button::{Button, CheckButton},
    frame::Frame,
    group::Group,
    input::{Input, IntInput},
    output::Output,
    prelude::*,
    text::{TextBuffer, TextDisplay},
    window::Window,
};

#[cfg(not(target_os = "windows"))]
use pnet_datalink as datalink;

use crate::config::{write_config, Config};
use crate::protocol::{CustomInputAreas, Rect};
use crate::gui_art::{
    app_icon, draw_glyph, ICON_CHECK_SVG, ICON_COPY_SVG, ICON_HOME_SVG, ICON_SETTINGS_SVG,
    LOGO_GLYPH_SVG,
};
use crate::theme::{palette_by_name, Rgb24, Skin, DEFAULT_PALETTE, PALETTES};
use crate::web::Web2UiMessage::UInputInaccessible;

/// The active palette, kept globally so the custom draw callbacks of the
/// sidebar, page backdrop and widgets read the same colors at paint time.
static SKIN: Mutex<Option<Skin>> = Mutex::new(None);

/// Current skin; falls back to the default palette until it is initialized.
fn skin() -> Skin {
    let guard = SKIN.lock().unwrap();
    match guard.as_ref() {
        Some(s) => s.clone(),
        None => Skin::from_palette(DEFAULT_PALETTE),
    }
}

/// Adopt a palette by name ( Falling back to the default for unknown or
/// missing names, which also covers configs written by older releases) and
/// re-skin the whole window.
fn apply_palette(name: Option<&str>) {
    let (palette, found) = match name.map(palette_by_name) {
        Some(Some(p)) => (p, true),
        Some(None) => (DEFAULT_PALETTE, false),
        None => (DEFAULT_PALETTE, true),
    };
    if let (false, Some(missing)) = (found, name) {
        warn!("Unknown gui theme '{missing}', falling back to {}", DEFAULT_PALETTE.name);
    }
    *SKIN.lock().unwrap() = Some(Skin::from_palette(palette));
    apply_skin();
    // Read the flag first: passing the lock guard straight into show_page()
    // would keep HOME_ACTIVE locked while show_page() locks it again.
    let home = *HOME_ACTIVE.lock().unwrap();
    show_page(home);
    if let Some(mut w) = app::widget_from_id::<Window>("main_window") {
        w.redraw();
    }
}

/// Whether a font family looks installed, judged by a file name prefix in the
/// system and per-user Windows font folders (e.g. "inter" for Inter.ttc,
/// InterVariable.ttf or Inter-Regular.ttf).
fn font_installed(file_prefix: &str) -> bool {
    let mut dirs = vec![std::path::PathBuf::from(r"C:\Windows\Fonts")];
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(std::path::PathBuf::from(local).join(r"Microsoft\Windows\Fonts"));
    }
    dirs.iter().any(|dir| {
        std::fs::read_dir(dir)
            .map(|entries| {
                entries.flatten().any(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .to_ascii_lowercase()
                        .starts_with(file_prefix)
                })
            })
            .unwrap_or(false)
    })
}

/// Re-point FLTK's built-in faces at the Eclypsion type stack: Inter for the
/// interface and JetBrains Mono for values when they are installed, Segoe UI
/// and Consolas otherwise. Windows expects a one-character variant prefix
/// before the family (' ' plain, 'B' bold, 'I' italic, 'P' bold-italic);
/// FLTK keeps the pointer, so the CString is leaked.
fn install_fonts() {
    use std::ffi::CString;
    // Headings use weight 600: Segoe UI ships it as its own "Segoe UI
    // Semibold" family, Inter is bolded through the variant prefix.
    let (sans, sans_bold) = if font_installed("inter") {
        (" Inter", "BInter")
    } else {
        (" Segoe UI", " Segoe UI Semibold")
    };
    let mono = if font_installed("jetbrainsmono") {
        "JetBrains Mono"
    } else {
        "Consolas"
    };
    let family = &sans[1..];
    for (face, name) in [
        (0, sans.to_string()),
        (1, sans_bold.to_string()),
        (2, format!("I{family}")),
        (3, format!("P{family}")),
        (4, format!(" {mono}")),
        (5, format!("B{mono}")),
    ] {
        let name = CString::new(name).unwrap().into_raw();
        unsafe {
            fltk_sys::fl::Fl_set_font2(face, name);
        }
    }
}

/// Find the LAN address when the server binds to an unspecified address.
///
/// Windows can route ordinary traffic through a VPN or a virtual adapter, so a
/// plain UDP route probe is not enough: the phone on the local Wi-Fi still
/// needs the physical LAN address. `ipconfig` is used to enumerate IPv4
/// addresses and a private address is preferred; the UDP probe remains a
/// fallback.
#[cfg(target_os = "windows")]
fn guess_local_ip(ipv4: bool) -> Option<IpAddr> {
    if ipv4 {
        if let Some(ip) = local_ipv4_from_ipconfig() {
            return Some(IpAddr::V4(ip));
        }
    }
    udp_local_ip(ipv4)
}

#[cfg(target_os = "windows")]
fn local_ipv4_from_ipconfig() -> Option<std::net::Ipv4Addr> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let output = Command::new("ipconfig")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let mut candidates = Vec::new();
    for line in text.lines() {
        if !line.to_ascii_lowercase().contains("ipv4") {
            continue;
        }
        if let Some(value) = line.rsplit(':').next() {
            if let Ok(std::net::IpAddr::V4(ip)) = value.trim().parse::<IpAddr>() {
                if !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified() {
                    candidates.push(ip);
                }
            }
        }
    }
    candidates.sort_by_key(|ip| {
        let o = ip.octets();
        if o[0] == 192 && o[1] == 168 {
            0
        } else if o[0] == 10 {
            1
        } else if o[0] == 172 && (16..=31).contains(&o[1]) {
            2
        } else {
            3
        }
    });
    candidates.first().copied()
}

#[cfg(target_os = "windows")]
fn udp_local_ip(ipv4: bool) -> Option<IpAddr> {
    let (bind_addr, target) = if ipv4 {
        ("0.0.0.0:0", "8.8.8.8:80")
    } else {
        ("[::]:0", "[2001:4860:4860::8888]:80")
    };
    let socket = UdpSocket::bind(bind_addr).ok()?;
    socket.connect(target).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    if ip.is_unspecified() || ip.is_loopback() {
        None
    } else {
        Some(ip)
    }
}

pub fn run(config: &Config, log_receiver: mpsc::Receiver<String>) {
    install_fonts();

    // Rounded chrome used by the cards, fields and the log panel.
    app::set_frame_type_cb(
        field_frame(),
        draw_field_box,
        FIELD_PAD_X,
        FIELD_PAD_Y,
        2 * FIELD_PAD_X,
        2 * FIELD_PAD_Y,
    );
    app::set_frame_type_cb(card_frame(), draw_card_box, 0, 0, 0, 0);
    app::set_frame_type_cb(log_frame(), draw_log_box, 12, 10, 24, 20);

    // Desktop shell: a fixed navigation sidebar plus a page area holding the
    // "Главная" (home) and "Настройки" (settings) views.
    const NAV_W: i32 = 224;
    const WIN_W: i32 = 760;
    const WIN_H: i32 = 720;
    const CONTENT_W: i32 = WIN_W - NAV_W;
    const PAD: i32 = 28;
    const PAGE_W: i32 = CONTENT_W - 2 * PAD;

    let app = App::default().with_scheme(fltk::app::AppScheme::Gtk);
    // Create the icon before the window and keep it alive for the whole run:
    // FLTK stores a pointer to the image supplied here.
    let window_icon = app_icon();
    let mut wind = Window::default()
        .with_size(WIN_W, WIN_H)
        .center_screen()
        .with_label(&format!("Proximity - {}", env!("CARGO_PKG_VERSION")));
    wind.set_xclass("proximity");
    wind.set_callback(move |_win| app.quit());
    wind.set_frame(FrameType::FlatBox);
    wind.set_color(Color::from_rgb(14, 15, 18));
    wind.set_id("main_window");
    // Use the project mark instead of the default FLTK icon in the taskbar.
    if window_icon.is_some() {
        wind.set_icon(window_icon.clone());
    }
    // The layout is laid out in absolute pixels, so the shell keeps a fixed
    // size instead of stretching the cards.
    wind.size_range(WIN_W, WIN_H, WIN_W, WIN_H);

    // Ambient backdrop spans the whole window; the sidebar island and the
    // page widgets are painted on top of it.
    let mut app_glow = Frame::default()
        .with_size(WIN_W, WIN_H)
        .with_pos(0, 0)
        .with_id("app_glow");
    app_glow.draw(draw_page_glow);
    app_glow.super_draw(false);

    // --- Sidebar with the "Главная" / "Настройки" navigation --------------
    let mut nav_bar = Group::default()
        .with_pos(0, 0)
        .with_size(NAV_W, WIN_H)
        .with_id("nav_bar");

    // The rail itself: a floating glass island on the alternate surface.
    let mut nav_island = Frame::default()
        .with_size(200, WIN_H - 24)
        .with_pos(12, 12)
        .with_id("nav_island");
    nav_island.draw(draw_nav_island);
    nav_island.super_draw(false);

    Frame::default()
        .with_size(36, 36)
        .with_pos(28, 30)
        .with_align(Align::Center | Align::Inside.bits())
        .with_label("")
        .with_id("brand_mark");
    Frame::default()
        .with_size(120, 20)
        .with_pos(74, 29)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label("Proximity")
        .with_id("head_title");
    Frame::default()
        .with_size(120, 14)
        .with_pos(74, 50)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label(&format!("v{}", env!("CARGO_PKG_VERSION")))
        .with_id("head_sub");

    Frame::default()
        .with_size(160, 14)
        .with_pos(30, 100)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label("МЕНЮ")
        .with_id("nav_caption");

    let mut nav_home_btn = Button::default()
        .with_size(176, 40)
        .with_pos(24, 120)
        .with_label("Главная")
        .with_id("nav_home");
    nav_home_btn.draw(|b| draw_nav_item(b, true));
    nav_home_btn.super_draw(false);
    nav_home_btn.clear_visible_focus();
    let mut nav_settings_btn = Button::default()
        .with_size(176, 40)
        .with_pos(24, 164)
        .with_label("Настройки")
        .with_id("nav_settings");
    nav_settings_btn.draw(|b| draw_nav_item(b, false));
    nav_settings_btn.super_draw(false);
    nav_settings_btn.clear_visible_focus();

    // Server state card at the foot of the island: live dot, state and a
    // one-line hint underneath.
    let mut status_pill = Frame::default()
        .with_size(176, 56)
        .with_pos(24, WIN_H - 84)
        .with_align(Align::Center | Align::Inside.bits())
        .with_label("Сервер выключен")
        .with_id("status_pill");
    status_pill.draw(draw_status_pill);
    status_pill.super_draw(false);
    status_pill.set_tooltip("Shows whether the Proximity server is accepting connections.");

    // Perfectly rounded corners for the brand tile.
    if let Some(mut f) = app::widget_from_id::<Frame>("brand_mark") {
        f.draw(draw_brand_mark);
        f.super_draw(false);
    }

    nav_bar.end();

    // --- Home page ---------------------------------------------------------
    //
    //  ГЛАВНАЯ
    //  Управляй компьютером с телефона
    //  ┌ hero card ───────────────────────────────────────┐
    //  │ ┌ visual / QR ┐  state title                     │
    //  │ │             │  description                     │
    //  │ │             │  [ Запустить / Остановить ]      │
    //  │ └─────────────┘  [ http://address        ][⧉]   │
    //  └──────────────────────────────────────────────────┘
    //  [ порт ] [ доступ ] [ кодек ]
    //  ЖУРНАЛ                               СООБЩЕНИЯ СЕРВЕРА
    //  [ log                                              ]
    let mut page_home = Group::default()
        .with_pos(NAV_W, 0)
        .with_size(CONTENT_W, WIN_H)
        .with_id("page_home");

    Frame::default()
        .with_size(PAGE_W, 14)
        .with_pos(PAD, 30)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label("ГЛАВНАЯ")
        .with_id("home_eyebrow");

    Frame::default()
        .with_size(PAGE_W, 32)
        .with_pos(PAD, 48)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label("Управляй компьютером с телефона")
        .with_id("hero_title");

    Frame::default()
        .with_size(PAGE_W, 18)
        .with_pos(PAD, 82)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label("Запусти сервер и открой адрес в браузере на телефоне")
        .with_id("hero_sub");

    const HERO_Y: i32 = 120;
    const HERO_H: i32 = 212;
    const VIS: i32 = 172;
    const COL_X: i32 = PAD + 20 + VIS + 24;
    const COL_W: i32 = PAGE_W - 20 - VIS - 24 - 20;

    Frame::default()
        .with_size(PAGE_W, HERO_H)
        .with_pos(PAD, HERO_Y)
        .with_id("server_card");

    // Offline: the proximity mark as a slowly breathing radar. Live: the QR
    // code of the connect address takes its place.
    let mut hero_visual = Frame::default()
        .with_size(VIS, VIS)
        .with_pos(PAD + 20, HERO_Y + 20)
        .with_id("hero_visual");
    hero_visual.draw(draw_hero_visual);
    hero_visual.super_draw(false);

    let mut qr_frame = Frame::default()
        .with_size(VIS, VIS)
        .with_pos(PAD + 20, HERO_Y + 20)
        .with_align(Align::Center | Align::Inside.bits())
        .with_id("qr_frame");
    qr_frame.hide();

    Frame::default()
        .with_size(COL_W, 24)
        .with_pos(COL_X, HERO_Y + 24)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label("Сервер выключен")
        .with_id("state_title");

    Frame::default()
        .with_size(COL_W, 36)
        .with_pos(COL_X, HERO_Y + 50)
        .with_align(Align::Left | Align::Top | Align::Inside.bits())
        .with_label(STATE_DESC_OFF)
        .with_id("state_desc");

    let mut but_toggle = Button::default()
        .with_size(COL_W, 48)
        .with_pos(COL_X, HERO_Y + 96)
        .with_label("Запустить")
        .with_id("btn_toggle");
    but_toggle.draw(draw_primary_button);
    but_toggle.super_draw(false);
    but_toggle.clear_visible_focus();

    Frame::default()
        .with_size(COL_W, 36)
        .with_pos(COL_X, HERO_Y + 156)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label("Адрес появится после запуска")
        .with_id("offline_hint");

    let mut output_server_addr = Output::default()
        .with_size(COL_W - 44, 36)
        .with_pos(COL_X, HERO_Y + 156)
        .with_label("")
        .with_id("output_addr");
    output_server_addr.hide();

    let mut btn_copy = Button::default()
        .with_size(36, 36)
        .with_pos(COL_X + COL_W - 36, HERO_Y + 156)
        .with_label("")
        .with_id("btn_copy");
    btn_copy.draw(draw_copy_button);
    btn_copy.super_draw(false);
    btn_copy.clear_visible_focus();
    btn_copy.set_tooltip("Скопировать адрес");
    btn_copy.set_callback(|_| {
        if let Some(out) = app::widget_from_id::<Output>("output_addr") {
            app::copy(&out.value());
            MOTION.lock().unwrap().copied = 1.0;
        }
    });
    btn_copy.hide();

    // Three small tiles with the settings that matter when connecting.
    const TILE_Y: i32 = HERO_Y + HERO_H + 16;
    const TILE_W: i32 = (PAGE_W - 2 * 12) / 3;
    for (i, (id, tile)) in [
        ("tile_port", Tile::Port),
        ("tile_access", Tile::Access),
        ("tile_codec", Tile::Codec),
    ]
    .into_iter()
    .enumerate()
    {
        let mut f = Frame::default()
            .with_size(TILE_W, 76)
            .with_pos(PAD + i as i32 * (TILE_W + 12), TILE_Y)
            .with_id(id);
        f.draw(move |f| draw_stat_tile(f, tile));
        f.super_draw(false);
    }

    let log_caption_y = TILE_Y + 76 + 24;
    let log_y = log_caption_y + 20;
    Frame::default()
        .with_size(PAGE_W, 14)
        .with_pos(PAD, log_caption_y)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label("ЖУРНАЛ")
        .with_id("log_caption");

    Frame::default()
        .with_size(PAGE_W, 14)
        .with_pos(PAD, log_caption_y)
        .with_align(Align::Right | Align::Inside.bits())
        .with_label("СООБЩЕНИЯ СЕРВЕРА")
        .with_id("log_hint");

    let output_buf = TextBuffer::default();
    let mut output = TextDisplay::default()
        .with_size(PAGE_W, WIN_H - log_y - 28)
        .with_pos(PAD, log_y)
        .with_id("log_display");
    output.set_buffer(output_buf);
    let output_buf = output.buffer().unwrap();

    page_home.end();
    // fltk stores children of a group with window-absolute coordinates; the
    // page widgets above were authored relative to their page, so put them
    // where the page actually sits.
    shift_children(&mut page_home, NAV_W);

    // --- Settings page -----------------------------------------------------
    //
    // Grouped list cards like the web client: a title on the left of every
    // row, its control on the right, hairlines between the rows.
    let mut page_settings = Group::default()
        .with_pos(NAV_W, 0)
        .with_size(CONTENT_W, WIN_H)
        .with_id("page_settings");

    Frame::default()
        .with_size(PAGE_W, 14)
        .with_pos(PAD, 30)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label("НАСТРОЙКИ")
        .with_id("set_eyebrow");

    Frame::default()
        .with_size(PAGE_W, 32)
        .with_pos(PAD, 48)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label("Параметры сервера")
        .with_id("set_title");

    const ROW_X: i32 = PAD + 16;
    const ROW_W: i32 = PAGE_W - 32;
    const CTRL_W: i32 = 232;
    const CTRL_X: i32 = PAD + PAGE_W - 16 - CTRL_W;
    let row_label = |id: &str, label: &str, y: i32, h: i32| {
        Frame::default()
            .with_size(ROW_W - CTRL_W - 12, h)
            .with_pos(ROW_X, y)
            .with_align(Align::Left | Align::Inside.bits())
            .with_label(label)
            .with_id(id);
    };
    let divider = |id: &str, y: i32| {
        let mut f = Frame::default()
            .with_size(ROW_W, 1)
            .with_pos(ROW_X, y)
            .with_id(id);
        f.draw(draw_divider);
        f.super_draw(false);
    };

    // Appearance.
    const SKIN_Y: i32 = 120;
    Frame::default()
        .with_size(PAGE_W, 14)
        .with_pos(PAD, SKIN_Y - 20)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label("ОФОРМЛЕНИЕ")
        .with_id("sec_skin");

    Frame::default()
        .with_size(PAGE_W, 60)
        .with_pos(PAD, SKIN_Y)
        .with_id("card_skin");

    row_label("theme_label", "Тема", SKIN_Y, 60);

    let mut choice_theme = Choice::default()
        .with_size(CTRL_W, 36)
        .with_pos(CTRL_X, SKIN_Y + 12)
        .with_label("")
        .with_id("choice_theme");
    choice_theme.draw(draw_choice);
    choice_theme.super_draw(false);
    choice_theme.clear_visible_focus();

    for palette in PALETTES {
        choice_theme.add_choice(palette.name);
    }
    let selected = config
        .gui_theme
        .as_deref()
        .and_then(palette_by_name)
        .unwrap_or(DEFAULT_PALETTE);
    choice_theme.set_value(
        PALETTES
            .iter()
            .position(|p| p.name == selected.name)
            .unwrap_or(0) as i32,
    );

    // Network and access: three fields plus the autostart switch.
    const NET_Y: i32 = SKIN_Y + 60 + 40;
    const ROW_H: i32 = 56;
    const SWITCH_ROW_H: i32 = 64;
    Frame::default()
        .with_size(PAGE_W, 14)
        .with_pos(PAD, NET_Y - 20)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label("СЕТЬ И ДОСТУП")
        .with_id("sec_net");

    Frame::default()
        .with_size(PAGE_W, 3 * ROW_H + SWITCH_ROW_H)
        .with_pos(PAD, NET_Y)
        .with_id("card_net");

    for (id, label, row) in [
        ("label_access", "Код доступа", 0),
        ("label_bind", "Адрес привязки", 1),
        ("label_port", "Порт", 2),
    ] {
        row_label(id, label, NET_Y + row * ROW_H, ROW_H);
    }
    divider("div_net_1", NET_Y + ROW_H);
    divider("div_net_2", NET_Y + 2 * ROW_H);
    divider("div_net_3", NET_Y + 3 * ROW_H);

    let mut input_access_code = Input::default()
        .with_pos(CTRL_X, NET_Y + 10)
        .with_size(CTRL_W, 36)
        .with_id("input_access");
    input_access_code.set_tooltip(
        "Restrict who can control your computer with an access code. Note that this does NOT do \
        any kind of encryption and it is advised to only run Proximity inside trusted networks! Do \
        NOT reuse any of your passwords! If left blank, no code is required to access Proximity \
        remotely.",
    );
    if let Some(code) = config.access_code.as_ref() {
        input_access_code.set_value(code);
    }

    let mut input_bind_addr = Input::default()
        .with_size(CTRL_W, 36)
        .with_pos(CTRL_X, NET_Y + ROW_H + 10)
        .with_id("input_bind");
    input_bind_addr.set_value(&config.bind_address.to_string());

    let mut input_port = IntInput::default()
        .with_size(CTRL_W, 36)
        .with_pos(CTRL_X, NET_Y + 2 * ROW_H + 10)
        .with_id("input_port");
    input_port.set_value(&config.web_port.to_string());

    let auto_y = NET_Y + 3 * ROW_H;
    let mut check_auto_start = CheckButton::default()
        .with_size(ROW_W, 30)
        .with_pos(ROW_X, auto_y + 8)
        .with_label("Автозапуск")
        .with_id("check_auto");
    check_auto_start.draw(|c| draw_switch(c, Switch::Auto));
    check_auto_start.super_draw(false);
    check_auto_start.clear_visible_focus();
    check_auto_start.set_tooltip("Start Proximity server immediately on program start.");
    check_auto_start.set_checked(config.auto_start);

    Frame::default()
        .with_size(ROW_W - 60, 14)
        .with_pos(ROW_X, auto_y + 38)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label("Поднимать сервер при старте приложения")
        .with_id("desc_auto");

    // Hardware acceleration.
    const HW_Y: i32 = NET_Y + 3 * ROW_H + SWITCH_ROW_H + 40;
    Frame::default()
        .with_size(PAGE_W, 14)
        .with_pos(PAD, HW_Y - 20)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label("УСКОРЕНИЕ")
        .with_id("sec_hw");

    let hw_rows = if cfg!(target_os = "linux") { 3 } else { 2 };
    Frame::default()
        .with_size(PAGE_W, hw_rows * SWITCH_ROW_H)
        .with_pos(PAD, HW_Y)
        .with_id("card_hw");
    divider("div_hw_1", HW_Y + SWITCH_ROW_H);
    if hw_rows > 2 {
        divider("div_hw_2", HW_Y + 2 * SWITCH_ROW_H);
    }

    let mut check_native_hw_accel = CheckButton::default()
        .with_size(ROW_W, 30)
        .with_pos(ROW_X, HW_Y + 8)
        .with_id("check_native");
    check_native_hw_accel.draw(|c| draw_switch(c, Switch::Native));
    check_native_hw_accel.super_draw(false);
    check_native_hw_accel.clear_visible_focus();

    #[cfg(target_os = "linux")]
    let native_desc = "Кодирование через VAAPI";
    #[cfg(target_os = "macos")]
    let native_desc = "Кодирование через VideoToolbox";
    #[cfg(target_os = "windows")]
    let native_desc = "Аппаратное кодирование через Media Foundation";
    Frame::default()
        .with_size(ROW_W - 60, 14)
        .with_pos(ROW_X, HW_Y + 38)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label(native_desc)
        .with_id("desc_native");

    #[cfg(target_os = "linux")]
    {
        check_native_hw_accel.set_label("VAAPI");
        check_native_hw_accel
            .set_tooltip("Try to use hardware acceleration through the Video Acceleration API.");
        check_native_hw_accel.set_checked(config.try_vaapi);
    }

    #[cfg(target_os = "macos")]
    {
        check_native_hw_accel.set_label("VideoToolbox");
        check_native_hw_accel
            .set_tooltip("Try to use hardware acceleration through the VideoToolbox API.");
        check_native_hw_accel.set_checked(config.try_videotoolbox);
    }

    #[cfg(target_os = "windows")]
    {
        check_native_hw_accel.set_label("Media Foundation");
        check_native_hw_accel
            .set_tooltip("Try to use hardware acceleration through the MediaFoundation API.");
        check_native_hw_accel.set_checked(config.try_mediafoundation);
    }

    let mut check_nvenc = CheckButton::default()
        .with_size(ROW_W, 30)
        .with_pos(ROW_X, HW_Y + SWITCH_ROW_H + 8)
        .with_label("NVENC")
        .with_id("check_nvenc");
    check_nvenc.draw(|c| draw_switch(c, Switch::Nvenc));
    check_nvenc.super_draw(false);
    check_nvenc.clear_visible_focus();
    check_nvenc.set_tooltip("Try to use Nvidia's NVENC to encode the video via GPU.");

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        Frame::default()
            .with_size(ROW_W - 60, 14)
            .with_pos(ROW_X, HW_Y + SWITCH_ROW_H + 38)
            .with_align(Align::Left | Align::Inside.bits())
            .with_label("Кодирование через NVIDIA NVENC")
            .with_id("desc_nvenc");
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    check_nvenc.set_checked(config.try_nvenc);

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        check_nvenc.deactivate();
        check_nvenc.hide();
    }

    #[cfg(target_os = "linux")]
    let mut check_wayland = CheckButton::default()
        .with_size(ROW_W, 30)
        .with_pos(ROW_X, HW_Y + 2 * SWITCH_ROW_H + 8)
        .with_label("Wayland / PipeWire")
        .with_id("check_wayland");
    #[cfg(target_os = "linux")]
    {
        check_wayland.draw(|c| draw_switch(c, Switch::Wayland));
        check_wayland.super_draw(false);
        check_wayland.clear_visible_focus();
        check_wayland.set_tooltip(
            "EXPERIMENTAL! This may crash your desktop! Enables screen \
        capturing for Wayland using PipeWire and GStreamer.",
        );
        check_wayland.set_checked(config.wayland_support);
        Frame::default()
            .with_size(ROW_W - 60, 14)
            .with_pos(ROW_X, HW_Y + 2 * SWITCH_ROW_H + 38)
            .with_align(Align::Left | Align::Inside.bits())
            .with_label("Захват экрана через PipeWire (экспериментально)")
            .with_id("desc_wayland");
    }

    // The home tiles mirror these settings; repaint them on every edit.
    input_access_code.set_trigger(CallbackTrigger::Changed);
    input_access_code.set_callback(|_| redraw_tiles());
    input_port.set_trigger(CallbackTrigger::Changed);
    input_port.set_callback(|_| redraw_tiles());
    check_native_hw_accel.set_callback(|_| redraw_tiles());
    check_nvenc.set_callback(|_| redraw_tiles());

    page_settings.end();
    shift_children(&mut page_settings, NAV_W);
    page_settings.hide();

    nav_home_btn.set_callback(|_| show_page(true));
    nav_settings_btn.set_callback(|_| show_page(false));

    wind.make_resizable(true);
    wind.end();
    wind.show();
    // First paint with the palette from the config (or the default), then
    // show the home page.
    apply_palette(config.gui_theme.as_deref());
    // One ticker drives every small animation in the shell.
    app::add_timeout2(1.0 / 60.0, ui_motion_tick);

    let output_buf = Arc::new(Mutex::new(output_buf));

    std::thread::spawn(move || {
        while let Ok(log_message) = log_receiver.recv() {
            let mut output_buf = output_buf.lock().unwrap();
            output_buf.append(&log_message);
        }
    });

    let mut proximity = crate::proximity::Proximity::new();
    let mut is_server_running = false;
    let auto_start = config.auto_start;
    let config = Arc::new(Mutex::new(config.clone()));

    {
        let config = config.clone();
        choice_theme.set_callback(move |c| {
            let v = c.value();
            if v >= 0 && (v as usize) < PALETTES.len() {
                let palette = &PALETTES[v as usize];
                apply_palette(Some(palette.name));
                config.lock().unwrap().gui_theme = Some(palette.name.to_string());
                write_config(&config.lock().unwrap());
            }
        });
    }

    let mut toggle_server = move |but: &mut Button| {
        if let Err(err) = || -> Result<(), Box<dyn std::error::Error>> {
            let mut config = config.lock().unwrap();
            if !is_server_running {
                {
                    let access_code_string = input_access_code.value();
                    let access_code = match access_code_string.as_str() {
                        "" => None,
                        code => Some(code),
                    };
                    let bind_addr: IpAddr = input_bind_addr.value().parse()?;
                    let web_port: u16 = input_port.value().parse()?;

                    config.access_code = access_code.map(|s| s.to_string());
                    config.web_port = web_port;
                    config.bind_address = bind_addr;
                    config.auto_start = check_auto_start.is_checked();
                    config.gui_theme = PALETTES
                        .get(choice_theme.value().max(0) as usize)
                        .map(|p| p.name.to_string());
                    #[cfg(target_os = "linux")]
                    {
                        config.try_vaapi = check_native_hw_accel.is_checked();
                        config.wayland_support = check_wayland.is_checked();
                    }
                    #[cfg(any(target_os = "linux", target_os = "windows"))]
                    {
                        config.try_nvenc = check_nvenc.is_checked();
                    }
                    #[cfg(target_os = "macos")]
                    {
                        config.try_videotoolbox = check_native_hw_accel.is_checked();
                    }
                    #[cfg(target_os = "windows")]
                    {
                        config.try_mediafoundation = check_native_hw_accel.is_checked();
                    }
                }
                if !proximity.start(&config, |message| match message {
                    UInputInaccessible => awake_callback(move || {
                        let w = 500;
                        let h = 300;
                        let mut pop_up = Window::default()
                            .with_size(w, h)
                            .center_screen()
                            .with_label("Proximity - UInput inaccessible!");
                        pop_up.set_xclass("proximity");
                        pop_up.set_frame(FrameType::FlatBox);
                        pop_up.set_color(Color::from_rgb(14, 15, 18));

                        let buf = TextBuffer::default();
                        let mut pop_up_text = TextDisplay::default().with_size(w, h);
                        pop_up_text.set_buffer(buf);
                        pop_up_text.wrap_mode(fltk::text::WrapMode::AtBounds, 5);
                        pop_up_text.set_frame(FrameType::FlatBox);
                        pop_up_text.set_color(Color::from_rgb(14, 15, 18));
                        pop_up_text.set_text_color(Color::from_rgb(239, 240, 242));
                        pop_up_text.set_text_size(13);
                        let mut buf = pop_up_text.buffer().unwrap();
                        buf.set_text(std::include_str!("strings/uinput_error.txt"));

                        pop_up.end();
                        pop_up.make_modal(true);
                        pop_up.show();
                    }),
                }) {
                    return Ok(());
                }
                is_server_running = true;
                SERVER_LIVE.store(true, Ordering::Relaxed);
                kick_status(true);

                write_config(&config);

                let mut web_sock = SocketAddr::new(config.bind_address, config.web_port);

                #[cfg(not(target_os = "windows"))]
                {
                    if web_sock.ip().is_unspecified() {
                        // try to guess an ip
                        let mut ips = Vec::<IpAddr>::new();
                        for iface in datalink::interfaces()
                            .iter()
                            .filter(|iface| iface.is_up() && !iface.is_loopback())
                        {
                            for ipnetw in &iface.ips {
                                if (ipnetw.is_ipv4() && web_sock.ip().is_ipv4())
                                    || (ipnetw.is_ipv6() && web_sock.ip().is_ipv6())
                                {
                                    // filtering ipv6 unicast requires nightly or more fiddling,
                                    // lets wait for nightlies to stabilize...
                                    ips.push(ipnetw.ip())
                                }
                            }
                        }
                        if !ips.is_empty() {
                            web_sock.set_ip(ips[0]);
                        }
                        if ips.len() > 1 {
                            info!("Found more than one IP address for browsers to connect to,");
                            info!("other urls are:");
                            for ip in &ips[1..] {
                                info!("http://{}", SocketAddr::new(*ip, config.web_port));
                            }
                        }
                    }
                }

                #[cfg(target_os = "windows")]
                {
                    if web_sock.ip().is_unspecified() {
                        if let Some(ip) = guess_local_ip(web_sock.is_ipv4()) {
                            web_sock.set_ip(ip);
                        }
                    }
                }

                // Address and QR code, on every platform: the phone camera
                // opens the page straight from the code.
                let known_ip = !web_sock.ip().is_unspecified();
                let addr_string = if known_ip {
                    format!("http://{}", web_sock)
                } else {
                    "http://<your ip address>".to_string()
                };
                output_server_addr.set_value(&addr_string);
                if known_ip {
                    use image::Luma;
                    use qrcode::QrCode;
                    let mut url_string = addr_string;
                    if let Some(access_code) = &config.access_code {
                        url_string.push_str("?access_code=");
                        url_string.push_str(
                            &percent_encoding::utf8_percent_encode(
                                access_code,
                                percent_encoding::NON_ALPHANUMERIC,
                            )
                            .to_string(),
                        );
                    }

                    // The frame is a white rounded tile; the code sits inside
                    // it with the tile's padding as its quiet zone.
                    let cb = move |qr_frame: &mut Frame, _, _, w: i32, h: i32| {
                        let code = QrCode::new(&url_string).unwrap();
                        let img_buf = code.render::<Luma<u8>>().quiet_zone(false).build();
                        let image = image::DynamicImage::ImageLuma8(img_buf);
                        let dims = (min(w, h) - 2 * QR_PAD).max(16) as u32;
                        let image =
                            image.resize_exact(dims, dims, image::imageops::FilterType::Nearest);
                        let mut buf = vec![];
                        let mut cursor = Cursor::new(&mut buf);
                        image
                            .write_to(&mut cursor, image::ImageFormat::Png)
                            .unwrap();
                        let png = PngImage::from_data(&buf).unwrap();
                        qr_frame.set_image(Some(png));
                    };

                    let x = qr_frame.x();
                    let y = qr_frame.y();
                    let w = qr_frame.width();
                    let h = qr_frame.height();
                    cb(&mut qr_frame, x, y, w, h);
                    qr_frame.resize_callback(cb);
                    qr_frame.show();
                    if let Some(mut f) = app::widget_from_id::<Frame>("hero_visual") {
                        f.hide();
                    }
                }
                output_server_addr.show();
                but.set_label("Остановить");
                set_hero_state(true);
            } else {
                proximity.stop();
                but.set_label("Запустить");
                output_server_addr.hide();
                qr_frame.resize_callback(|_, _, _, _, _| {});
                qr_frame.hide();
                if let Some(mut f) = app::widget_from_id::<Frame>("hero_visual") {
                    f.show();
                }
                set_hero_state(false);
                is_server_running = false;
                SERVER_LIVE.store(false, Ordering::Relaxed);
                kick_status(false);
            }
            Ok(())
        }() {
            error!("{}", err);
        };
    };

    if auto_start {
        toggle_server(&mut but_toggle);
    }

    but_toggle.set_callback(toggle_server);

    app.run().expect("Failed to run Gui!");

    // TODO: Remove when https://github.com/fltk-rs/fltk-rs/issues/1480 is fixed
    // this is required to drop the callback and do a graceful shutdown of the web server
    but_toggle.set_callback(|_| ());
}

const BORDER: i32 = 30;

/// Which side page is currently visible (true = home). Kept in a static so the
/// nav callbacks and the skin can share the state without capturing widgets.
static HOME_ACTIVE: Mutex<bool> = Mutex::new(true);

/// Move every child of a group horizontally while keeping the group itself
/// untouched (fltk does not offset the coordinates of grouped widgets).
fn shift_children(g: &mut Group, dx: i32) {
    for i in 0..g.children() {
        if let Some(mut child) = g.child(i) {
            child.set_pos(child.x() + dx, child.y());
        }
    }
}

/// Switch between the "Главная" (home) and "Настройки" (settings) pages and
/// restyle the sidebar items to make the active entry obvious.
fn show_page(home: bool) {
    if let Some(mut g) = app::widget_from_id::<Group>("page_home") {
        if home {
            g.show();
        } else {
            g.hide();
        }
    }
    if let Some(mut g) = app::widget_from_id::<Group>("page_settings") {
        if home {
            g.hide();
        } else {
            g.show();
        }
    }
    *HOME_ACTIVE.lock().unwrap() = home;
    style_nav_items();
}

/// Redraw the two sidebar entries; their custom draw callbacks pick up the
/// active page and palette themselves.
fn style_nav_items() {
    for id in ["nav_home", "nav_settings"] {
        if let Some(mut b) = app::widget_from_id::<Button>(id) {
            b.redraw();
        }
    }
}

/// Glass skin applied on top of every widget, driven by the active palette
/// (see src/theme.rs). Every styled widget is looked up by id, so this can be
/// re-run at any time - in particular right after a theme switch, which
/// would otherwise keep stale colors around.
fn apply_skin() {
    let skin = skin();
    let bg = Color::from(skin.bg_end);
    let surface = Color::from(skin.surface);
    // Inputs sit a step above the card, like --color-fill-neutral on glass.
    let field = Color::from(field_color(&skin));
    let accent = Color::from(skin.accent);
    let accent_pressed = Color::from(skin.accent_pressed);
    let text = Color::from(skin.text);
    let dim = Color::from(skin.dim);
    let log_text = Color::from(skin.log_text);

    // Window chrome: the page background fills the frame, the sidebar island
    // and the page widgets paint on top of the ambient backdrop.
    if let Some(mut w) = app::widget_from_id::<Window>("main_window") {
        w.set_frame(FrameType::FlatBox);
        w.set_color(bg);
    }
    for id in ["nav_bar", "page_home", "page_settings"] {
        if let Some(mut g) = app::widget_from_id::<Group>(id) {
            g.set_frame(FrameType::NoBox);
        }
    }

    // Plain text frames: (id, font, size, color).
    let bold = Font::HelveticaBold;
    let regular = Font::Helvetica;
    let mono = Font::Courier;
    for (id, font, size, color) in [
        ("head_title", bold, 16, text),
        ("head_sub", mono, 11, dim),
        ("hero_title", bold, 24, text),
        ("hero_sub", regular, 13, dim),
        ("set_title", bold, 24, text),
        ("state_title", bold, 18, text),
        ("state_desc", regular, 13, dim),
        ("offline_hint", regular, 12, dim),
        ("theme_label", regular, 14, text),
        ("label_access", regular, 14, text),
        ("label_bind", regular, 14, text),
        ("label_port", regular, 14, text),
        ("desc_auto", regular, 12, dim),
        ("desc_native", regular, 12, dim),
        ("desc_nvenc", regular, 12, dim),
        ("desc_wayland", regular, 12, dim),
    ] {
        if let Some(mut f) = app::widget_from_id::<Frame>(id) {
            f.set_frame(FrameType::NoBox);
            f.set_label_font(font);
            f.set_label_size(size);
            f.set_label_color(color);
        }
    }

    // Eyebrow captions: uppercase, tracked, tertiary.
    for id in [
        "nav_caption",
        "home_eyebrow",
        "set_eyebrow",
        "sec_skin",
        "sec_net",
        "sec_hw",
        "log_caption",
        "log_hint",
    ] {
        if let Some(mut f) = app::widget_from_id::<Frame>(id) {
            f.set_frame(FrameType::NoBox);
            f.draw(draw_eyebrow);
            f.super_draw(false);
        }
    }

    // Glass cards.
    for id in ["server_card", "card_skin", "card_net", "card_hw"] {
        if let Some(mut f) = app::widget_from_id::<Frame>(id) {
            f.set_frame(card_frame());
            f.set_color(surface);
        }
    }

    if let Some(mut f) = app::widget_from_id::<Frame>("status_pill") {
        f.set_label_size(12);
    }
    if let Some(mut b) = app::widget_from_id::<Button>("btn_toggle") {
        b.set_label_color(Color::White);
        b.set_label_size(14);
        b.set_label_font(Font::HelveticaBold);
        b.set_selection_color(accent_pressed);
    }
    if let Some(mut w) = app::widget_from_id::<Output>("output_addr") {
        w.set_frame(field_frame());
        w.set_color(field);
        w.set_label_color(dim);
        w.set_text_color(text);
        w.set_text_font(Font::Courier);
        w.set_text_size(12);
    }
    for id in ["input_access", "input_bind"] {
        if let Some(mut w) = app::widget_from_id::<Input>(id) {
            w.set_frame(field_frame());
            w.set_color(field);
            w.set_text_color(text);
            w.set_text_font(Font::Helvetica);
            w.set_text_size(13);
            w.set_cursor_color(accent);
            w.set_selection_color(accent);
        }
    }
    if let Some(mut w) = app::widget_from_id::<IntInput>("input_port") {
        w.set_frame(field_frame());
        w.set_color(field);
        w.set_text_color(text);
        w.set_text_font(Font::Courier);
        w.set_text_size(13);
        w.set_cursor_color(accent);
        w.set_selection_color(accent);
    }
    if let Some(mut c) = app::widget_from_id::<Choice>("choice_theme") {
        // These colors style the popup list FLTK opens for the choice.
        c.set_color(Color::from(skin.surface));
        c.set_text_color(text);
        c.set_text_size(13);
        c.set_selection_color(accent);
        c.redraw();
    }
    for id in ["check_auto", "check_native", "check_nvenc", "check_wayland"] {
        if let Some(mut c) = app::widget_from_id::<CheckButton>(id) {
            c.set_label_color(text);
            c.set_label_size(14);
            c.set_color(surface);
            c.set_selection_color(accent);
            c.redraw();
        }
    }
    if let Some(mut t) = app::widget_from_id::<TextDisplay>("log_display") {
        t.set_frame(log_frame());
        t.set_color(surface);
        t.set_text_color(log_text);
        t.set_text_font(Font::Courier);
        t.set_text_size(11);
        t.set_selection_color(accent);
    }
    if let Some(mut f) = app::widget_from_id::<Frame>("qr_frame") {
        f.set_frame(card_frame());
        f.set_color(Color::White);
    }
}

/// Fill of text fields and the theme picker: the card surface lifted a step
/// towards the text color (Eclypsion --color-fill-neutral).
fn field_color(skin: &Skin) -> Rgb24 {
    skin.surface.mix(skin.pill, 550)
}

/// Repaint the three home tiles after a setting they mirror changed.
fn redraw_tiles() {
    for id in ["tile_port", "tile_access", "tile_codec"] {
        if let Some(mut f) = app::widget_from_id::<Frame>(id) {
            f.redraw();
        }
    }
}

const STATE_DESC_OFF: &str = "Запусти сервер, затем открой адрес\nна телефоне в той же сети";
const STATE_DESC_ON: &str = "Наведи камеру телефона на QR-код\nили открой адрес в браузере";

/// Padding of the white QR tile around the code (its quiet zone).
const QR_PAD: i32 = 14;

/// Swap the hero texts and the copy button between the stopped and the
/// running server.
fn set_hero_state(live: bool) {
    if let Some(mut f) = app::widget_from_id::<Frame>("state_title") {
        f.set_label(if live { "Сервер работает" } else { "Сервер выключен" });
    }
    if let Some(mut f) = app::widget_from_id::<Frame>("state_desc") {
        f.set_label(if live { STATE_DESC_ON } else { STATE_DESC_OFF });
    }
    if let Some(mut f) = app::widget_from_id::<Frame>("offline_hint") {
        if live {
            f.hide();
        } else {
            f.show();
        }
    }
    if let Some(mut b) = app::widget_from_id::<Button>("btn_copy") {
        if live {
            b.show();
        } else {
            b.hide();
        }
    }
    if let Some(mut f) = app::widget_from_id::<Frame>("server_card") {
        f.redraw();
    }
    if let Some(mut f) = app::widget_from_id::<Frame>("status_pill") {
        f.set_label(if live { "Сервер работает" } else { "Сервер выключен" });
        f.redraw();
    }
    // Parent repaint so the labels never leave ghosts on the card.
    if let Some(mut g) = app::widget_from_id::<Group>("page_home") {
        g.redraw();
    }
}

/// FLTK keeps a range of free box-type slots; the redesign registers its
/// rounded chrome boxes there instead of painting every widget by hand.
fn free_frame(offset: i32) -> FrameType {
    let base = FrameType::FreeBoxType.as_i32();
    // Safety: `from_i32` only wraps the box index, which FLTK stores as-is.
    unsafe { FrameType::UserFrameType(UnmappedFrameType::from_i32(base + offset)) }
}

/// Inner padding of the field box, in pixels (left, top and the sums).
const FIELD_PAD_X: i32 = 14;
const FIELD_PAD_Y: i32 = 9;

fn field_frame() -> FrameType {
    free_frame(0)
}
fn card_frame() -> FrameType {
    free_frame(1)
}
fn log_frame() -> FrameType {
    free_frame(2)
}

/// Eclypsion radii: fields and buttons use --radius-lg, cards --radius-xl and
/// the floating islands --radius-2xl.
const RADIUS_LG: i32 = 8;
const RADIUS_XL: i32 = 12;
const RADIUS_2XL: i32 = 16;

/// One-pixel light edge along the top of a raised surface: FLTK has no inset
/// shadows, so this line stands in for the web client's glass highlight.
fn draw_top_highlight(x: i32, y: i32, w: i32, base: Rgb24, inset: i32) {
    let skin = skin();
    if skin.light || w <= 2 * inset {
        return;
    }
    draw::set_draw_color(Color::from(base.mix(Rgb24::hex(0xffffff), 55)));
    draw::draw_line(x + inset, y + 1, x + w - inset - 1, y + 1);
}

fn rgb24_of(color: Color) -> Rgb24 {
    let (r, g, b) = color.to_rgb();
    Rgb24::hex(((r as u32) << 16) | ((g as u32) << 8) | b as u32)
}

fn draw_field_box(x: i32, y: i32, w: i32, h: i32, color: Color) {
    let skin = skin();
    draw::set_draw_color(color);
    draw::draw_rounded_rectf(x, y, w, h, RADIUS_LG);
    draw::set_draw_color(Color::from(skin.border));
    draw::draw_rounded_rect(x, y, w - 1, h - 1, RADIUS_LG);
}

fn draw_card_box(x: i32, y: i32, w: i32, h: i32, color: Color) {
    let skin = skin();
    draw::set_draw_color(color);
    draw::draw_rounded_rectf(x, y, w, h, RADIUS_XL);
    draw_top_highlight(x, y, w, rgb24_of(color), RADIUS_XL);
    draw::set_draw_color(Color::from(skin.border));
    draw::draw_rounded_rect(x, y, w - 1, h - 1, RADIUS_XL);
}

fn draw_log_box(x: i32, y: i32, w: i32, h: i32, color: Color) {
    let skin = skin();
    draw::set_draw_color(color);
    draw::draw_rounded_rectf(x, y, w, h, RADIUS_XL);
    draw::set_draw_color(Color::from(skin.border));
    draw::draw_rounded_rect(x, y, w - 1, h - 1, RADIUS_XL);
}

/// A soft radial bloom: concentric discs from the rim inwards, each one a
/// little closer to `tint`. FLTK has no alpha, so the falloff is baked into
/// the colors; the per-step change stays below one 8-bit level, so no banding
/// shows. `strength` is the centre mix in thousandths.
fn draw_bloom(cx: i32, cy: i32, radius: i32, base: Rgb24, tint: Rgb24, strength: u32) {
    const STEPS: i32 = 48;
    for i in 0..STEPS {
        let d = 2 * radius * (STEPS - i) / STEPS;
        if d < 2 {
            break;
        }
        // Quadratic falloff from the rim (0) to the centre (strength).
        let t = (i + 1) as u32 * 1000 / STEPS as u32;
        let mix = strength * t / 1000 * t / 1000;
        fill_circle(cx, cy, d, Color::from(base.mix(tint, mix)));
    }
}

/// Ambient backdrop: the flat page color lit by two soft blooms - the accent
/// companion from the top left and a cool teal from the bottom right - the
/// same aurora the web client's glass panels sit on. The blooms do not
/// overlap, so each can paint its own rim in the base color.
fn draw_page_glow(f: &mut Frame) {
    let skin = skin();
    let (x, y) = (f.x(), f.y());
    let (w, h) = (f.w(), f.h());
    draw::set_draw_color(Color::from(skin.bg_end));
    draw::draw_rectf(x, y, w, h);
    let (main, side) = if skin.light { (170, 110) } else { (260, 110) };
    draw_bloom(
        x + w * 30 / 100,
        y - h / 10,
        w * 70 / 100,
        skin.bg_end,
        skin.bloom,
        main,
    );
    draw_bloom(
        x + w + 40,
        y + h + 60,
        w * 56 / 100,
        skin.bg_end,
        Rgb24::hex(0x35c9c2),
        side,
    );
}

/// Primary action of the hero card. Stopped: a solid accent button with a
/// glass sheen and a soft accent shadow. Running: a calm danger-subtle
/// "stop" button (Eclypsion --color-fill-danger-subtle + text-danger).
fn draw_primary_button(b: &mut Button) {
    let skin = skin();
    let hover = {
        let m = MOTION.lock().unwrap();
        m.primary
    };
    let (x, y, w, h) = (b.x(), b.y(), b.w(), b.h());
    let live = SERVER_LIVE.load(Ordering::Relaxed);
    let pressed = b.value();
    // The press is painted as a 2px smaller button.
    let inset = if pressed { 2 } else { 0 };
    // The bottom 4px of the widget hold the shadow, so everything stays
    // inside the widget's own box.
    const SHADOW: i32 = 4;
    let (bx, by, bw, bh) = (x + inset, y + inset, w - 2 * inset, h - SHADOW - 2 * inset);

    // Clear the widget area to the card surface first: the button switches
    // between shapes and the old one must not linger.
    draw::set_draw_color(Color::from(skin.surface));
    draw::draw_rectf(x, y, w, h);

    if live {
        let fill = skin
            .surface
            .mix(skin.danger, 150 + (70.0 * hover) as u32 + if pressed { 60 } else { 0 });
        draw::set_draw_color(Color::from(fill));
        draw::draw_rounded_rectf(bx, by, bw, bh, RADIUS_LG);
        draw::set_draw_color(Color::from(skin.surface.mix(skin.danger, 320)));
        draw::draw_rounded_rect(bx, by, bw - 1, bh - 1, RADIUS_LG);
        let label = if skin.light {
            skin.danger.mix(Rgb24::hex(0x000000), 160)
        } else {
            skin.danger.mix(Rgb24::hex(0xffffff), 380)
        };
        draw::set_font(Font::HelveticaBold, 14);
        draw::set_draw_color(Color::from(label));
        draw::draw_text2(&b.label(), bx, by, bw, bh, Align::Center);
        return;
    }

    let base = if pressed {
        skin.accent.mix(Rgb24::hex(0x000000), 120)
    } else {
        skin.accent.mix(skin.accent_pressed, (1000.0 * hover) as u32)
    };
    let halo = skin.surface.mix(skin.accent, 220);
    draw::set_draw_color(Color::from(halo));
    draw::draw_rounded_rectf(x + 8, y + SHADOW, w - 16, h - SHADOW, RADIUS_LG + 2);
    draw::set_draw_color(Color::from(base));
    draw::draw_rounded_rectf(bx, by, bw, bh, RADIUS_LG);
    // Glass sheen: the upper half is lifted a touch, plus a crisp top edge.
    draw::set_draw_color(Color::from(base.mix(Rgb24::hex(0xffffff), 45)));
    draw::draw_rounded_rectf(bx, by, bw, bh / 2, RADIUS_LG);
    draw::draw_rectf(bx, by + RADIUS_LG, bw, bh / 2 - RADIUS_LG);
    draw::set_draw_color(Color::from(base.mix(Rgb24::hex(0xffffff), 170)));
    draw::draw_line(bx + RADIUS_LG, by + 1, bx + bw - RADIUS_LG - 1, by + 1);
    draw::set_font(Font::HelveticaBold, 14);
    draw::set_draw_color(Color::White);
    draw::draw_text2(&b.label(), bx, by, bw, bh, Align::Center);
}

/// Theme picker: a field-shaped pill showing the active palette plus a
/// chevron; the popup itself is handled by FLTK.
fn draw_choice(c: &mut Choice) {
    let skin = skin();
    let hover = {
        let m = MOTION.lock().unwrap();
        m.choice
    };
    let (x, y, w, h) = (c.x(), c.y(), c.w(), c.h());
    draw::set_draw_color(Color::from(
        field_color(&skin).mix(skin.pill, (700.0 * hover) as u32),
    ));
    draw::draw_rounded_rectf(x, y, w, h, RADIUS_LG);
    draw::set_draw_color(Color::from(skin.border));
    draw::draw_rounded_rect(x, y, w - 1, h - 1, RADIUS_LG);
    // A swatch of the palette's accent in front of its name.
    let swatch = PALETTES
        .get(c.value().max(0) as usize)
        .map(|p| p.accent)
        .unwrap_or(skin.accent);
    fill_circle(x + 18, y + h / 2, 12, Color::from(swatch));
    let label = c.text(c.value().max(0)).unwrap_or_default();
    draw::set_font(Font::Helvetica, 13);
    draw::set_draw_color(Color::from(skin.text));
    draw::draw_text(&label, x + 32, y + h / 2 + 5);
    let cx = x + w - 20;
    let cy = y + h / 2;
    draw::set_draw_color(Color::from(skin.dim));
    draw::draw_line(cx - 4, cy - 2, cx, cy + 2);
    draw::draw_line(cx, cy + 2, cx + 4, cy - 2);
}

/// Draw a filled circle centred on (cx, cy). FLTK's primitive takes the
/// top-left corner of the bounding box, which is easy to get wrong.
fn fill_circle(cx: i32, cy: i32, d: i32, color: Color) {
    draw::draw_circle_fill(cx - d / 2, cy - d / 2, d, color);
}




/// Toggle switch drawn in place of the stock check box: an iOS-like 42x26
/// track with a 20px white knob and a soft shadow, the title on the left.
/// The knob follows the shared spring, so toggling has a small overshoot.
fn draw_switch(c: &mut CheckButton, which: Switch) {
    let skin = skin();
    let p = switch_spring(which).clamp(-0.08, 1.08);
    let track_w = 42;
    let track_h = 26;
    let (x, y) = (c.x(), c.y());
    let tx = x + c.w() - track_w;
    let ty = y + (c.h() - track_h) / 2;
    let cy = ty + track_h / 2;
    let on = p.clamp(0.0, 1.0);
    let track = skin.track.mix(skin.accent, (1000.0 * on) as u32);
    draw::set_draw_color(Color::from(track));
    draw::draw_rounded_rectf(tx, ty, track_w, track_h, track_h / 2);

    let title_w = (c.w() - track_w - 16).max(80);
    draw::set_font(Font::Helvetica, 14);
    draw::set_draw_color(Color::from(if c.active() {
        skin.text
    } else {
        skin.dim
    }));
    draw::draw_text2(
        &c.label(),
        x,
        ty,
        title_w,
        track_h,
        Align::Left | Align::Inside,
    );

    let knob_d = 20;
    let pad = 3;
    let travel = (track_w - knob_d - 2 * pad) as f32;
    let kx = tx + pad + knob_d / 2 + (travel * p) as i32;
    let shadow = track.mix(Rgb24::hex(0x000000), 260);
    fill_circle(kx, cy + 1, knob_d + 2, Color::from(shadow));
    fill_circle(kx, cy, knob_d, Color::White);
}

/// Sidebar entry: a soft raised pill for the active page (with the glass top
/// edge) plus a crisp SVG line icon, accent-tinted when active.
fn draw_nav_item(b: &mut Button, home: bool) {
    let skin = skin();
    let active = *HOME_ACTIVE.lock().unwrap() == home;
    let hover = hover_value(home);
    let hovered = hover > 0.02;
    let (x, y, w, h) = (b.x(), b.y(), b.w(), b.h());
    let pressed = b.value();
    let inset = if pressed { 1 } else { 0 };
    if active {
        draw::set_draw_color(Color::from(skin.pill));
        draw::draw_rounded_rectf(x + inset, y + inset, w - 2 * inset, h - 2 * inset, RADIUS_LG + 2);
        draw_top_highlight(x + inset, y + inset, w - 2 * inset, skin.pill, RADIUS_LG + 2);
    } else if hovered {
        draw::set_draw_color(Color::from(
            skin.nav_bg.mix(skin.pill, (550.0 * hover) as u32),
        ));
        draw::draw_rounded_rectf(x + inset, y + inset, w - 2 * inset, h - 2 * inset, RADIUS_LG + 2);
    }
    let icon_color = if active {
        skin.accent_soft
    } else {
        // Quantise the hover fade into a few cached glyphs: smooth to the eye,
        // but no SVG re-parse for every intermediate colour.
        let step = (hover * 5.0).round() as u32;
        skin.dim.mix(skin.text, step * 200)
    };
    draw_glyph(
        if home {
            ICON_HOME_SVG
        } else {
            ICON_SETTINGS_SVG
        },
        icon_color,
        x + 12,
        y + (h - 20) / 2,
        20,
        20,
    );
    draw::set_draw_color(Color::from(if active {
        skin.text
    } else {
        skin.dim.mix(skin.text, (1000.0 * hover) as u32)
    }));
    draw::set_font(
        if active {
            Font::HelveticaBold
        } else {
            Font::Helvetica
        },
        13,
    );
    draw::draw_text2(
        &b.label(),
        x + 42,
        y,
        w - 52,
        h,
        Align::Left | Align::Inside,
    );
}

/// Small caps eyebrow labels get the design set's letter spacing; FLTK has
/// no tracking, so the string is drawn character by character.
fn draw_eyebrow(f: &mut Frame) {
    let skin = skin();
    let (x, y, w, h) = (f.x(), f.y(), f.w(), f.h());
    let label = f.label();
    if label.is_empty() {
        return;
    }
    draw::set_font(Font::HelveticaBold, 10);
    draw::set_draw_color(Color::from(skin.dim));
    let spacing = 1.2f64;
    let mut total = spacing * label.chars().count().saturating_sub(1) as f64;
    for ch in label.chars() {
        total += draw::width(&ch.to_string());
    }
    let right = f.align().bits() & Align::Right.bits() != 0;
    let mut cx = if right { (x + w) as f64 - total } else { x as f64 };
    let baseline = y + (h + 7) / 2;
    for ch in label.chars() {
        let glyph = ch.to_string();
        draw::draw_text(&glyph, cx as i32, baseline);
        cx += draw::width(&glyph) + spacing;
    }
}

/// Hairline between the rows of a grouped settings card.
fn draw_divider(f: &mut Frame) {
    let skin = skin();
    draw::set_draw_color(Color::from(skin.surface.mix(skin.pill, 700)));
    draw::draw_line(f.x(), f.y(), f.x() + f.w() - 1, f.y());
}

/// Hero visual of the stopped server: a soft accent tile carrying the
/// proximity mark as a radar - two rings, the dot and the satellite. Static.
fn draw_hero_visual(f: &mut Frame) {
    let skin = skin();
    let (x, y, w, h) = (f.x(), f.y(), f.w(), f.h());
    // Tile: the card surface tinted with the accent, plus the glass edge.
    let tile = skin.surface.mix(skin.accent, if skin.light { 70 } else { 110 });
    draw::set_draw_color(Color::from(tile));
    draw::draw_rounded_rectf(x, y, w, h, RADIUS_XL);
    draw_top_highlight(x, y, w, tile, RADIUS_XL);
    draw::set_draw_color(Color::from(tile.mix(skin.accent_soft, 160)));
    draw::draw_rounded_rect(x, y, w - 1, h - 1, RADIUS_XL);

    let cx = (x + w / 2) as f64;
    let cy = (y + h / 2) as f64;
    // A soft glow in the middle.
    for (d, mix) in [(118, 60u32), (96, 110), (76, 160)] {
        fill_circle(cx as i32, cy as i32, d, Color::from(tile.mix(skin.accent, mix)));
    }

    // The mark: outer faint ring, main ring (drawn twice for weight), the
    // centre dot and the satellite at 45 degrees.
    draw::set_draw_color(Color::from(tile.mix(skin.accent_soft, 380)));
    draw::draw_circle(cx, cy, 42.0);
    draw::set_draw_color(Color::from(skin.accent_soft));
    draw::draw_circle(cx, cy, 28.0);
    draw::draw_circle(cx, cy, 27.0);
    draw::draw_circle(cx, cy, 29.0);
    fill_circle(cx as i32, cy as i32, 18, Color::from(skin.accent_soft));
    let sat = 28.0 * std::f64::consts::FRAC_1_SQRT_2;
    fill_circle(
        (cx + sat) as i32,
        (cy - sat) as i32,
        14,
        Color::from(tile),
    );
    fill_circle(
        (cx + sat) as i32,
        (cy - sat) as i32,
        10,
        Color::from(skin.accent_soft),
    );
}

/// Square ghost button next to the address field: a copy glyph that turns
/// into a check for a moment after the address was copied.
fn draw_copy_button(b: &mut Button) {
    let skin = skin();
    let (x, y, w, h) = (b.x(), b.y(), b.w(), b.h());
    let copied = {
        let m = MOTION.lock().unwrap();
        m.copied
    };
    let fill = if b.value() {
        skin.surface.mix(skin.pill, 1000)
    } else {
        field_color(&skin)
    };
    draw::set_draw_color(Color::from(fill));
    draw::draw_rounded_rectf(x, y, w, h, RADIUS_LG);
    draw::set_draw_color(Color::from(skin.border));
    draw::draw_rounded_rect(x, y, w - 1, h - 1, RADIUS_LG);
    let (glyph, color) = if copied > 0.0 {
        (ICON_CHECK_SVG, skin.success)
    } else {
        (ICON_COPY_SVG, skin.log_text)
    };
    draw_glyph(glyph, color, x + (w - 16) / 2, y + (h - 16) / 2, 16, 16);
}

/// The settings mirrored on the home page.
#[derive(Clone, Copy)]
enum Tile {
    Port,
    Access,
    Codec,
}

/// Small glass tile: eyebrow caption, a value and a status dot. Values are
/// read from the settings widgets at paint time, so they are always current.
fn draw_stat_tile(f: &mut Frame, tile: Tile) {
    let skin = skin();
    let (x, y, w, h) = (f.x(), f.y(), f.w(), f.h());
    draw_card_box(x, y, w, h, Color::from(skin.surface));

    let (caption, value, mono, dot) = match tile {
        Tile::Port => {
            let port = app::widget_from_id::<IntInput>("input_port")
                .map(|i| i.value())
                .unwrap_or_default();
            ("ПОРТ", if port.is_empty() { "—".to_string() } else { port }, true, None)
        }
        Tile::Access => {
            let protected = app::widget_from_id::<Input>("input_access")
                .map(|i| !i.value().is_empty())
                .unwrap_or(false);
            if protected {
                ("ДОСТУП", "По коду".to_string(), false, Some(skin.success))
            } else {
                ("ДОСТУП", "Открытый".to_string(), false, Some(skin.dim))
            }
        }
        Tile::Codec => {
            let value = if checked("check_nvenc") {
                "NVENC".to_string()
            } else if checked("check_native") {
                app::widget_from_id::<CheckButton>("check_native")
                    .map(|c| c.label())
                    .unwrap_or_default()
            } else {
                "x264 · CPU".to_string()
            };
            ("КОДЕК", value, false, None)
        }
    };

    // Caption with the eyebrow's tracking.
    draw::set_font(Font::HelveticaBold, 10);
    draw::set_draw_color(Color::from(skin.dim));
    let mut cx = (x + 16) as f64;
    for ch in caption.chars() {
        let glyph = ch.to_string();
        draw::draw_text(&glyph, cx as i32, y + 26);
        cx += draw::width(&glyph) + 1.2;
    }

    let mut vx = x + 16;
    if let Some(dot) = dot {
        fill_circle(vx + 4, y + 50, 8, Color::from(dot));
        vx += 16;
    }
    draw::set_font(if mono { Font::Courier } else { Font::HelveticaBold }, 16);
    draw::set_draw_color(Color::from(skin.text));
    draw::draw_text2(&value, vx, y + 38, w - (vx - x) - 12, 24, Align::Left | Align::Inside);
}

/// Sidebar island: the floating rail the navigation lives on.
fn draw_nav_island(f: &mut Frame) {
    let skin = skin();
    let (x, y, w, h) = (f.x(), f.y(), f.w(), f.h());
    draw::set_draw_color(Color::from(skin.nav_bg));
    draw::draw_rounded_rectf(x, y, w, h, RADIUS_2XL);
    // A one-pixel highlight along the top edge, like the glass islands.
    draw_top_highlight(x, y, w, skin.nav_bg, RADIUS_2XL);
    draw::set_draw_color(Color::from(skin.border));
    draw::draw_rounded_rect(x, y, w - 1, h - 1, RADIUS_2XL);
}

/// Brand tile: a rounded accent square carrying the proximity mark.
fn draw_brand_mark(f: &mut Frame) {
    let skin = skin();
    let (x, y, w, h) = (f.x(), f.y(), f.w(), f.h());
    // A soft accent halo lifts the tile from the nav island.
    draw::set_draw_color(Color::from(skin.nav_bg.mix(skin.accent, 130)));
    draw::draw_rounded_rectf(x - 2, y - 1, w + 4, h + 5, RADIUS_LG + 3);
    draw::set_draw_color(Color::from(skin.accent));
    draw::draw_rounded_rectf(x, y, w, h, RADIUS_LG + 2);
    // Glass sheen on the upper half and a crisp top edge, like the web tile.
    draw::set_draw_color(Color::from(skin.accent.mix(Rgb24::hex(0xffffff), 90)));
    draw::draw_rounded_rectf(x, y, w, h / 2, RADIUS_LG + 2);
    draw::draw_rectf(x, y + RADIUS_LG + 2, w, h / 2 - RADIUS_LG - 2);
    draw::set_draw_color(Color::from(skin.accent.mix(Rgb24::hex(0xffffff), 220)));
    draw::draw_line(x + 9, y + 1, x + w - 9, y + 1);
    draw::set_draw_color(Color::from(skin.accent.mix(Rgb24::hex(0x000000), 120)));
    draw::draw_rounded_rect(x, y, w - 1, h - 1, RADIUS_LG + 2);
    draw_glyph(
        LOGO_GLYPH_SVG,
        Rgb24::hex(0xffffff),
        x + 5,
        y + 5,
        w - 10,
        h - 10,
    );
}

/// Whether the proximity server is currently accepting connections.
static SERVER_LIVE: AtomicBool = AtomicBool::new(false);

/// Motion state of the shell: hover fades, switch springs, the status
/// crossfade and the copy check. A single 60 Hz ticker advances everything and
/// only redraws widgets whose value actually moved.
struct UiMotion {
    nav_home: f32,
    nav_settings: f32,
    primary: f32,
    choice: f32,
    sw_auto: Spring,
    sw_native: Spring,
    sw_nvenc: Spring,
    sw_wayland: Spring,
    status: Spring,
    /// Countdown of the "copied" check on the copy button.
    copied: f32,
}

#[derive(Clone, Copy)]
struct Spring {
    x: f32,
    v: f32,
}

#[derive(Clone, Copy)]
enum Switch {
    Auto,
    Native,
    Nvenc,
    Wayland,
}

static MOTION: Mutex<UiMotion> = Mutex::new(UiMotion {
    nav_home: 0.0,
    nav_settings: 0.0,
    primary: 0.0,
    choice: 0.0,
    sw_auto: Spring { x: 0.0, v: 0.0 },
    sw_native: Spring { x: 0.0, v: 0.0 },
    sw_nvenc: Spring { x: 0.0, v: 0.0 },
    sw_wayland: Spring { x: 0.0, v: 0.0 },
    status: Spring { x: 0.0, v: 0.0 },
    copied: 0.0,
});

fn pointer_over<W>(id: &str) -> bool
where
    W: WidgetExt + WidgetBase + Clone + 'static,
{
    match (app::widget_from_id::<W>(id), app::belowmouse::<W>()) {
        (Some(target), Some(under)) => target.as_widget_ptr() == under.as_widget_ptr(),
        _ => false,
    }
}

fn checked(id: &str) -> bool {
    app::widget_from_id::<CheckButton>(id)
        .map(|c| c.is_checked())
        .unwrap_or(false)
}

/// Exponential approach, frame-rate independent enough at a fixed 60 Hz tick.
fn approach(cur: &mut f32, target: f32, k: f32) -> bool {
    let diff = target - *cur;
    if diff.abs() <= 0.002 {
        if *cur == target {
            return false;
        }
        *cur = target;
        return true;
    }
    *cur += diff * k;
    true
}

/// Under-damped spring, the launcher's "spring-snap" (slight overshoot).
fn spring_step(s: &mut Spring, target: f32, dt: f32) -> bool {
    const STIFFNESS: f32 = 400.0;
    const DAMPING: f32 = 26.0;
    let force = STIFFNESS * (target - s.x) - DAMPING * s.v;
    s.v += force * dt;
    s.x += s.v * dt;
    if (target - s.x).abs() < 0.002 && s.v.abs() < 0.02 {
        if s.x == target && s.v == 0.0 {
            return false;
        }
        s.x = target;
        s.v = 0.0;
        return true;
    }
    true
}

fn redraw_id(id: &str) {
    if let Some(mut b) = app::widget_from_id::<Button>(id) {
        b.redraw();
        return;
    }
    if let Some(mut c) = app::widget_from_id::<Choice>(id) {
        c.redraw();
        return;
    }
    if let Some(mut c) = app::widget_from_id::<CheckButton>(id) {
        c.redraw();
        return;
    }
    if let Some(mut f) = app::widget_from_id::<Frame>(id) {
        f.redraw();
    }
}

fn ui_motion_tick() {
    let dt = 0.016f32;
    let hover_k = 0.24;
    let mut dirty: Vec<&'static str> = Vec::new();
    let mut m = MOTION.lock().unwrap();

    if m.copied > 0.0 {
        m.copied = (m.copied - dt / 1.6).max(0.0);
        if m.copied == 0.0 {
            dirty.push("btn_copy");
        }
    }

    if approach(
        &mut m.nav_home,
        if pointer_over::<Button>("nav_home") { 1.0 } else { 0.0 },
        hover_k,
    ) {
        dirty.push("nav_home");
    }
    if approach(
        &mut m.nav_settings,
        if pointer_over::<Button>("nav_settings") { 1.0 } else { 0.0 },
        hover_k,
    ) {
        dirty.push("nav_settings");
    }
    if approach(
        &mut m.primary,
        if pointer_over::<Button>("btn_toggle") { 1.0 } else { 0.0 },
        hover_k,
    ) {
        dirty.push("btn_toggle");
    }
    if approach(
        &mut m.choice,
        if pointer_over::<Choice>("choice_theme") { 1.0 } else { 0.0 },
        hover_k,
    ) {
        dirty.push("choice_theme");
    }

    let switches = [
        (Switch::Auto, "check_auto"),
        (Switch::Native, "check_native"),
        (Switch::Nvenc, "check_nvenc"),
        (Switch::Wayland, "check_wayland"),
    ];
    for (which, id) in switches {
        let target = if checked(id) { 1.0 } else { 0.0 };
        let spring = match which {
            Switch::Auto => &mut m.sw_auto,
            Switch::Native => &mut m.sw_native,
            Switch::Nvenc => &mut m.sw_nvenc,
            Switch::Wayland => &mut m.sw_wayland,
        };
        if spring_step(spring, target, dt) {
            dirty.push(match which {
                Switch::Auto => "check_auto",
                Switch::Native => "check_native",
                Switch::Nvenc => "check_nvenc",
                Switch::Wayland => "check_wayland",
            });
        }
    }

    let live = SERVER_LIVE.load(Ordering::Relaxed);
    if spring_step(&mut m.status, if live { 1.0 } else { 0.0 }, dt) {
        dirty.push("status_pill");
    }

    drop(m);
    for id in dirty {
        redraw_id(id);
    }
    app::repeat_timeout2(1.0 / 60.0, ui_motion_tick);
}

fn switch_spring(which: Switch) -> f32 {
    let m = MOTION.lock().unwrap();
    match which {
        Switch::Auto => m.sw_auto.x,
        Switch::Native => m.sw_native.x,
        Switch::Nvenc => m.sw_nvenc.x,
        Switch::Wayland => m.sw_wayland.x,
    }
}

fn hover_value(home: bool) -> f32 {
    let m = MOTION.lock().unwrap();
    if home {
        m.nav_home
    } else {
        m.nav_settings
    }
}

/// Kick the status crossfade when the server changes state.
fn kick_status(live: bool) {
    MOTION.lock().unwrap().status.v = if live { 9.0 } else { -9.0 };
}

/// Server state card at the foot of the sidebar: a power glyph in a tinted
/// tile, the state in bold and a one-line hint underneath. Nothing loops; the
/// tile and glyph only cross-fade to green when the server starts or stops.
fn draw_status_pill(f: &mut Frame) {
    let skin = skin();
    let (x, y, w, h) = (f.x(), f.y(), f.w(), f.h());
    let live = SERVER_LIVE.load(Ordering::Relaxed);
    let on = MOTION.lock().unwrap().status.x.clamp(0.0, 1.0);
    let on_k = (1000.0 * on) as u32;

    // Card: the sidebar glass with a green edge while the server runs.
    let card = skin.nav_bg.mix(skin.pill, 600);
    draw::set_draw_color(Color::from(card));
    draw::draw_rounded_rectf(x, y, w, h, RADIUS_XL);
    draw_top_highlight(x, y, w, card, RADIUS_XL);
    draw::set_draw_color(Color::from(skin.border.mix(skin.success, (160.0 * on) as u32)));
    draw::draw_rounded_rect(x, y, w - 1, h - 1, RADIUS_XL);

    // Icon tile: 32px square, grey when stopped and green when live.
    const TILE: i32 = 32;
    let tx = x + 12;
    let ty = y + (h - TILE) / 2;
    let tile_off = skin.surface.mix(skin.dim, 120);
    let tile_on = skin.surface.mix(skin.success, 220);
    draw::set_draw_color(Color::from(tile_off.mix(tile_on, on_k)));
    draw::draw_rounded_rectf(tx, ty, TILE, TILE, RADIUS_LG);

    // Power glyph: a 300 degree arc open at the top, with a stem through the gap.
    let (gx, gy) = (tx + TILE / 2, ty + TILE / 2);
    draw::set_draw_color(Color::from(skin.dim.mix(skin.success, on_k)));
    draw::set_line_style(draw::LineStyle::Solid, 2);
    draw::draw_arc(gx - 7, gy - 7, 14, 14, 120.0, 420.0);
    draw::draw_line(gx, gy - 9, gx, gy - 1);
    draw::set_line_style(draw::LineStyle::Solid, 0);

    // Title and hint, left-aligned after the tile.
    let text_x = tx + TILE + 10;
    let text_w = x + w - text_x - 8;
    draw::set_draw_color(Color::from(skin.text));
    draw::set_font(Font::HelveticaBold, 12);
    draw::draw_text2(&f.label(), text_x, y + 11, text_w, 18, Align::Left | Align::Inside);
    draw::set_draw_color(Color::from(skin.dim));
    draw::set_font(Font::Helvetica, 11);
    draw::draw_text2(
        if live {
            "Ждёт подключения"
        } else {
            "Нажми «Запустить»"
        },
        text_x,
        y + 30,
        text_w,
        16,
        Align::Left | Align::Inside,
    );
}
static WINCTX: Mutex<Option<InputAreaWindowContext>> = Mutex::new(None);

struct InputAreaWindowContext {
    win: Window,
    choice_mouse: Choice,
    choice_touch: Choice,
    choice_pen: Choice,
    workspaces: Vec<Rect>,
}

pub fn get_input_area(
    no_gui: bool,
    output_sender: std::sync::mpsc::Sender<crate::protocol::CustomInputAreas>,
) {
    // If no gui is running there is no event loop and windows can not be created.
    // That's why we initialize the fltk app here one the first call.
    if no_gui {
        static GUI_INITIALIZED: AtomicBool = AtomicBool::new(false);

        if !GUI_INITIALIZED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            std::thread::spawn(move || {
                let _app = App::default().with_scheme(fltk::app::AppScheme::Gtk);
                let mut winctx = create_custom_input_area_window();
                custom_input_area_window_handle_events(&mut winctx.win, output_sender.clone());
                show_overlay_window(&mut winctx);
                WINCTX.lock().unwrap().replace(winctx);
                loop {
                    // calling wait_for ensures that the fltk event loop keeps running even if
                    // there is no window shown
                    if let Err(err) = app::wait_for(1.0) {
                        warn!("Error waiting for fltk events: {err}.");
                    }
                }
            });
        } else {
            fltk::app::awake_callback(move || {
                let mut winctx = WINCTX.lock().unwrap();
                let winctx = winctx.as_mut().unwrap();
                custom_input_area_window_handle_events(&mut winctx.win, output_sender.clone());
                show_overlay_window(winctx);
            });
        }
    } else {
        fltk::app::awake_callback(move || {
            let mut winctx = WINCTX.lock().unwrap();
            if winctx.is_none() {
                winctx.replace(create_custom_input_area_window());
            }

            let winctx = winctx.as_mut().unwrap();
            custom_input_area_window_handle_events(&mut winctx.win, output_sender.clone());
            show_overlay_window(winctx);
        });
    }
}

fn create_custom_input_area_window() -> InputAreaWindowContext {
    let mut win = Window::default().with_size(600, 600).center_screen();
    win.make_resizable(true);
    win.set_border(false);
    win.set_frame(FrameType::FlatBox);
    win.set_color(fltk::enums::Color::from_rgb(240, 240, 240));
    let mut frame = Frame::default()
        .with_size(win.w() - 2 * BORDER, win.h() - 2 * BORDER)
        .center_of_parent()
        .with_label(
            "Press Enter to submit\ncurrent selection as\ncustom input area,\nEscape to abort.",
        );
    frame.set_label_type(LabelType::Normal);
    frame.set_label_size(20);
    frame.set_color(fltk::enums::Color::Black);
    frame.set_frame(FrameType::BorderFrame);
    frame.set_label_font(fltk::enums::Font::HelveticaBold);
    let width = 200;
    let height = 30;
    let padding = 10;
    let tool_tip = "Some systems may have the input device mapped to a specific screen, this screen has to be selected here. Otherwise input mapping will be wrong. Selecting None disables any mapping.";
    let mut choice_mouse = Choice::default()
        .with_size(width, height)
        .with_pos(padding, 4 * padding)
        .center_x(&frame)
        .with_id("choice_mouse")
        .with_label("Map Mouse from:");
    choice_mouse.set_tooltip(tool_tip);
    let mut choice_touch = Choice::default()
        .with_size(width, height)
        .below_of(&choice_mouse, padding)
        .with_id("choice_touch")
        .with_label("Map Touch from:");
    choice_touch.set_tooltip(tool_tip);
    let mut choice_pen = Choice::default()
        .with_size(width, height)
        .below_of(&choice_touch, padding)
        .with_id("choice_pen")
        .with_label("Map Pen from:");
    choice_pen.set_tooltip(tool_tip);

    frame.handle(|frame, event| match event {
        fltk::enums::Event::Push => {
            if app::event_clicks() {
                if let Some(mut win) = frame.window() {
                    win.fullscreen(!win.fullscreen_active());
                }
                true
            } else {
                false
            }
        }
        _ => false,
    });
    win.resize_callback(move |_win, _x, _y, w, h| {
        frame.resize(BORDER, BORDER, w - 2 * BORDER, h - 2 * BORDER)
    });
    win.end();
    InputAreaWindowContext {
        win,
        choice_mouse,
        choice_touch,
        choice_pen,
        workspaces: Vec::new(),
    }
}

fn custom_input_area_window_handle_events(
    win: &mut Window,
    sender: std::sync::mpsc::Sender<crate::protocol::CustomInputAreas>,
) {
    #[derive(Debug)]
    enum MouseFlags {
        All,
        Edge(bool, bool),
        Corner(bool, bool),
    }
    fn get_mouse_flags(win: &Window, x: i32, y: i32) -> MouseFlags {
        let dx0 = (win.x() - x).abs();
        let dy0 = (win.y() - y).abs();
        let dx1 = (win.x() + win.w() - x).abs();
        let dy1 = (win.y() + win.h() - y).abs();
        let dx = min(dx0, dx1);
        let dy = min(dy0, dy1);
        let d = min(dx, dy);
        if d <= BORDER {
            if dx <= BORDER && dy <= BORDER {
                MouseFlags::Corner(dx0 <= dx1, dy0 <= dy1)
            } else {
                MouseFlags::Edge(dx <= dy, if dx <= dy { dx0 <= dx1 } else { dy0 <= dy1 })
            }
        } else {
            MouseFlags::All
        }
    }
    fn get_screen_coords_from_event_coords(win: &Window, (x, y): (i32, i32)) -> (i32, i32) {
        (x + win.x(), y + win.y())
    }
    fn set_cursor(
        win: &mut Window,
        current_cursor: &mut fltk::enums::Cursor,
        flags: Option<MouseFlags>,
    ) {
        let cursor = match flags {
            Some(MouseFlags::All) => fltk::enums::Cursor::Move,
            Some(MouseFlags::Edge(bx, by)) => match (bx, by) {
                (true, true) => fltk::enums::Cursor::W,
                (true, false) => fltk::enums::Cursor::E,
                (false, true) => fltk::enums::Cursor::N,
                (false, false) => fltk::enums::Cursor::S,
            },
            Some(MouseFlags::Corner(bx, by)) => match (bx, by) {
                (true, true) => fltk::enums::Cursor::NWSE,
                (true, false) => fltk::enums::Cursor::NESW,
                (false, true) => fltk::enums::Cursor::NESW,
                (false, false) => fltk::enums::Cursor::NWSE,
            },
            None => fltk::enums::Cursor::Default,
        };
        if *current_cursor != cursor {
            *current_cursor = cursor;
            win.set_cursor(cursor);
        }
    }

    let mut drag_flags = MouseFlags::All;
    let mut current_cursor = fltk::enums::Cursor::Default;
    let mut x = 0;
    let mut y = 0;
    let mut win_x_drag_start = 0;
    let mut win_y_drag_start = 0;
    let mut win_w_drag_start = 0;
    let mut win_h_drag_start = 0;
    win.handle(move |win, event| {
        match event {
            fltk::enums::Event::Move => {
                let (x, y) = get_screen_coords_from_event_coords(&win, app::event_coords());
                let flags = get_mouse_flags(&win, x, y);
                set_cursor(win, &mut current_cursor, Some(flags));
                true
            }
            fltk::enums::Event::Leave => {
                win.set_cursor(fltk::enums::Cursor::Default);
                true
            }
            fltk::enums::Event::Push => {
                (x, y) = get_screen_coords_from_event_coords(&win, app::event_coords());
                win_x_drag_start = win.x();
                win_y_drag_start = win.y();
                win_w_drag_start = win.w();
                win_h_drag_start = win.h();
                drag_flags = get_mouse_flags(&win, x, y);
                true
            }
            fltk::enums::Event::Drag => {
                if win.opacity() == 1.0 {
                    win.set_opacity(0.5);
                }
                let (x_new, y_new) = get_screen_coords_from_event_coords(&win, app::event_coords());
                let dx = x_new - x;
                let dy = y_new - y;
                match drag_flags {
                    MouseFlags::All => win.set_pos(win_x_drag_start + dx, win_y_drag_start + dy),
                    MouseFlags::Edge(bx, by) => match (bx, by) {
                        (true, true) => win.resize(
                            win_x_drag_start + dx,
                            win_y_drag_start,
                            win_w_drag_start - dx,
                            win_h_drag_start,
                        ),
                        (true, false) => win.resize(
                            win_x_drag_start,
                            win_y_drag_start,
                            win_w_drag_start + dx,
                            win_h_drag_start,
                        ),
                        (false, true) => win.resize(
                            win_x_drag_start,
                            win_y_drag_start + dy,
                            win_w_drag_start,
                            win_h_drag_start - dy,
                        ),
                        (false, false) => win.resize(
                            win_x_drag_start,
                            win_y_drag_start,
                            win_w_drag_start,
                            win_h_drag_start + dy,
                        ),
                    },
                    MouseFlags::Corner(bx, by) => match (bx, by) {
                        (true, true) => win.resize(
                            win_x_drag_start + dx,
                            win_y_drag_start + dy,
                            win_w_drag_start - dx,
                            win_h_drag_start - dy,
                        ),

                        (true, false) => win.resize(
                            win_x_drag_start + dx,
                            win_y_drag_start,
                            win_w_drag_start - dx,
                            win_h_drag_start + dy,
                        ),
                        (false, true) => win.resize(
                            win_x_drag_start,
                            win_y_drag_start + dy,
                            win_w_drag_start + dx,
                            win_h_drag_start - dy,
                        ),
                        (false, false) => win.resize(
                            win_x_drag_start,
                            win_y_drag_start,
                            win_w_drag_start + dx,
                            win_h_drag_start + dy,
                        ),
                    },
                }
                true
            }
            fltk::enums::Event::Released => {
                if win.opacity() != 1.0 {
                    win.set_opacity(1.0);
                }
                true
            }
            fltk::enums::Event::KeyDown => match app::event_key() {
                fltk::enums::Key::Enter => {
                    fn relative_rect(win: &Window, workspace: &Rect) -> Rect {
                        // clamp rect to workspace and ensure it has non-zero area
                        let mut rect = crate::protocol::Rect {
                            x: (win.x() as f64 - workspace.x).min(workspace.w) / workspace.w,
                            y: (win.y() as f64 - workspace.y).min(workspace.h) / workspace.h,
                            w: win.w().max(1) as f64 / workspace.w,
                            h: win.h().max(1) as f64 / workspace.h,
                        };
                        rect.w = rect.w.min(1.0 - rect.x);
                        rect.h = rect.h.min(1.0 - rect.y);
                        rect
                    }
                    win.set_cursor(fltk::enums::Cursor::Default);
                    win.hide();
                    let mut areas = CustomInputAreas::default();
                    let workspaces = WINCTX.lock().unwrap().as_ref().unwrap().workspaces.clone();
                    for (name, area) in [
                        ("choice_mouse", &mut areas.mouse),
                        ("choice_touch", &mut areas.touch),
                        ("choice_pen", &mut areas.pen),
                    ] {
                        let c: Choice = fltk::app::widget_from_id(name).unwrap();
                        match c.value() {
                            0 => (),
                            v @ 1.. if (v as usize) <= workspaces.len() => {
                                let workspace = workspaces[v as usize - 1];
                                *area = Some(relative_rect(win, &workspace))
                            }
                            v => warn!("Unexpected value in {name}: {v}!"),
                        }
                    }
                    // The client may have disconnected while the dialog was
                    // open; ignore a dead receiver instead of panicking
                    // inside the fltk event loop.
                    if let Err(err) = sender.send(areas) {
                        warn!("Failed to deliver custom input areas: {err}.");
                    }
                    true
                }
                fltk::enums::Key::Escape => {
                    win.set_cursor(fltk::enums::Cursor::Default);
                    win.hide();
                    true
                }
                _ => false,
            },
            _ => false,
        }
    });
}

fn show_overlay_window(winctx: &mut InputAreaWindowContext) {
    let win = &mut winctx.win;
    if win.shown() {
        return;
    }
    let screens = fltk::app::Screen::all_screens();
    winctx.workspaces.clear();
    winctx.workspaces.push(get_full_workspace_rect());
    for screen in &screens {
        let fltk::draw::Rect { x, y, w, h } = screen.work_area();
        winctx.workspaces.push(Rect {
            x: x as f64,
            y: y as f64,
            w: w as f64,
            h: h as f64,
        });
    }
    for c in [
        &mut winctx.choice_mouse,
        &mut winctx.choice_touch,
        &mut winctx.choice_pen,
    ] {
        let v = c.value();
        c.clear();
        c.add_choice("None");
        c.add_choice("Full Workspace");
        for screen in &screens {
            c.add_choice(&format!(
                "Screen {n} at {w}x{h}+{x}+{y}",
                n = screen.n,
                w = screen.w(),
                h = screen.h(),
                x = screen.x(),
                y = screen.y()
            ));
        }
        if v >= 0 && (v as usize) < 2 + screens.len() {
            c.set_value(v);
        } else {
            c.set_value(0);
        }
    }
    if win.fullscreen_active() {
        win.set_size(600, 600);
        let n = win.screen_num();
        let screen = app::Screen::new(n).unwrap();
        win.set_pos(
            screen.x() + (screen.w() - 600) / 2,
            screen.y() + (screen.h() - 600) / 2,
        );
    }
    win.show();
    win.set_on_top();
    win.set_visible_focus();
}

pub fn get_full_workspace_rect() -> Rect {
    let mut rect = Rect::default();
    for screen in fltk::app::Screen::all_screens() {
        let fltk::draw::Rect { x, y, w, h } = screen.work_area();
        rect.x = (x as f64).min(rect.x);
        rect.y = (y as f64).min(rect.y);
        rect.w = ((x + w) as f64).max(rect.w);
        rect.h = ((y + h) as f64).max(rect.h);
    }
    rect
}
