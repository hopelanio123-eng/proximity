use std::cmp::min;
use std::io::Cursor;
use std::iter::Iterator;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::AtomicBool;

use fltk::app;
use fltk::enums::{Align, Color, Font, FrameType, LabelType};
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

use crate::config::{write_config, Config, ThemeType};
use crate::protocol::{CustomInputAreas, Rect};
use crate::web::Web2UiMessage::UInputInaccessible;

pub fn run(config: &Config, log_receiver: mpsc::Receiver<String>) {
    let width = 200;
    let height = 30;
    let padding = 10;

    // Desktop shell: a fixed navigation sidebar plus a page area holding the
    // "Главная" (home) and "Настройки" (settings) views.
    const NAV_W: i32 = 176;
    const WIN_W: i32 = 680;
    const WIN_H: i32 = 660;
    const CONTENT_W: i32 = WIN_W - NAV_W;
    const PAGE_W: i32 = CONTENT_W - 48;

    let app = App::default().with_scheme(fltk::app::AppScheme::Gtk);
    config.gui_theme.map(|th| th.apply());
    let mut wind = Window::default()
        .with_size(WIN_W, WIN_H)
        .center_screen()
        .with_label(&format!("Proximity - {}", env!("CARGO_PKG_VERSION")));
    wind.set_xclass("proximity");
    wind.set_callback(move |_win| app.quit());
    wind.set_frame(FrameType::FlatBox);
    wind.set_color(Color::from_rgb(14, 17, 22));
    wind.set_id("main_window");

    // --- Sidebar with the "Главная" / "Настройки" navigation --------------
    let mut nav_bar = Group::default()
        .with_pos(0, 0)
        .with_size(NAV_W, WIN_H)
        .with_id("nav_bar");

    Frame::default()
        .with_size(40, 40)
        .with_pos(16, 14)
        .with_align(Align::Center | Align::Inside.bits())
        .with_label("P")
        .with_id("brand_mark");
    Frame::default()
        .with_size(104, 20)
        .with_pos(64, 17)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label("Proximity")
        .with_id("head_title");
    Frame::default()
        .with_size(104, 14)
        .with_pos(66, 39)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label(&format!("v{}", env!("CARGO_PKG_VERSION")))
        .with_id("head_sub");

    Frame::default()
        .with_size(140, 16)
        .with_pos(16, 78)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label("МЕНЮ")
        .with_id("nav_caption");

    let mut nav_home_btn = Button::default()
        .with_size(148, 32)
        .with_pos(14, 98)
        .with_label("Главная")
        .with_id("nav_home");
    let mut nav_settings_btn = Button::default()
        .with_size(148, 32)
        .with_pos(14, 132)
        .with_label("Настройки")
        .with_id("nav_settings");

    let mut status_pill = Frame::default()
        .with_size(148, 30)
        .with_pos(14, WIN_H - 62)
        .with_align(Align::Center | Align::Inside.bits())
        .with_label("Server offline")
        .with_id("status_pill");
    status_pill.set_tooltip("Shows whether the Proximity server is accepting connections.");

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

    Frame::default()
        .with_size(PAGE_W, 40)
        .with_pos(24, 18)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label("Control your PC from any device\nwith a browser.")
        .with_id("home_caption");

    let mut output_server_addr = Output::default()
        .with_size(330, height)
        .with_pos(150, 76)
        .with_label("Connect to:")
        .with_id("output_addr");
    output_server_addr.hide();

    let mut but_toggle = Button::default()
        .with_size(150, 34)
        .with_pos(150, 114)
        .with_label("Start")
        .with_id("btn_toggle");

    let mut qr_frame = Frame::default()
        .with_size(235, 235)
        .with_pos(245, 114)
        .with_id("qr_frame");
    qr_frame.hide();

    let output_buf = TextBuffer::default();
    let mut output = TextDisplay::default()
        .with_size(PAGE_W, 250)
        .with_pos(24, 390)
        .with_id("log_display");
    output.set_buffer(output_buf);
    let output_buf = output.buffer().unwrap();

    page_home.end();

    // --- Settings page -----------------------------------------------------
    let mut page_settings = Group::default()
        .with_pos(NAV_W, 0)
        .with_size(CONTENT_W, WIN_H)
        .with_id("page_settings");

    Frame::default()
        .with_size(PAGE_W, 16)
        .with_pos(24, 18)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label("ОФОРМЛЕНИЕ")
        .with_id("sec_skin");

    let mut choice_theme = Choice::default()
        .with_size(330, height)
        .with_pos(150, 40)
        .with_label("Theme")
        .with_id("choice_theme");

    for theme in ThemeType::themes() {
        choice_theme.add_choice(&theme.name());
    }
    choice_theme.set_value(config.gui_theme.unwrap_or(ThemeType::default()).to_index());

    Frame::default()
        .with_size(PAGE_W, 16)
        .with_pos(24, 90)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label("СЕТЬ И ДОСТУП")
        .with_id("sec_net");

    let mut input_access_code = Input::default()
        .with_pos(150, 112)
        .with_size(330, height)
        .with_label("Access code")
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
        .with_size(330, height)
        .with_pos(150, 150)
        .with_label("Bind Address")
        .with_id("input_bind");
    input_bind_addr.set_value(&config.bind_address.to_string());

    let mut input_port = IntInput::default()
        .with_size(330, height)
        .with_pos(150, 188)
        .with_label("Port")
        .with_id("input_port");
    input_port.set_value(&config.web_port.to_string());

    let mut check_auto_start = CheckButton::default()
        .with_size(330, height)
        .with_pos(150, 228)
        .with_label("Auto Start")
        .with_id("check_auto");
    check_auto_start.set_tooltip("Start Proximity server immediately on program start.");
    check_auto_start.set_checked(config.auto_start);

    #[cfg(target_os = "linux")]
    let mut check_wayland = CheckButton::default()
        .with_size(330, height)
        .with_pos(150, 258)
        .with_label("Wayland / PipeWire Support")
        .with_id("check_wayland");
    #[cfg(target_os = "linux")]
    {
        check_wayland.set_tooltip(
            "EXPERIMENTAL! This may crash your desktop! Enables screen \
        capturing for Wayland using PipeWire and GStreamer.",
        );
        check_wayland.set_checked(config.wayland_support);
    }

    Frame::default()
        .with_size(PAGE_W, 16)
        .with_pos(24, 312)
        .with_align(Align::Left | Align::Inside.bits())
        .with_label("УСКОРЕНИЕ")
        .with_id("sec_hw");

    let mut label_hw_accel = Frame::default()
        .with_size(330, height)
        .with_pos(150, 334)
        .with_label("Try Hardware acceleration")
        .with_id("label_hw");
    label_hw_accel.set_tooltip(
        "On many systems video encoding can be done with hardware \
        acceleration. By default this is disabled as the quality and stability of video encoding \
        varies greatly among hardware and drivers. Currently this is only supported on Linux.",
    );

    let mut check_native_hw_accel = CheckButton::default()
        .with_size(160, height)
        .with_pos(150, 364)
        .with_id("check_native");

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
        check_native_hw_accel.set_label("Media-\nFoundation");
        check_native_hw_accel
            .set_tooltip("Try to use hardware acceleration through the MediaFoundation API.");
        check_native_hw_accel.set_checked(config.try_mediafoundation);
    }

    let mut check_nvenc = CheckButton::default()
        .with_size(164, height)
        .with_pos(316, 364)
        .with_label("NVENC")
        .with_id("check_nvenc");
    check_nvenc.set_tooltip("Try to use Nvidia's NVENC to encode the video via GPU.");

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    check_nvenc.set_checked(config.try_nvenc);

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        check_nvenc.deactivate();
        check_nvenc.hide();
    }

    page_settings.end();
    page_settings.hide();

    nav_home_btn.set_callback(|_| show_page(true));
    nav_settings_btn.set_callback(|_| show_page(false));

    wind.make_resizable(true);
    wind.end();
    wind.show();
    apply_skin();
    show_page(true);

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
            if v >= 0 {
                ThemeType::from_index(v).apply();
                let home = app::widget_from_id::<Group>("page_home")
                    .map(|g| g.visible())
                    .unwrap_or(true);
                apply_skin();
                show_page(home);
                config.lock().unwrap().gui_theme = Some(ThemeType::from_index(v));
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
                    config.gui_theme = Some(ThemeType::from_index(choice_theme.value()));
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
                if let Some(mut pill) = app::widget_from_id::<Frame>("status_pill") {
                    pill.set_label("Server live");
                    pill.set_label_color(Color::from_rgb(52, 211, 153));
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
                but.set_label("Stop");
            } else {
                proximity.stop();
                but.set_label("Start");
                output_server_addr.hide();
                qr_frame.resize_callback(|_, _, _, _, _| {});
                qr_frame.hide();
                is_server_running = false;
                if let Some(mut pill) = app::widget_from_id::<Frame>("status_pill") {
                    pill.set_label("Server offline");
                    pill.set_label_color(Color::from_rgb(139, 150, 165));
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

/// Pill-shaped highlight for the active sidebar entry, flat and dimmed for
/// the inactive ones.
fn style_nav_items() {
    let home_active = *HOME_ACTIVE.lock().unwrap();
    let bg = Color::from_rgb(14, 17, 22);
    let surface = Color::from_rgb(30, 35, 42);
    let text = Color::from_rgb(232, 236, 241);
    let dim = Color::from_rgb(139, 150, 165);
    for (id, active) in [("nav_home", home_active), ("nav_settings", !home_active)] {
        if let Some(mut b) = app::widget_from_id::<Button>(id) {
            if active {
                b.set_frame(FrameType::RoundedBox);
                b.set_color(surface);
                b.set_label_color(text);
                b.set_label_font(Font::HelveticaBold);
            } else {
                b.set_frame(FrameType::FlatBox);
                b.set_color(bg);
                b.set_label_color(dim);
                b.set_label_font(Font::Helvetica);
            }
            b.set_label_size(13);
            b.redraw();
        }
    }
}

/// Modern dark "Clean Sheet" skin applied on top of (or instead of) the
/// classic fltk themes. Every styled widget is looked up by id, so this can
/// be re-run at any time - in particular right after a theme switch, which
/// would otherwise repaint everything in the classic grey look.
fn apply_skin() {
    let bg = Color::from_rgb(14, 17, 22);
    let surface = Color::from_rgb(30, 35, 42);
    let accent = Color::from_rgb(59, 130, 246);
    let accent_pressed = Color::from_rgb(37, 99, 235);
    let text = Color::from_rgb(232, 236, 241);
    let dim = Color::from_rgb(139, 150, 165);
    let log_text = Color::from_rgb(160, 170, 182);

    for id in ["head_title", "head_sub", "label_hw"] {
        if let Some(mut f) = app::widget_from_id::<Frame>(id) {
            f.set_frame(FrameType::FlatBox);
            f.set_color(bg);
        }
    }
    if let Some(mut f) = app::widget_from_id::<Frame>("head_title") {
        f.set_label_color(text);
        f.set_label_font(Font::HelveticaBold);
        f.set_label_size(22);
    }
    if let Some(mut f) = app::widget_from_id::<Frame>("head_sub") {
        f.set_label_color(dim);
        f.set_label_size(12);
    }
    if let Some(mut f) = app::widget_from_id::<Frame>("label_hw") {
        f.set_label_color(text);
        f.set_label_size(13);
    }
    if let Some(mut f) = app::widget_from_id::<Frame>("brand_mark") {
        f.set_frame(FrameType::RoundedBox);
        f.set_color(accent);
        f.set_label_color(Color::White);
        f.set_label_font(Font::HelveticaBold);
        f.set_label_size(20);
    }
    if let Some(mut f) = app::widget_from_id::<Frame>("status_pill") {
        f.set_frame(FrameType::RoundedBox);
        f.set_color(surface);
        f.set_label_size(12);
        f.set_label_font(Font::HelveticaBold);
    }
    if let Some(mut f) = app::widget_from_id::<Frame>("head_divider") {
        f.set_frame(FrameType::FlatBox);
        f.set_color(Color::from_rgb(42, 49, 58));
    }
    if let Some(mut f) = app::widget_from_id::<Frame>("nav_caption") {
        f.set_frame(FrameType::FlatBox);
        f.set_color(bg);
        f.set_label_color(dim);
        f.set_label_size(11);
        f.set_label_font(Font::HelveticaBold);
    }
    if let Some(mut f) = app::widget_from_id::<Frame>("home_caption") {
        f.set_frame(FrameType::FlatBox);
        f.set_color(bg);
        f.set_label_color(text);
        f.set_label_size(16);
        f.set_label_font(Font::HelveticaBold);
    }
    if let Some(mut f) = app::widget_from_id::<Frame>("sec_skin") {
        f.set_frame(FrameType::FlatBox);
        f.set_color(bg);
        f.set_label_color(dim);
        f.set_label_size(11);
        f.set_label_font(Font::HelveticaBold);
    }
    if let Some(mut f) = app::widget_from_id::<Frame>("sec_net") {
        f.set_frame(FrameType::FlatBox);
        f.set_color(bg);
        f.set_label_color(dim);
        f.set_label_size(11);
        f.set_label_font(Font::HelveticaBold);
    }
    if let Some(mut f) = app::widget_from_id::<Frame>("sec_hw") {
        f.set_frame(FrameType::FlatBox);
        f.set_color(bg);
        f.set_label_color(dim);
        f.set_label_size(11);
        f.set_label_font(Font::HelveticaBold);
    }

    for id in ["input_access", "input_bind"] {
        if let Some(mut w) = app::widget_from_id::<Input>(id) {
            w.set_frame(FrameType::FlatBox);
            w.set_color(surface);
            w.set_label_color(dim);
            w.set_text_color(text);
            w.set_text_font(Font::Helvetica);
            w.set_text_size(13);
            w.set_cursor_color(accent);
        }
    }
    if let Some(mut w) = app::widget_from_id::<IntInput>("input_port") {
        w.set_frame(FrameType::FlatBox);
        w.set_color(surface);
        w.set_label_color(dim);
        w.set_text_color(text);
        w.set_text_font(Font::Helvetica);
        w.set_text_size(13);
        w.set_cursor_color(accent);
    }
    if let Some(mut c) = app::widget_from_id::<Choice>("choice_theme") {
        c.set_frame(FrameType::FlatBox);
        c.set_color(surface);
        c.set_label_color(dim);
        c.set_text_color(text);
        c.set_text_size(13);
    }
    if let Some(mut o) = app::widget_from_id::<Output>("output_addr") {
        o.set_frame(FrameType::FlatBox);
        o.set_color(surface);
        o.set_label_color(dim);
        o.set_text_color(accent);
        o.set_text_font(Font::HelveticaBold);
        o.set_text_size(13);
    }
    for id in ["check_auto", "check_native", "check_nvenc", "check_wayland"] {
        if let Some(mut c) = app::widget_from_id::<CheckButton>(id) {
            c.set_label_color(text);
            c.set_label_size(13);
            c.set_color(surface);
            c.set_selection_color(accent);
        }
    }
    if let Some(mut b) = app::widget_from_id::<Button>("btn_toggle") {
        b.set_frame(FrameType::RoundedBox);
        b.set_color(accent);
        b.set_label_color(Color::White);
        b.set_label_size(16);
        b.set_label_font(Font::HelveticaBold);
        b.set_down_frame(FrameType::FlatBox);
        b.set_selection_color(accent_pressed);
    }
    if let Some(mut t) = app::widget_from_id::<TextDisplay>("log_display") {
        t.set_frame(FrameType::FlatBox);
        t.set_color(surface);
        t.set_text_color(log_text);
        t.set_text_font(Font::Courier);
        t.set_text_size(11);
    }
    if let Some(mut f) = app::widget_from_id::<Frame>("qr_frame") {
        f.set_frame(FrameType::FlatBox);
        f.set_color(Color::White);
    }
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
