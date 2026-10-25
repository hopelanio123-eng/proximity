use std::net::IpAddr;
use std::{fs, path::PathBuf};

use clap::Parser;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

#[derive(Serialize, Deserialize, Parser, Debug, Clone)]
#[command(version, about, long_about = None)]
pub struct Config {
    #[arg(long, help = "Access code")]
    pub access_code: Option<String>,
    #[arg(long, default_value = "0.0.0.0", help = "Bind address")]
    pub bind_address: IpAddr,
    #[arg(long, default_value = "1701", help = "Web port")]
    pub web_port: u16,
    #[cfg(target_os = "linux")]
    #[arg(
        long,
        help = "Try to use hardware acceleration through the Video Acceleration API."
    )]
    pub try_vaapi: bool,
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[arg(long, help = "Try to use Nvidia's NVENC to encode the video via GPU.")]
    #[serde(default)]
    pub try_nvenc: bool,
    #[cfg(target_os = "macos")]
    #[arg(
        long,
        help = "Try to use hardware acceleration through the VideoToolbox API."
    )]
    #[serde(default)]
    pub try_videotoolbox: bool,
    #[cfg(target_os = "windows")]
    #[arg(
        long,
        help = "Try to use hardware acceleration through the MediaFoundation API."
    )]
    #[serde(default)]
    pub try_mediafoundation: bool,
    #[arg(long, help = "Start Proximity server immediately on program start.")]
    #[serde(default)]
    pub auto_start: bool,
    #[arg(long, help = "Gui theme; one of the palettes offered in the settings window")]
    #[serde(default)]
    pub gui_theme: Option<String>,
    #[arg(long, help = "Run Proximity without gui and start immediately.")]
    #[serde(default)]
    pub no_gui: bool,
    #[cfg(target_os = "linux")]
    #[arg(long, help = "Wayland/PipeWire Support.")]
    #[serde(default)]
    pub wayland_support: bool,

    #[arg(long, help = "Print template of index.html served by Proximity.")]
    #[serde(skip)]
    pub print_index_html: bool,
    #[arg(long, help = "Print access.html served by Proximity.")]
    #[serde(skip)]
    pub print_access_html: bool,
    #[arg(long, help = "Print style.css served by Proximity.")]
    #[serde(skip)]
    pub print_style_css: bool,
    #[arg(long, help = "Print lib.js served by Proximity.")]
    #[serde(skip)]
    pub print_lib_js: bool,

    #[arg(
        long,
        help = "Use custom template of index.html to be served by Proximity."
    )]
    #[serde(skip)]
    pub custom_index_html: Option<PathBuf>,
    #[arg(long, help = "Use custom access.html to be served by Proximity.")]
    #[serde(skip)]
    pub custom_access_html: Option<PathBuf>,
    #[arg(long, help = "Use custom style.css to be served by Proximity.")]
    #[serde(skip)]
    pub custom_style_css: Option<PathBuf>,
    #[arg(long, help = "Use custom lib.js to be served by Proximity.")]
    #[serde(skip)]
    pub custom_lib_js: Option<PathBuf>,

    #[arg(long, help = "Print shell completions for given shell.")]
    #[serde(skip)]
    pub completions: Option<clap_complete::Shell>,
}

pub fn read_config() -> Option<Config> {
    if let Some(mut config_path) = dirs::config_dir() {
        config_path.push("proximity");
        config_path.push("proximity.toml");
        match fs::read_to_string(&config_path) {
            Ok(s) => match toml::from_str(&s) {
                Ok(c) => Some(c),
                Err(e) => {
                    warn!("Failed to read configuration file: {}", e);
                    None
                }
            },
            Err(err) => {
                match err.kind() {
                    std::io::ErrorKind::NotFound => {
                        debug!("Failed to read configuration file: {}", err)
                    }
                    _ => warn!("Failed to read configuration file: {}", err),
                }
                None
            }
        }
    } else {
        None
    }
}

pub fn write_config(conf: &Config) {
    match dirs::config_dir() {
        Some(mut config_path) => {
            config_path.push("proximity");
            if !config_path.exists() {
                if let Err(err) = fs::create_dir_all(&config_path) {
                    warn!("Failed create directory for configuration: {}", err);
                    return;
                }
            }
            config_path.push("proximity.toml");
            if let Err(err) = fs::write(
                config_path,
                &toml::to_string_pretty(&conf).expect("Failed to encode config to toml."),
            ) {
                warn!("Failed to write configuration file: {}", err);
            }
        }
        None => {
            warn!("Failed to find configuration directory!");
        }
    }
}

pub fn get_config() -> Config {
    let args: Vec<String> = std::env::args().collect();
    if let Some(mut config) = read_config() {
        if args.len() > 1 {
            // clap's `update_from` resets every argument not present on the
            // command line to its default (boolean flags are SetTrue), which
            // silently clobbers values from the configuration file. Instead
            // parse the args and copy only the ones that actually differ from
            // the pure clap baseline, i.e. were provided by the user.
            if let Ok(cli) = Config::try_parse_from(args.clone()) {
                if let Ok(defaults) = Config::try_parse_from(["proximity"]) {
                    if cli.access_code != defaults.access_code {
                        config.access_code = cli.access_code;
                    }
                    if cli.bind_address != defaults.bind_address {
                        config.bind_address = cli.bind_address;
                    }
                    if cli.web_port != defaults.web_port {
                        config.web_port = cli.web_port;
                    }
                    #[cfg(any(target_os = "linux", target_os = "windows"))]
                    if cli.try_nvenc != defaults.try_nvenc {
                        config.try_nvenc = cli.try_nvenc;
                    }
                    #[cfg(target_os = "linux")]
                    if cli.try_vaapi != defaults.try_vaapi {
                        config.try_vaapi = cli.try_vaapi;
                    }
                    #[cfg(target_os = "linux")]
                    if cli.wayland_support != defaults.wayland_support {
                        config.wayland_support = cli.wayland_support;
                    }
                    #[cfg(target_os = "macos")]
                    if cli.try_videotoolbox != defaults.try_videotoolbox {
                        config.try_videotoolbox = cli.try_videotoolbox;
                    }
                    #[cfg(target_os = "windows")]
                    if cli.try_mediafoundation != defaults.try_mediafoundation {
                        config.try_mediafoundation = cli.try_mediafoundation;
                    }
                    if cli.auto_start != defaults.auto_start {
                        config.auto_start = cli.auto_start;
                    }
                    if cli.no_gui != defaults.no_gui {
                        config.no_gui = cli.no_gui;
                    }
                    if cli.gui_theme != defaults.gui_theme {
                        config.gui_theme = cli.gui_theme;
                    }
                    if cli.print_index_html {
                        config.print_index_html = true;
                    }
                    if cli.print_access_html {
                        config.print_access_html = true;
                    }
                    if cli.print_style_css {
                        config.print_style_css = true;
                    }
                    if cli.print_lib_js {
                        config.print_lib_js = true;
                    }
                    if cli.custom_index_html.is_some() {
                        config.custom_index_html = cli.custom_index_html;
                    }
                    if cli.custom_access_html.is_some() {
                        config.custom_access_html = cli.custom_access_html;
                    }
                    if cli.custom_style_css.is_some() {
                        config.custom_style_css = cli.custom_style_css;
                    }
                    if cli.custom_lib_js.is_some() {
                        config.custom_lib_js = cli.custom_lib_js;
                    }
                    if cli.completions.is_some() {
                        config.completions = cli.completions;
                    }
                } else {
                    config.update_from(args.clone());
                }
            } else {
                config.update_from(args);
            }
        }
        config
    } else {
        Config::parse()
    }
}
