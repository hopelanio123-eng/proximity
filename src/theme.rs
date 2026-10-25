//! Application themes.
//!
//! The palettes are ports of the built-in theme presets of the anime launcher
//! (`src/domain/settings/theme-presets.ts`): the flat backgrounds, panels and
//! accents of Discord's own gradient themes, Teleboss ports and the classic
//! palettes (Dracula, Nord, Tokyo Night, Gruvbox, GitHub Dark...). A couple of
//! light presets are kept as well.
//!
//! Only colors survive the port: FLTK draws solid shapes, so the radial and
//! linear gradients of the originals are translated into a single ambient glow -
//! the accent-tinted top edge of the window - and a "glow" color pulled from
//! each preset's `--slider-button-active-color` stops.

use fltk::enums::Color;

pub struct Palette {
    /// Display name, same as in the anime launcher.
    pub name: &'static str,
    /// `dark` or `light` base of the preset, decides text colors.
    pub light: bool,
    /// `--background-color`: window and sidebar background.
    pub bg: Rgb24,
    /// `--alt-background-color`: cards, inputs, log view.
    pub surface: Rgb24,
    /// `--accent-fill-color`: primary button, checks, selection.
    pub accent: Rgb24,
    /// `--accent-fill-hover-color`: pressed/selected state of the accent.
    pub accent_pressed: Rgb24,
    /// `--slider-button-active-color`: the bright companion of the accent,
    /// used for the ambient glow and small highlights.
    pub glow: Rgb24,
}

#[derive(Clone, Copy)]
pub struct Rgb24(u32);

impl Rgb24 {
    /// Build from a `0xRRGGBB` literal.
    pub const fn hex(h: u32) -> Self {
        Rgb24(h)
    }
    pub const fn r(self: &Self) -> u8 {
        (self.0 >> 16) as u8
    }
    pub const fn g(self: &Self) -> u8 {
        (self.0 >> 8) as u8
    }
    pub const fn b(self: &Self) -> u8 {
        self.0 as u8
    }
    /// Linear blend towards `other` by `t` (0..1).
    pub const fn mix(&self, other: Rgb24, t: u32) -> Rgb24 {
        // t is in thousandths; const-friendly integer math only.
        assert!(t <= 1000);
        let r = (self.r() as u32 * (1000 - t) + other.r() as u32 * t) / 1000;
        let g = (self.g() as u32 * (1000 - t) + other.g() as u32 * t) / 1000;
        let b = (self.b() as u32 * (1000 - t) + other.b() as u32 * t) / 1000;
        Rgb24((r << 16) | (g << 8) | b)
    }
}

impl From<Rgb24> for Color {
    fn from(c: Rgb24) -> Color {
        Color::from_rgb(c.r(), c.g(), c.b())
    }
}

/// Everything the skin needs, derived from a palette plus base shades.
pub struct Skin {
    pub bg_start: Rgb24, // page background, ambient glow tinted top
    pub bg_end: Rgb24,   // page background bottom
    pub nav_bg: Rgb24,   // sidebar background
    pub surface: Rgb24,  // cards and inputs
    pub pill: Rgb24,     // active nav item / status pill
    pub divider: Rgb24,
    pub accent: Rgb24,
    pub accent_pressed: Rgb24,
    pub text: Rgb24,
    pub dim: Rgb24,
    pub log_text: Rgb24,
    pub light: bool,
}

impl Skin {
    pub fn from_palette(p: &Palette) -> Skin {
        // Ambient glow: the top of the window is misted with the accent,
        // echoing the radial gradients of the originals ("glow at the top").
        let bg_start = p.bg.mix(p.glow, 60);
        let navy = if p.light {
            Rgb24::hex(0x2b2113)
        } else {
            Rgb24::hex(0x000000)
        };
        // Sidebar sits a touch darker than the page so the divider reads.
        let nav_bg = p.bg.mix(navy, 150);
        // Active pill: slightly lifted surface (works on light and dark).
        let towards = if p.light {
            Rgb24::hex(0x000000)
        } else {
            Rgb24::hex(0xffffff)
        };
        let pill = p.surface.mix(towards, 55);
        let divider = p.bg.mix(towards, 90);
        let (text, dim, log_text) = if p.light {
            (
                Rgb24::hex(0x22273a),
                Rgb24::hex(0x63707f),
                Rgb24::hex(0x4a5561),
            )
        } else {
            (
                Rgb24::hex(0xe8ecf1),
                Rgb24::hex(0x8b96a5),
                Rgb24::hex(0xa0aab6),
            )
        };
        Skin {
            bg_start,
            bg_end: p.bg,
            nav_bg,
            surface: p.surface,
            pill,
            divider,
            accent: p.accent,
            accent_pressed: p.accent_pressed,
            text,
            dim,
            log_text,
            light: p.light,
        }
    }
}

pub const PALETTES: &[Palette] = &[
    Palette {
        name: "Гитхаб Дарк",
        light: false,
        bg: Rgb24::hex(0x0d1117),
        surface: Rgb24::hex(0x161b22),
        accent: Rgb24::hex(0x1f6feb),
        accent_pressed: Rgb24::hex(0x388bfd),
        glow: Rgb24::hex(0x58a6ff),
    },
    Palette {
        name: "Оникс",
        light: false,
        bg: Rgb24::hex(0x08070b),
        surface: Rgb24::hex(0x0f0d14),
        accent: Rgb24::hex(0x4f59e8),
        accent_pressed: Rgb24::hex(0x7070f5),
        glow: Rgb24::hex(0x8b93ff),
    },
    Palette {
        name: "Пепел",
        light: false,
        bg: Rgb24::hex(0x211e28),
        surface: Rgb24::hex(0x2a2733),
        accent: Rgb24::hex(0x2b6fd6),
        accent_pressed: Rgb24::hex(0x3f80e0),
        glow: Rgb24::hex(0x61afef),
    },
    Palette {
        name: "Дракула",
        light: false,
        bg: Rgb24::hex(0x1e1030),
        surface: Rgb24::hex(0x281640),
        accent: Rgb24::hex(0x7c3aed),
        accent_pressed: Rgb24::hex(0x8b5cf6),
        glow: Rgb24::hex(0xbd93f9),
    },
    Palette {
        name: "Норд",
        light: false,
        bg: Rgb24::hex(0x2e3440),
        surface: Rgb24::hex(0x343b48),
        accent: Rgb24::hex(0x3d5a80),
        accent_pressed: Rgb24::hex(0x4c6da0),
        glow: Rgb24::hex(0x88c0d0),
    },
    Palette {
        name: "Токийская ночь",
        light: false,
        bg: Rgb24::hex(0x1a1b26),
        surface: Rgb24::hex(0x222736),
        accent: Rgb24::hex(0x3d59a1),
        accent_pressed: Rgb24::hex(0x5472cf),
        glow: Rgb24::hex(0x7aa2f7),
    },
    Palette {
        name: "Мокка",
        light: false,
        bg: Rgb24::hex(0x1e1e2e),
        surface: Rgb24::hex(0x282838),
        accent: Rgb24::hex(0x7c5cd6),
        accent_pressed: Rgb24::hex(0x8d6ce8),
        glow: Rgb24::hex(0xcba6f7),
    },
    Palette {
        name: "Грувбокс",
        light: false,
        bg: Rgb24::hex(0x282828),
        surface: Rgb24::hex(0x322e2c),
        accent: Rgb24::hex(0xaf3a03),
        accent_pressed: Rgb24::hex(0xd65d0e),
        glow: Rgb24::hex(0xfe8019),
    },
    Palette {
        name: "Уан Дарк",
        light: false,
        bg: Rgb24::hex(0x282c34),
        surface: Rgb24::hex(0x2a303a),
        accent: Rgb24::hex(0x2b6fd6),
        accent_pressed: Rgb24::hex(0x3f80e0),
        glow: Rgb24::hex(0x61afef),
    },
    Palette {
        name: "Океан",
        light: false,
        bg: Rgb24::hex(0x04121a),
        surface: Rgb24::hex(0x0a2534),
        accent: Rgb24::hex(0x1a7d9c),
        accent_pressed: Rgb24::hex(0x2195b5),
        glow: Rgb24::hex(0x38c6f4),
    },
    Palette {
        name: "Фиолет",
        light: false,
        bg: Rgb24::hex(0x0d0716),
        surface: Rgb24::hex(0x1c102f),
        accent: Rgb24::hex(0x6f4de0),
        accent_pressed: Rgb24::hex(0x7f5ef0),
        glow: Rgb24::hex(0x9d7bff),
    },
    Palette {
        name: "Бездна",
        light: false,
        bg: Rgb24::hex(0x050f1e),
        surface: Rgb24::hex(0x10233c),
        accent: Rgb24::hex(0x2b6fd6),
        accent_pressed: Rgb24::hex(0x3d80e6),
        glow: Rgb24::hex(0x4da3ff),
    },
    Palette {
        name: "Ноктюрн",
        light: false,
        bg: Rgb24::hex(0x0a090f),
        surface: Rgb24::hex(0x171624),
        accent: Rgb24::hex(0x5f5fe8),
        accent_pressed: Rgb24::hex(0x7070f5),
        glow: Rgb24::hex(0x8b93ff),
    },
    Palette {
        name: "Багровая луна",
        light: false,
        bg: Rgb24::hex(0x1f1414),
        surface: Rgb24::hex(0x302222),
        accent: Rgb24::hex(0x950909),
        accent_pressed: Rgb24::hex(0xb01818),
        glow: Rgb24::hex(0xf22727),
    },
    Palette {
        name: "Марс",
        light: false,
        bg: Rgb24::hex(0x1e1615),
        surface: Rgb24::hex(0x2f2423),
        accent: Rgb24::hex(0x8b4141),
        accent_pressed: Rgb24::hex(0xa25858),
        glow: Rgb24::hex(0xb66363),
    },
    Palette {
        name: "Неоновые ночи",
        light: false,
        bg: Rgb24::hex(0x131221),
        surface: Rgb24::hex(0x201e33),
        accent: Rgb24::hex(0x077f77),
        accent_pressed: Rgb24::hex(0x0f9c92),
        glow: Rgb24::hex(0x25f4e7),
    },
    Palette {
        name: "Под водой",
        light: false,
        bg: Rgb24::hex(0x181b1a),
        surface: Rgb24::hex(0x272b29),
        accent: Rgb24::hex(0x3d7f68),
        accent_pressed: Rgb24::hex(0x529a80),
        glow: Rgb24::hex(0x64b498),
    },
    Palette {
        name: "Цитрусовый щербет",
        light: true,
        bg: Rgb24::hex(0xf8f6f4),
        surface: Rgb24::hex(0xf6f3f1),
        accent: Rgb24::hex(0xb94513),
        accent_pressed: Rgb24::hex(0xb94513),
        glow: Rgb24::hex(0xee8558),
    },
    Palette {
        name: "Сахарная вата",
        light: true,
        bg: Rgb24::hex(0xf7f4f8),
        surface: Rgb24::hex(0xf4f1f6),
        accent: Rgb24::hex(0x0a34c2),
        accent_pressed: Rgb24::hex(0x0a34c2),
        glow: Rgb24::hex(0x6485f7),
    },
    Palette {
        name: "Сакура",
        light: true,
        bg: Rgb24::hex(0xf8f6f4),
        surface: Rgb24::hex(0xf6f4f1),
        accent: Rgb24::hex(0x926f16),
        accent_pressed: Rgb24::hex(0x926f16),
        glow: Rgb24::hex(0xe9c871),
    },
    Palette {
        name: "Восход",
        light: true,
        bg: Rgb24::hex(0xf7f6f5),
        surface: Rgb24::hex(0xf5f3f2),
        accent: Rgb24::hex(0x946238),
        accent_pressed: Rgb24::hex(0x946238),
        glow: Rgb24::hex(0xc49064),
    },
];

pub const DEFAULT_PALETTE: &Palette = &PALETTES[0];

/// Look up a palette by its display name; forgiving about case and failure,
/// a miss falls back to the default entry (old configs hold legacy names).
pub fn palette_by_name(name: &str) -> Option<&'static Palette> {
    PALETTES
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(name.trim()))
}
