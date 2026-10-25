use std::cmp::min;
use std::io::Cursor;
use std::iter::Iterator;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};

use fltk::app;
use fltk::draw;
use fltk::enums::{Align, Color, Font, FrameType, LabelType, UnmappedFrameType};
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

pub fn run(config: &Config, log_receiver: mpsc::Receiver<String>) {
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
    const NAV_W: i32 = 208;
    const WIN_W: i32 = 720;
    const WIN_H: i32 = 720;
    const CONTENT_W: i32 = WIN_W - NAV_W;
    const PAD: i32 = 28;
    const PAGE_W: i32 = CONTENT_W - 2 * PAD;

    let app = App::default().with_scheme(fltk::app::AppScheme::Gtk);
    let mut wind = Window::default()
        .with_size(WIN_W, WIN_H)
        .center_screen()
        .with_label(&format!("Proximity - {}", env!("CARGO_PKG_VERSION")));
    wind.set_xclass("proximity");
    wind.set_callback(move |_win| app.quit());
    wind.set_frame(FrameType::FlatBox);
    wind.set_color(Color::from_rgb(14, 17, 22));
    wind.set_id("main_window");
    // The layout is laid out in absolute pixels, so the shell keeps a fixed
    // size instead of stretching the cards.
    wind.size_range(WIN_W, WIN_H, WIN_W, WIN_H);

    // --- Sidebar with the "Главная" / "Настройки" navigation --------------
    let mut nav_bar = Group::default()
        .with_pos(0, 0)
        .with_size(NAV_W, WIN_H)
        .with_id("nav_bar");

    Frame::default()
        .with_size(36, 36)
        .with_pos(20, 20)
        .with_align(Align::Center | Align::Inside.bits())
        .with_label("P")
        .with_id("brand_mark");
    Frame::default()
        .with_size(130, 24)
        .with_pos(66, 16)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label("Proximity")
        .with_id("head_title");
    Frame::default()
        .with_size(130, 14)
        .with_pos(67, 41)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label(&format!("v{}", env!("CARGO_PKG_VERSION")))
        .with_id("head_sub");

    Frame::default()
        .with_size(140, 16)
        .with_pos(20, 94)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label("МЕНЮ")
        .with_id("nav_caption");

    let mut nav_home_btn = Button::default()
        .with_size(184, 42)
        .with_pos(12, 116)
        .with_label("Главная")
        .with_id("nav_home");
    nav_home_btn.draw(|b| draw_nav_item(b, true));
    nav_home_btn.super_draw(false);
    let mut nav_settings_btn = Button::default()
        .with_size(184, 42)
        .with_pos(12, 164)
        .with_label("Настройки")
        .with_id("nav_settings");
    nav_settings_btn.draw(|b| draw_nav_item(b, false));
    nav_settings_btn.super_draw(false);

    let mut status_pill = Frame::default()
        .with_size(184, 34)
        .with_pos(12, WIN_H - 62)
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

    // 1px divider between the navigation sidebar and the pages.
    Frame::default()
        .with_size(1, WIN_H)
        .with_pos(NAV_W - 1, 0)
        .with_id("head_divider");

    // --- Home page ---------------------------------------------------------
    let mut page_home = Group::default()
        .with_pos(NAV_W, 0)
        .with_size(CONTENT_W, WIN_H)
        .with_id("page_home");

    // Ambient glow backdrop, painted by the current palette (first child, so
    // every other widget sits on top of it).
    let mut home_glow = Frame::default()
        .with_size(CONTENT_W, WIN_H)
        .with_pos(0, 0)
        .with_id("page_glow_home");
    home_glow.draw(draw_page_glow);
    home_glow.super_draw(false);

    Frame::default()
        .with_size(PAGE_W, 62)
        .with_pos(PAD, 24)
        .with_align(Align::Left | Align::Top | Align::Inside.bits())
        .with_label("Управляй компьютером\nиз любого браузера")
        .with_id("hero_title");

    Frame::default()
        .with_size(PAGE_W, 18)
        .with_pos(PAD, 92)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label("Запусти сервер и открой адрес на телефоне")
        .with_id("hero_sub");

    // Server card: state, primary action, connect address and the QR code.
    Frame::default()
        .with_size(PAGE_W, 220)
        .with_pos(PAD, 124)
        .with_id("server_card");

    let mut but_toggle = Button::default()
        .with_size(200, 46)
        .with_pos(PAD + 20, 146)
        .with_label("Запустить")
        .with_id("btn_toggle");
    but_toggle.draw(draw_primary_button);
    but_toggle.super_draw(false);

    Frame::default()
        .with_size(216, 18)
        .with_pos(PAD + 20, 212)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label("Сервер не запущен")
        .with_id("offline_hint");

    let mut label_addr = Frame::default()
        .with_size(216, 16)
        .with_pos(PAD + 20, 210)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label("АДРЕС ДЛЯ ПОДКЛЮЧЕНИЯ")
        .with_id("label_addr");
    label_addr.hide();

    let mut output_server_addr = Output::default()
        .with_size(
            if cfg!(target_os = "linux") {
                216
            } else {
                PAGE_W - 40
            },
            40,
        )
        .with_pos(PAD + 20, 228)
        .with_label("")
        .with_id("output_addr");
    output_server_addr.hide();

    let mut qr_frame = Frame::default()
        .with_size(180, 180)
        .with_pos(PAD + 256, 142)
        .with_id("qr_frame");
    qr_frame.hide();

    let mut qr_hint = Frame::default()
        .with_size(180, 180)
        .with_pos(PAD + 256, 142)
        .with_align(Align::Center | Align::Inside.bits())
        .with_label("QR-код появится\nпосле запуска")
        .with_id("qr_hint");
    // The Windows and macOS builds never show a QR code.
    if !cfg!(target_os = "linux") {
        qr_hint.hide();
    }

    Frame::default()
        .with_size(PAGE_W, 14)
        .with_pos(PAD, 368)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label("ЖУРНАЛ")
        .with_id("log_caption");

    Frame::default()
        .with_size(PAGE_W, 14)
        .with_pos(PAD, 368)
        .with_align(Align::Right | Align::Inside.bits())
        .with_label("СООБЩЕНИЯ СЕРВЕРА")
        .with_id("log_hint");

    let output_buf = TextBuffer::default();
    let mut output = TextDisplay::default()
        .with_size(PAGE_W, 304)
        .with_pos(PAD, 388)
        .with_id("log_display");
    output.set_buffer(output_buf);
    let output_buf = output.buffer().unwrap();

    page_home.end();
    // fltk stores children of a group with window-absolute coordinates; the
    // page widgets above were authored relative to their page, so put them
    // where the page actually sits.
    shift_children(&mut page_home, NAV_W);

    // --- Settings page -----------------------------------------------------
    let mut page_settings = Group::default()
        .with_pos(NAV_W, 0)
        .with_size(CONTENT_W, WIN_H)
        .with_id("page_settings");

    let mut settings_glow = Frame::default()
        .with_size(CONTENT_W, WIN_H)
        .with_pos(0, 0)
        .with_id("page_glow_settings");
    settings_glow.draw(draw_page_glow);
    settings_glow.super_draw(false);

    Frame::default()
        .with_size(PAGE_W, 14)
        .with_pos(PAD, 26)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label("ОФОРМЛЕНИЕ")
        .with_id("sec_skin");

    Frame::default()
        .with_size(PAGE_W, 92)
        .with_pos(PAD, 46)
        .with_id("card_skin");

    Frame::default()
        .with_size(240, 14)
        .with_pos(PAD + 18, 62)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label("ТЕМА")
        .with_id("theme_label");

    let mut choice_theme = Choice::default()
        .with_size(PAGE_W - 36, 40)
        .with_pos(PAD + 18, 80)
        .with_label("")
        .with_id("choice_theme");
    choice_theme.draw(draw_choice);
    choice_theme.super_draw(false);

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

    Frame::default()
        .with_size(PAGE_W, 14)
        .with_pos(PAD, 160)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label("СЕТЬ И ДОСТУП")
        .with_id("sec_net");

    Frame::default()
        .with_size(PAGE_W, 292)
        .with_pos(PAD, 180)
        .with_id("card_net");

    for (id, label, y) in [
        ("label_access", "КОД ДОСТУПА", 196),
        ("label_bind", "АДРЕС ПРИВЯЗКИ", 270),
        ("label_port", "ПОРТ", 344),
    ] {
        Frame::default()
            .with_size(300, 14)
            .with_pos(PAD + 18, y)
            .with_align(Align::Left | Align::Inside.bits())
            .with_label(label)
            .with_id(id);
    }

    let mut input_access_code = Input::default()
        .with_pos(PAD + 18, 214)
        .with_size(PAGE_W - 36, 40)
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
        .with_size(PAGE_W - 36, 40)
        .with_pos(PAD + 18, 288)
        .with_id("input_bind");
    input_bind_addr.set_value(&config.bind_address.to_string());

    let mut input_port = IntInput::default()
        .with_size(PAGE_W - 36, 40)
        .with_pos(PAD + 18, 362)
        .with_id("input_port");
    input_port.set_value(&config.web_port.to_string());

    let mut check_auto_start = CheckButton::default()
        .with_size(PAGE_W - 36, 36)
        .with_pos(PAD + 18, 420)
        .with_label("Автозапуск")
        .with_id("check_auto");
    check_auto_start.draw(draw_switch);
    check_auto_start.super_draw(false);
    check_auto_start.set_tooltip("Start Proximity server immediately on program start.");
    check_auto_start.set_checked(config.auto_start);

    #[cfg(target_os = "linux")]
    let mut check_wayland = CheckButton::default()
        .with_size(PAGE_W - 36, 40)
        .with_pos(PAD + 18, 584)
        .with_label("Wayland / PipeWire")
        .with_id("check_wayland");
    #[cfg(target_os = "linux")]
    {
        check_wayland.draw(draw_switch);
        check_wayland.super_draw(false);
        check_wayland.set_tooltip(
            "EXPERIMENTAL! This may crash your desktop! Enables screen \
        capturing for Wayland using PipeWire and GStreamer.",
        );
        check_wayland.set_checked(config.wayland_support);
    }

    Frame::default()
        .with_size(PAGE_W, 14)
        .with_pos(PAD, 496)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label("УСКОРЕНИЕ")
        .with_id("sec_hw");

    let hw_card_h = if cfg!(target_os = "linux") { 144 } else { 96 };
    Frame::default()
        .with_size(PAGE_W, hw_card_h)
        .with_pos(PAD, 516)
        .with_id("card_hw");

    let mut check_native_hw_accel = CheckButton::default()
        .with_size(200, 40)
        .with_pos(PAD + 18, 536)
        .with_id("check_native");
    check_native_hw_accel.draw(draw_switch);
    check_native_hw_accel.super_draw(false);

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
        .with_size(200, 40)
        .with_pos(PAD + 226, 536)
        .with_label("NVENC")
        .with_id("check_nvenc");
    check_nvenc.draw(draw_switch);
    check_nvenc.super_draw(false);
    check_nvenc.set_tooltip("Try to use Nvidia's NVENC to encode the video via GPU.");

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    check_nvenc.set_checked(config.try_nvenc);

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        check_nvenc.deactivate();
        check_nvenc.hide();
    }

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
                        pop_up.set_color(Color::from_rgb(14, 17, 22));

                        let buf = TextBuffer::default();
                        let mut pop_up_text = TextDisplay::default().with_size(w, h);
                        pop_up_text.set_buffer(buf);
                        pop_up_text.wrap_mode(fltk::text::WrapMode::AtBounds, 5);
                        pop_up_text.set_frame(FrameType::FlatBox);
                        pop_up_text.set_color(Color::from_rgb(14, 17, 22));
                        pop_up_text.set_text_color(Color::from_rgb(232, 236, 241));
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
                if let Some(mut pill) = app::widget_from_id::<Frame>("status_pill") {
                    pill.set_label("Сервер работает");
                    pill.redraw();
                }

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

                #[cfg(not(target_os = "windows"))]
                {
                    use image::Luma;
                    use qrcode::QrCode;
                    let addr_string = format!("http://{}", web_sock);
                    output_server_addr.set_value(&addr_string);
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

                    let cb = move |qr_frame: &mut Frame, _, _, w, h| {
                        let code = QrCode::new(&url_string).unwrap();
                        let img_buf = code.render::<Luma<u8>>().build();
                        let image = image::DynamicImage::ImageLuma8(img_buf);
                        let dims = min(w, h) as u32;
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
                }
                #[cfg(target_os = "windows")]
                {
                    if web_sock.ip().is_unspecified() {
                        output_server_addr.set_value("http://<your ip address>");
                    } else {
                        output_server_addr.set_value(&format!("http://{}", web_sock.to_string()));
                    }
                }
                output_server_addr.show();
                but.set_label("Остановить");
                if let Some(mut f) = app::widget_from_id::<Frame>("offline_hint") {
                    f.hide();
                }
                if let Some(mut f) = app::widget_from_id::<Frame>("qr_hint") {
                    if cfg!(target_os = "linux") {
                        f.hide();
                    }
                }
                if let Some(mut f) = app::widget_from_id::<Frame>("label_addr") {
                    f.show();
                }
            } else {
                proximity.stop();
                but.set_label("Запустить");
                output_server_addr.hide();
                qr_frame.resize_callback(|_, _, _, _, _| {});
                qr_frame.hide();
                if let Some(mut f) = app::widget_from_id::<Frame>("offline_hint") {
                    f.show();
                }
                if let Some(mut f) = app::widget_from_id::<Frame>("qr_hint") {
                    if cfg!(target_os = "linux") {
                        f.show();
                    }
                }
                if let Some(mut f) = app::widget_from_id::<Frame>("label_addr") {
                    f.hide();
                }
                is_server_running = false;
                SERVER_LIVE.store(false, Ordering::Relaxed);
                if let Some(mut pill) = app::widget_from_id::<Frame>("status_pill") {
                    pill.set_label("Сервер выключен");
                    pill.redraw();
                }
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

/// Modern skin applied on top of every widget, driven by the active palette
/// (see src/theme.rs). Every styled widget is looked up by id, so this can be
/// re-run at any time - in particular right after a theme switch, which
/// would otherwise keep stale colors around.
fn apply_skin() {
    let skin = skin();
    let bg = Color::from(skin.bg_end);
    let nav_bg = Color::from(skin.nav_bg);
    let surface = Color::from(skin.surface);
    let accent = Color::from(skin.accent);
    let accent_pressed = Color::from(skin.accent_pressed);
    let text = Color::from(skin.text);
    let dim = Color::from(skin.dim);
    let log_text = Color::from(skin.log_text);

    // Window chrome: the whole window takes the sidebar tone so no classic
    // grey bleeds through, then the pieces paint their own colors.
    if let Some(mut w) = app::widget_from_id::<Window>("main_window") {
        w.set_frame(FrameType::FlatBox);
        w.set_color(nav_bg);
    }
    if let Some(mut g) = app::widget_from_id::<Group>("nav_bar") {
        g.set_frame(FrameType::FlatBox);
        g.set_color(nav_bg);
    }
    for id in ["head_title", "head_sub"] {
        if let Some(mut f) = app::widget_from_id::<Frame>(id) {
            f.set_frame(FrameType::NoBox);
            f.set_label_color(text);
        }
    }
    if let Some(mut f) = app::widget_from_id::<Frame>("head_title") {
        f.set_label_font(Font::HelveticaBold);
        f.set_label_size(20);
    }
    if let Some(mut f) = app::widget_from_id::<Frame>("head_sub") {
        f.set_label_size(11);
    }
    // Cards and their captions on the settings page.
    for id in ["card_skin", "card_net", "card_hw"] {
        if let Some(mut f) = app::widget_from_id::<Frame>(id) {
            f.set_frame(card_frame());
            f.set_color(surface);
        }
    }
    for id in ["theme_label", "label_access", "label_bind", "label_port"] {
        if let Some(mut f) = app::widget_from_id::<Frame>(id) {
            f.set_frame(FrameType::NoBox);
            f.set_label_color(dim);
            f.set_label_font(Font::HelveticaBold);
            f.set_label_size(10);
        }
    }
    if let Some(mut f) = app::widget_from_id::<Frame>("brand_mark") {
        f.set_label_font(Font::HelveticaBold);
        f.set_label_size(18);
    }
    if let Some(mut f) = app::widget_from_id::<Frame>("status_pill") {
        f.set_label_size(12);
    }
    if let Some(mut f) = app::widget_from_id::<Frame>("head_divider") {
        f.set_frame(FrameType::FlatBox);
        f.set_color(Color::from(skin.divider));
    }
    if let Some(mut b) = app::widget_from_id::<Button>("btn_toggle") {
        b.set_label_color(Color::White);
        b.set_label_size(15);
        b.set_label_font(Font::HelveticaBold);
        b.set_selection_color(accent_pressed);
    }
    if let Some(mut w) = app::widget_from_id::<Output>("output_addr") {
        w.set_frame(field_frame());
        w.set_color(surface);
        w.set_label_color(dim);
        w.set_text_color(text);
        w.set_text_font(Font::CourierBold);
        w.set_text_size(13);
    }
    if let Some(mut f) = app::widget_from_id::<Frame>("server_card") {
        f.set_frame(card_frame());
        f.set_color(surface);
    }
    for id in ["input_access", "input_bind"] {
        if let Some(mut w) = app::widget_from_id::<Input>(id) {
            w.set_frame(field_frame());
            w.set_color(surface);
            w.set_text_color(text);
            w.set_text_font(Font::Helvetica);
            w.set_text_size(13);
            w.set_cursor_color(accent);
        }
    }
    if let Some(mut w) = app::widget_from_id::<IntInput>("input_port") {
        w.set_frame(field_frame());
        w.set_color(surface);
        w.set_text_color(text);
        w.set_text_font(Font::Helvetica);
        w.set_text_size(13);
        w.set_cursor_color(accent);
    }
    if let Some(mut c) = app::widget_from_id::<Choice>("choice_theme") {
        c.set_color(surface);
        c.set_text_color(text);
        c.set_text_size(13);
        c.redraw();
    }
    for id in ["check_auto", "check_native", "check_nvenc", "check_wayland"] {
        if let Some(mut c) = app::widget_from_id::<CheckButton>(id) {
            c.set_label_color(text);
            c.set_label_size(13);
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
    }
    if let Some(mut f) = app::widget_from_id::<Frame>("qr_frame") {
        f.set_frame(card_frame());
        f.set_color(Color::White);
    }
    for id in ["hero_title", "hero_sub", "qr_hint", "offline_hint", "label_addr", "log_caption"] {
        if let Some(mut f) = app::widget_from_id::<Frame>(id) {
            f.set_frame(FrameType::NoBox);
        }
    }
    if let Some(mut f) = app::widget_from_id::<Frame>("hero_title") {
        f.set_label_color(text);
        f.set_label_font(Font::HelveticaBold);
        f.set_label_size(22);
    }
    if let Some(mut f) = app::widget_from_id::<Frame>("hero_sub") {
        f.set_label_color(dim);
        f.set_label_size(12);
    }
    for id in ["qr_hint", "offline_hint"] {
        if let Some(mut f) = app::widget_from_id::<Frame>(id) {
            f.set_label_color(dim);
            f.set_label_size(12);
        }
    }
    for id in ["label_addr", "log_caption"] {
        if let Some(mut f) = app::widget_from_id::<Frame>(id) {
            f.set_label_color(dim);
            f.set_label_font(Font::HelveticaBold);
            f.set_label_size(10);
        }
    }
    if let Some(mut f) = app::widget_from_id::<Frame>("log_hint") {
        f.set_frame(FrameType::NoBox);
        f.set_label_color(dim);
        f.set_label_size(10);
    }

    // Decorative captions sit directly on the glow backdrop, so they must not
    // paint their own rectangle; pages hold the backdrop as first child.
    for id in [
        "sec_skin",
        "sec_net",
        "sec_hw",
        "head_title",
        "head_sub",
        "nav_caption",
    ] {
        if let Some(mut f) = app::widget_from_id::<Frame>(id) {
            f.set_frame(FrameType::NoBox);
            f.set_color(bg);
        }
    }
    if let Some(mut f) = app::widget_from_id::<Frame>("nav_caption") {
        f.set_label_color(dim);
        f.set_label_size(10);
        f.set_label_font(Font::HelveticaBold);
        f.redraw();
    }
    for id in ["sec_skin", "sec_net", "sec_hw"] {
        if let Some(mut f) = app::widget_from_id::<Frame>(id) {
            f.set_label_color(dim);
            f.set_label_size(10);
            f.set_label_font(Font::HelveticaBold);
        }
    }
    // The page groups must not paint their own box over the glow backdrop,
    // and the backdrop frames themselves must stay transparent.
    for id in ["page_home", "page_settings"] {
        if let Some(mut g) = app::widget_from_id::<Group>(id) {
            g.set_frame(FrameType::NoBox);
        }
    }
    for id in ["page_glow_home", "page_glow_settings"] {
        if let Some(mut f) = app::widget_from_id::<Frame>(id) {
            f.set_frame(FrameType::NoBox);
        }
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

fn draw_field_box(x: i32, y: i32, w: i32, h: i32, color: Color) {
    let skin = skin();
    draw::set_draw_color(color);
    draw::draw_rounded_rectf(x, y, w, h, 10);
    draw::set_draw_color(Color::from(skin.border));
    draw::draw_rounded_rect(x, y, w - 1, h - 1, 10);
}

fn draw_card_box(x: i32, y: i32, w: i32, h: i32, color: Color) {
    let skin = skin();
    draw::set_draw_color(color);
    draw::draw_rounded_rectf(x, y, w, h, 14);
    draw::set_draw_color(Color::from(skin.border));
    draw::draw_rounded_rect(x, y, w - 1, h - 1, 14);
}

fn draw_log_box(x: i32, y: i32, w: i32, h: i32, color: Color) {
    let skin = skin();
    draw::set_draw_color(color);
    draw::draw_rounded_rectf(x, y, w, h, 12);
    draw::set_draw_color(Color::from(skin.border));
    draw::draw_rounded_rect(x, y, w - 1, h - 1, 12);
}

/// Ambient glow backdrop for a page: the palette's tinted top fades into the
/// flat background, echoing the radial "glow from the top" gradients of the
/// original anime presets.
fn draw_page_glow(f: &mut Frame) {
    let skin = skin();
    let (x, y) = (f.x(), f.y());
    let (w, h) = (f.w(), f.h());
    // The haze covers the upper third; below it stays the base color.
    let band = (h / 3).max(1);
    for row in 0..band {
        let t = ((row * 1000) / band) as u32;
        draw::set_draw_color(Color::from(skin.bg_end.mix(skin.bg_start, 1000 - t)));
        draw::draw_rectf(x, y + row, w, 1);
    }
    draw::set_draw_color(Color::from(skin.bg_end));
    draw::draw_rectf(x, y + band, w, h - band);
}

/// Primary action in the server card: a solid accent pill with a hover tint
/// and a pressed shade; all painting is ours, so the GTK box stays away.
fn draw_primary_button(b: &mut Button) {
    let skin = skin();
    let hovered = app::belowmouse::<Button>()
        .map(|w| w.as_widget_ptr() == b.as_widget_ptr())
        .unwrap_or(false);
    let (x, y, w, h) = (b.x(), b.y(), b.w(), b.h());
    let live = SERVER_LIVE.load(Ordering::Relaxed);
    let base_color = if live { skin.danger } else { skin.accent };
    let base = if b.value() {
        base_color.mix(Rgb24::hex(0x000000), 180)
    } else if hovered {
        base_color.mix(Rgb24::hex(0xffffff), 90)
    } else {
        base_color
    };
    draw::set_draw_color(Color::from(base));
    draw::draw_rounded_rectf(x, y, w, h, h / 2);
    draw::set_font(Font::HelveticaBold, 15);
    draw::set_draw_color(Color::White);
    draw::draw_text2(&b.label(), x, y, w, h, Align::Center);
}

/// Theme picker: a field-shaped pill showing the active palette plus a
/// chevron; the popup itself is handled by FLTK.
fn draw_choice(c: &mut Choice) {
    let skin = skin();
    let hovered = app::belowmouse::<Choice>()
        .map(|w| w.as_widget_ptr() == c.as_widget_ptr())
        .unwrap_or(false);
    let (x, y, w, h) = (c.x(), c.y(), c.w(), c.h());
    draw::set_draw_color(Color::from(if hovered {
        skin.pill
    } else {
        skin.surface
    }));
    draw::draw_rounded_rectf(x, y, w, h, 10);
    draw::set_draw_color(Color::from(skin.border));
    draw::draw_rounded_rect(x, y, w - 1, h - 1, 10);
    let label = c.text(c.value().max(0)).unwrap_or_default();
    draw::set_font(Font::Helvetica, 13);
    draw::set_draw_color(Color::from(skin.text));
    draw::draw_text(&label, x + 14, y + h / 2 + 5);
    let cx = x + w - 20;
    let cy = y + h / 2;
    draw::set_draw_color(Color::from(skin.dim));
    draw::draw_line(cx - 4, cy - 2, cx, cy + 2);
    draw::draw_line(cx, cy + 2, cx + 4, cy - 2);
}

/// Toggle switch drawn in place of the stock check box: an accent track with
/// a knob for "on" and a recessed slot for "off".
fn draw_switch(c: &mut CheckButton) {
    let skin = skin();
    let on = c.is_checked();
    let (x, y, h) = (c.x(), c.y(), c.h());
    let track_w = 40;
    let track_h = 22;
    let ty = y + (h - track_h) / 2;
    draw::set_draw_color(Color::from(if on { skin.accent } else { skin.track }));
    draw::draw_rounded_rectf(x, ty, track_w, track_h, track_h / 2);
    let d = track_h - 6;
    let kx = if on { x + track_w - d - 3 } else { x + 3 };
    let knob = if on {
        Color::White
    } else {
        Color::from(skin.dim)
    };
    draw::set_draw_color(knob);
    draw::draw_circle_fill(kx + d / 2, ty + track_h / 2, d, knob);
    if !c.label().is_empty() {
        draw::set_font(Font::Helvetica, 13);
        draw::set_draw_color(Color::from(if c.active() { skin.text } else { skin.dim }));
        draw::draw_text(&c.label(), x + track_w + 12, y + h / 2 + 5);
    }
}

/// Sidebar entry: a pill for the active page plus a small vector icon (house
/// for home, sliders for settings). Hovering lifts the pill so the sidebar
/// feels alive, the active entry keeps a small accent bar.
fn draw_nav_item(b: &mut Button, home: bool) {
    let skin = skin();
    let active = *HOME_ACTIVE.lock().unwrap() == home;
    let hovered = app::belowmouse::<Button>()
        .map(|w| w.as_widget_ptr() == b.as_widget_ptr())
        .unwrap_or(false);
    let (x, y) = (b.x(), b.y());
    let (w, h) = (b.w(), b.h());
    let cy = y + h / 2;
    if active {
        draw::set_draw_color(Color::from(skin.pill));
        draw::draw_rounded_rectf(x, y, w, h, 12);
        draw::set_draw_color(Color::from(skin.accent));
        draw::draw_rounded_rectf(x + 10, cy - 9, 3, 18, 2);
    } else if hovered {
        draw::set_draw_color(Color::from(skin.nav_bg.mix(skin.pill, 450)));
        draw::draw_rounded_rectf(x, y, w, h, 12);
    }
    let icon_color = if active {
        Color::from(skin.accent)
    } else if hovered {
        Color::from(skin.text)
    } else {
        Color::from(skin.dim)
    };
    draw::set_draw_color(icon_color);
    let icon_x = x + 30;
    if home {
        // House: roof triangle, body square and a door notch in the pill.
        draw::draw_polygon(icon_x, cy - 8, icon_x - 8, cy - 1, icon_x + 8, cy - 1);
        draw::draw_rectf(icon_x - 6, cy - 1, 12, 9);
        draw::set_draw_color(if active {
            Color::from(skin.pill)
        } else if hovered {
            Color::from(skin.nav_bg.mix(skin.pill, 450))
        } else {
            Color::from(skin.nav_bg)
        });
        draw::draw_rectf(icon_x - 2, cy + 3, 4, 5);
    } else {
        // Sliders: three lines with moved knobs.
        for (offset, knob) in [(-6, icon_x + 2), (0, icon_x - 3), (6, icon_x + 4)] {
            let line_y = cy + offset;
            draw::draw_line(icon_x - 8, line_y, icon_x + 8, line_y);
            draw::draw_circle_fill(knob, line_y, 6, icon_color);
        }
    }
    let label_color = if active || hovered {
        Color::from(skin.text)
    } else {
        Color::from(skin.dim)
    };
    draw::set_draw_color(label_color);
    draw::set_font(Font::HelveticaBold, 13);
    draw::draw_text(&b.label(), x + 52, cy + 5);
}

/// Brand tile: a rounded accent square with the initial.
fn draw_brand_mark(f: &mut Frame) {
    let skin = skin();
    let (x, y, w, h) = (f.x(), f.y(), f.w(), f.h());
    draw::set_draw_color(Color::from(skin.accent));
    draw::draw_rounded_rectf(x, y, w, h, 10);
    draw::set_font(Font::HelveticaBold, 18);
    draw::set_draw_color(Color::White);
    draw::draw_text2(&f.label(), x, y, w, h, Align::Center);
}

/// Whether the proximity server is currently accepting connections.
static SERVER_LIVE: AtomicBool = AtomicBool::new(false);

/// Status pill at the bottom of the sidebar: colored dot plus a short state.
fn draw_status_pill(f: &mut Frame) {
    let skin = skin();
    let (x, y, w, h) = (f.x(), f.y(), f.w(), f.h());
    let live = SERVER_LIVE.load(Ordering::Relaxed);
    draw::set_draw_color(Color::from(skin.pill));
    draw::draw_rounded_rectf(x, y, w, h, h / 2);
    let dot = if live { skin.success } else { skin.dim };
    draw::set_draw_color(Color::from(dot));
    draw::draw_circle_fill(x + 18, y + h / 2, 8, Color::from(dot));
    draw::set_draw_color(Color::from(if live { skin.text } else { skin.dim }));
    draw::set_font(Font::HelveticaBold, 12);
    draw::draw_text(&f.label(), x + 32, y + h / 2 + 4);
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
                    sender.send(areas).unwrap();
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
