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

pub static WORKFLOW: Icon = Icon {
    name: "w-workflow",
    view: 24.0,
    body: r#"<path fill="currentColor" fill-rule="evenodd" d="M12.629 1.25H11.37c-.395 0-.736 0-1.017.023a2.3 2.3 0 0 0-.875.222a2.25 2.25 0 0 0-.984.984a2.3 2.3 0 0 0-.222.875c-.023.28-.023.622-.023 1.017V5.63c0 .395 0 .736.023 1.017c.024.297.078.592.222.875c.216.424.56.768.984.984c.283.144.578.198.875.222c.252.02.552.023.896.023v2.5H8.17c-.535 0-.98 0-1.345.03c-.38.03-.736.098-1.073.27A2.75 2.75 0 0 0 4.55 12.75c-.172.338-.24.694-.27 1.074c-.03.365-.03.81-.03 1.345v.08c-.344 0-.644.002-.896.023a2.3 2.3 0 0 0-.875.222a2.25 2.25 0 0 0-.984.984a2.3 2.3 0 0 0-.222.875c-.023.28-.023.622-.023 1.017v1.258c0 .395 0 .736.023 1.017c.024.297.078.592.222.875c.216.424.56.768.984.984c.283.144.578.198.875.222c.28.023.622.023 1.017.023H5.63c.395 0 .736 0 1.017-.023a2.3 2.3 0 0 0 .875-.222a2.25 2.25 0 0 0 .984-.983c.144-.284.198-.58.222-.876c.023-.28.023-.622.023-1.017V18.37c0-.395 0-.736-.023-1.017a2.3 2.3 0 0 0-.222-.875a2.25 2.25 0 0 0-.984-.984a2.3 2.3 0 0 0-.875-.222c-.252-.02-.552-.023-.896-.023v-.05c0-.572 0-.957.025-1.252c.023-.288.065-.425.111-.516a1.25 1.25 0 0 1 .547-.546c.09-.046.227-.088.514-.111c.296-.024.68-.025 1.253-.025h7.6c.572 0 .957 0 1.252.025c.288.023.425.065.515.111c.236.12.427.311.547.547c.046.09.088.227.111.515c.024.295.025.68.025 1.252v.05c-.344 0-.644.002-.896.023a2.3 2.3 0 0 0-.875.222a2.25 2.25 0 0 0-.984.984a2.3 2.3 0 0 0-.222.875c-.023.28-.023.622-.023 1.017v1.258c0 .395 0 .736.023 1.017c.024.297.078.592.222.875c.216.424.56.768.984.984c.283.144.578.198.875.222c.28.023.622.023 1.017.023h1.258c.395 0 .736 0 1.017-.023a2.3 2.3 0 0 0 .875-.222a2.25 2.25 0 0 0 .984-.983c.144-.284.198-.58.222-.876c.023-.28.023-.622.023-1.017V18.37c0-.395 0-.736-.023-1.017a2.3 2.3 0 0 0-.222-.875a2.25 2.25 0 0 0-.983-.984a2.3 2.3 0 0 0-.876-.222c-.252-.02-.552-.023-.896-.023v-.08c0-.535 0-.98-.03-1.345c-.03-.38-.098-.736-.27-1.073a2.75 2.75 0 0 0-1.201-1.202c-.338-.172-.694-.24-1.074-.27c-.365-.03-.81-.03-1.345-.03h-3.08v-2.5c.344 0 .644-.002.896-.023a2.3 2.3 0 0 0 .875-.222a2.25 2.25 0 0 0 .984-.984a2.3 2.3 0 0 0 .222-.875c.023-.28.023-.622.023-1.017V4.37c0-.395 0-.736-.023-1.017a2.3 2.3 0 0 0-.222-.875a2.25 2.25 0 0 0-.984-.984a2.3 2.3 0 0 0-.875-.222c-.28-.023-.622-.023-1.017-.023m5.771 15.5c-.432 0-.712 0-.924.018c-.204.017-.28.045-.316.064a.75.75 0 0 0-.328.328c-.02.037-.047.112-.064.316c-.017.212-.018.492-.018.924v1.2c0 .432 0 .712.018.924c.017.204.045.28.064.316a.75.75 0 0 0 .328.328c.037.02.112.047.316.064c.212.017.492.018.924.018h1.2c.432 0 .712 0 .924-.018c.204-.017.28-.045.316-.064a.75.75 0 0 0 .328-.328c.02-.037.047-.112.064-.316c.017-.212.018-.492.018-.924v-1.2c0-.432 0-.712-.018-.924c-.017-.204-.045-.28-.064-.316a.75.75 0 0 0-.328-.328c-.037-.02-.112-.047-.316-.064a13 13 0 0 0-.924-.018zm-14 0h1.2c.432 0 .712 0 .924.018c.204.017.28.045.316.064a.75.75 0 0 1 .328.328c.02.037.047.112.064.316c.017.212.018.492.018.924v1.2c0 .432 0 .712-.018.924c-.017.204-.045.28-.064.316a.75.75 0 0 1-.328.328c-.037.02-.112.047-.316.064c-.212.017-.492.018-.924.018H4.4c-.432 0-.712 0-.924-.018c-.204-.017-.28-.045-.316-.064a.75.75 0 0 1-.328-.328c-.02-.037-.047-.112-.064-.316a13 13 0 0 1-.018-.924v-1.2c0-.432 0-.712.018-.924c.017-.204.045-.28.064-.316a.75.75 0 0 1 .328-.328c.037-.02.112-.047.316-.064c.212-.017.492-.018.924-.018m8.2-9.5c.432 0 .712 0 .924-.018c.204-.017.28-.045.317-.064a.75.75 0 0 0 .327-.328c.02-.037.047-.112.064-.316c.017-.212.018-.492.018-.924V4.4c0-.432 0-.712-.018-.924c-.017-.204-.045-.28-.064-.316a.75.75 0 0 0-.327-.328c-.038-.02-.113-.047-.317-.064a13 13 0 0 0-.924-.018h-1.2c-.432 0-.712 0-.924.018c-.204.017-.28.045-.317.064a.75.75 0 0 0-.327.328c-.02.037-.047.112-.064.316c-.017.212-.018.492-.018.924v1.2c0 .432 0 .712.018.924c.017.204.045.28.064.316a.75.75 0 0 0 .327.328c.038.02.113.047.317.064c.212.017.492.018.924.018h1.2"/>"#,
};

pub static DEPLOY: Icon = Icon {
    name: "w-deploy",
    view: 24.0,
    body: r#"<path fill="currentColor" fill-rule="evenodd" d="M17.406 1.25h.761c.872 0 1.6 0 2.18.078c.61.082 1.166.262 1.613.708s.63 1.001.712 1.612c.078.578.078 1.304.078 2.173v.76c0 1.05 0 1.787-.28 2.461s-.802 1.195-1.547 1.937l-.1.1l-2.67 2.662l.093.143c.394.608.713 1.1.943 1.525c.238.44.408.853.476 1.31a4.1 4.1 0 0 1-.28 2.18c-.107.258-.292.527-.482.772c-.2.258-.444.538-.706.823a58 58 0 0 1-1.702 1.746l-.091.09c-.72.718-1.968.486-2.352-.477c-.227-.572-.32-.8-.435-1.01a4 4 0 0 0-.367-.555c-.146-.187-.316-.36-.74-.781l-.505-.443l-.02-.019a2 2 0 0 1-.285.018c-.632 0-1.153-.263-1.644-.637c-.464-.353-.98-.867-1.596-1.481l-1.447-1.443c-.616-.614-1.131-1.128-1.486-1.591c-.374-.49-.64-1.01-.64-1.642q.001-.17.026-.337l-.83-.827l-.087-.087a4.1 4.1 0 0 0-1.41-.895l-.378-.15a1.514 1.514 0 0 1-.513-2.48l.007-.008a59 59 0 0 1 1.751-1.697c.287-.261.568-.504.825-.703c.247-.19.517-.375.774-.481a4.13 4.13 0 0 1 2.184-.278c.457.067.87.236 1.311.474c.426.23.92.547 1.53.94l.148.095l2.672-2.665l.1-.099c.744-.743 1.266-1.264 1.942-1.543s1.413-.278 2.467-.278M9.137 6.919c-.56-.36-.963-.616-1.301-.799c-.36-.193-.6-.278-.82-.31a2.63 2.63 0 0 0-1.39.179a2.4 2.4 0 0 0-.432.284c-.212.163-.459.376-.73.623a57 57 0 0 0-1.71 1.658l-.004.006v.007l.003.008l.008.004l.383.152l.128.051a5.6 5.6 0 0 1 1.768 1.16c.028.026.055.053.098.097l.478.476q.098-.122.204-.242c.322-.368.735-.78 1.214-1.257zm4.317 11.42l.081.071l.052.052c.39.39.633.63.844.9q.281.36.501.76c.16.291.284.598.477 1.083l.02-.02a57 57 0 0 0 1.663-1.705c.248-.27.462-.517.625-.728a2.3 2.3 0 0 0 .285-.43a2.6 2.6 0 0 0 .179-1.383c-.032-.218-.117-.457-.31-.815c-.184-.338-.441-.74-.804-1.3l-2.109 2.102c-.49.488-.912.909-1.287 1.233a8 8 0 0 1-.217.18M17.547 2.75c-1.252 0-1.67.013-2.036.164c-.365.151-.67.436-1.555 1.319l-5.847 5.829c-.498.497-.876.874-1.16 1.2c-.287.325-.433.549-.504.725a.7.7 0 0 0-.058.282c0 .17.059.374.332.73c.285.374.728.817 1.39 1.478l.157.156l1.53-1.526a.75.75 0 0 1 1.06 1.063l-1.527 1.522l.153.154c.663.66 1.107 1.101 1.482 1.386c.358.272.563.331.736.331a.7.7 0 0 0 .26-.048c.175-.064.398-.204.73-.49c.33-.286.716-.67 1.227-1.18l5.847-5.828c.885-.883 1.17-1.186 1.321-1.55s.165-.778.165-2.027v-.567c0-.934-.002-1.56-.065-2.025c-.06-.443-.162-.629-.284-.75s-.308-.224-.754-.284c-.468-.062-1.095-.064-2.032-.064zm-.884 4.574a1.196 1.196 0 0 0-1.687 0a1.18 1.18 0 0 0 0 1.676a1.196 1.196 0 0 0 1.687 0a1.18 1.18 0 0 0 0-1.676M13.917 6.26a2.696 2.696 0 0 1 3.806 0a2.68 2.68 0 0 1 0 3.801a2.696 2.696 0 0 1-3.806 0a2.68 2.68 0 0 1 0-3.8"/>"#,
};

pub fn workflow_icon(name: &str) -> &'static Icon {
    if name.to_ascii_lowercase().contains("deploy") {
        &DEPLOY
    } else {
        &WORKFLOW
    }
}
