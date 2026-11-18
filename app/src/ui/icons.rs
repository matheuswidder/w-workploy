use std::sync::OnceLock;

use rune::{Icon, Pixels};

fn decode_png_pixels(bytes: &[u8]) -> Pixels {
    let px = Pixels::new();
    if let Ok(img) = image::load_from_memory(bytes) {
        let rgba = img.into_rgba8();
        let (w, h) = rgba.dimensions();
        px.set(w, h, rgba.into_raw());
    }
    px
}

pub fn brand_icon_dark() -> &'static Pixels {
    static PX: OnceLock<Pixels> = OnceLock::new();
    PX.get_or_init(|| decode_png_pixels(include_bytes!("../../assets/icon-medium.png")))
}

pub fn brand_icon_light() -> &'static Pixels {
    static PX: OnceLock<Pixels> = OnceLock::new();
    PX.get_or_init(|| decode_png_pixels(include_bytes!("../../assets/icon-inverted.png")))
}

#[allow(dead_code)]
pub fn brand_icon_transparent() -> &'static Pixels {
    static PX: OnceLock<Pixels> = OnceLock::new();
    PX.get_or_init(|| decode_png_pixels(include_bytes!("../../assets/icon-transparent.png")))
}

pub fn brand_icon_for_theme(dark: bool) -> &'static Pixels {
    if dark {
        brand_icon_dark()
    } else {
        brand_icon_light()
    }
}

pub static PLAY: Icon = Icon {
    name: "w-play",
    view: 16.0,
    body: r#"<path d="M5 3.5L13 8L5 12.5V3.5Z" fill="currentColor"/>"#,
};

pub static CHECK_CIRCLE: Icon = Icon {
    name: "w-check-circle",
    view: 16.0,
    body: r#"<circle cx="8" cy="8" r="6" stroke="currentColor" stroke-width="1.35"/><path d="M5.5 8.2L7.2 9.9L10.7 6.1" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/>"#,
};

pub static X_CIRCLE: Icon = Icon {
    name: "w-x-circle",
    view: 16.0,
    body: r#"<circle cx="8" cy="8" r="6" stroke="currentColor" stroke-width="1.35"/><path d="M5.8 5.8L10.2 10.2M10.2 5.8L5.8 10.2" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/>"#,
};

pub static CLOCK: Icon = Icon {
    name: "w-clock",
    view: 16.0,
    body: r#"<circle cx="8" cy="8" r="6" stroke="currentColor" stroke-width="1.35"/><path d="M8 4.8V8.2L10.3 9.6" stroke="currentColor" stroke-width="1.35" stroke-linecap="round" stroke-linejoin="round"/>"#,
};

pub static BOLT: Icon = Icon {
    name: "w-bolt",
    view: 16.0,
    body: r#"<path d="M9.2 1.8L3.8 9H7.8L6.8 14.2L12.2 7H8.2L9.2 1.8Z" fill="currentColor"/>"#,
};

pub static SHIELD: Icon = Icon {
    name: "w-shield",
    view: 16.0,
    body: r#"<path d="M8 2L13 4V7.8C13 11 10.8 13.4 8 14.3C5.2 13.4 3 11 3 7.8V4L8 2Z" stroke="currentColor" stroke-width="1.35" stroke-linejoin="round"/><path d="M6 8.2L7.4 9.6L10.2 6.6" stroke="currentColor" stroke-width="1.35" stroke-linecap="round" stroke-linejoin="round"/>"#,
};

pub static SERVER: Icon = Icon {
    name: "w-server",
    view: 16.0,
    body: r#"<rect x="2.2" y="2.5" width="11.6" height="4.5" rx="1.2" stroke="currentColor" stroke-width="1.3"/><rect x="2.2" y="9" width="11.6" height="4.5" rx="1.2" stroke="currentColor" stroke-width="1.3"/><circle cx="4.8" cy="4.75" r="0.8" fill="currentColor"/><circle cx="4.8" cy="11.25" r="0.8" fill="currentColor"/>"#,
};

pub static BRANCH: Icon = Icon {
    name: "w-branch",
    view: 16.0,
    body: r#"<circle cx="5" cy="4" r="1.6" stroke="currentColor" stroke-width="1.3"/><circle cx="5" cy="12" r="1.6" stroke="currentColor" stroke-width="1.3"/><circle cx="11.5" cy="6" r="1.6" stroke="currentColor" stroke-width="1.3"/><path d="M5 5.6V10.4M5 10.2C5 8 11.5 9.2 11.5 7.6" stroke="currentColor" stroke-width="1.3"/>"#,
};

pub static COMMIT: Icon = Icon {
    name: "w-commit",
    view: 16.0,
    body: r#"<circle cx="8" cy="8" r="2.4" stroke="currentColor" stroke-width="1.35"/><path d="M1.8 8H5.6M10.4 8H14.2" stroke="currentColor" stroke-width="1.35" stroke-linecap="round"/>"#,
};

pub static REFRESH: Icon = Icon {
    name: "w-refresh",
    view: 16.0,
    body: r#"<path d="M13.2 4.5V7.8H9.9" stroke="currentColor" stroke-width="1.35" stroke-linecap="round" stroke-linejoin="round"/><path d="M12.7 7.6A5.2 5.2 0 1 0 11.8 11.6" stroke="currentColor" stroke-width="1.35" stroke-linecap="round"/>"#,
};

pub static COPY: Icon = Icon {
    name: "w-copy",
    view: 16.0,
    body: r#"<rect x="5.2" y="2.5" width="8.3" height="8.8" rx="1.2" stroke="currentColor" stroke-width="1.25"/><rect x="2.5" y="4.7" width="8.3" height="8.8" rx="1.2" fill="none" stroke="currentColor" stroke-width="1.25"/>"#,
};

pub static EDIT: Icon = Icon {
    name: "w-edit",
    view: 16.0,
    body: r#"<path d="M10.8 2.6L13.4 5.2L5.6 13H3V10.4L10.8 2.6Z" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round"/>"#,
};

pub static TRASH: Icon = Icon {
    name: "w-trash",
    view: 16.0,
    body: r#"<path d="M3 4.5H13M6 4.5V2.8H10V4.5M4.5 4.5L5.2 13.2H10.8L11.5 4.5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"/>"#,
};

pub static CODE: Icon = Icon {
    name: "w-code",
    view: 16.0,
    body: r#"<path d="M5.5 4.5L2 8L5.5 11.5M10.5 4.5L14 8L10.5 11.5M9 3.5L7 12.5" stroke="currentColor" stroke-width="1.35" stroke-linecap="round" stroke-linejoin="round"/>"#,
};

pub static PLUS: Icon = Icon {
    name: "w-plus",
    view: 16.0,
    body: r#"<path d="M8 3V13M3 8H13" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>"#,
};

pub static ARROW_LEFT: Icon = Icon {
    name: "w-arrow-left",
    view: 16.0,
    body: r#"<path d="M13 8H3M7.2 3.8L3 8L7.2 12.2" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/>"#,
};

pub static CHEVRON_DOWN: Icon = Icon {
    name: "w-chevron-down",
    view: 16.0,
    body: r#"<path d="M4.2 6.2L8 10L11.8 6.2" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/>"#,
};

pub static CHEVRON_RIGHT: Icon = Icon {
    name: "w-chevron-right",
    view: 16.0,
    body: r#"<path d="M6.2 4.2L10 8L6.2 11.8" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/>"#,
};

pub static CHEVRON_UP: Icon = Icon {
    name: "w-chevron-up",
    view: 16.0,
    body: r#"<path d="M4.2 9.8L8 6L11.8 9.8" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/>"#,
};

pub static LIST: Icon = Icon {
    name: "w-list",
    view: 16.0,
    body: r#"<path d="M3 4.2H13M3 8H13M3 11.8H10" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/>"#,
};

pub static SIDEBAR_TOGGLE: Icon = Icon {
    name: "w-sidebar",
    view: 16.0,
    body: r#"<rect x="2" y="3" width="12" height="10" rx="2" stroke="currentColor" stroke-width="1.3"/><path d="M6.2 3.2V12.8" stroke="currentColor" stroke-width="1.3"/>"#,
};

pub static SUN_MOON: Icon = Icon {
    name: "w-theme",
    view: 16.0,
    body: r#"<circle cx="8" cy="8" r="4.5" stroke="currentColor" stroke-width="1.35"/><path d="M8 3.5V12.5A4.5 4.5 0 0 0 8 3.5Z" fill="currentColor"/>"#,
};

pub static SLIDERS: Icon = Icon {
    name: "w-sliders",
    view: 16.0,
    body: r#"<path d="M2.5 5H13.5M2.5 11H13.5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/><circle cx="5.5" cy="5" r="1.6" fill="currentColor"/><circle cx="10.5" cy="11" r="1.6" fill="currentColor"/>"#,
};

pub static HOME: Icon = Icon {
    name: "w-home",
    view: 16.0,
    body: r#"<path d="M2.5 7.2L8 2.5L13.5 7.2V13.2C13.5 13.6 13.2 14 12.7 14H3.3C2.8 14 2.5 13.6 2.5 13.2V7.2Z" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round"/><path d="M6.2 14V9.5H9.8V14" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round"/>"#,
};

pub static ACTIVITY: Icon = Icon {
    name: "w-activity",
    view: 16.0,
    body: r#"<path d="M1.8 8H4.2L5.8 3.5L8.2 12.5L10.4 6.2L11.8 8H14.2" stroke="currentColor" stroke-width="1.35" stroke-linecap="round" stroke-linejoin="round"/>"#,
};

pub static INFO: Icon = Icon {
    name: "w-info",
    view: 16.0,
    body: r#"<circle cx="8" cy="8" r="6" stroke="currentColor" stroke-width="1.35"/><path d="M8 7.2V11.2" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/><circle cx="8" cy="5.1" r="0.85" fill="currentColor"/>"#,
};

pub static EXTERNAL_LINK: Icon = Icon {
    name: "w-external",
    view: 16.0,
    body: r#"<path d="M9.5 2.5H13.5V6.5M13.3 2.7L7.5 8.5" stroke="currentColor" stroke-width="1.35" stroke-linecap="round" stroke-linejoin="round"/><path d="M12.5 9.2V12.7C12.5 13.1 12.1 13.5 11.7 13.5H3.3C2.9 13.5 2.5 13.1 2.5 12.7V4.3C2.5 3.9 2.9 3.5 3.3 3.5H6.8" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/>"#,
};
