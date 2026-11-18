use std::ops::Range;

use rune::compose::{Compositor, Frame};
use rune::render::{EffectPhase, EffectSite, KIND_EFFECT};
use rune_fx::{Fx, FxFrame, FxLayer, FxOptions, FxRenderer, FxSlot, PulseParams, PulseStyle, Rgb, Shape, rgb};

use crate::state::AppState;

pub const MARK_PULSE_BREATHE: u32 = 1;
pub const MARK_PULSE_RADAR: u32 = 2;
pub const MARK_PULSE_HEARTBEAT: u32 = 3;
/// Spinner de execução — família `pulse-*` (fluxos de CI/build/teste).
pub const MARK_SPINNER_PULSE_SOFT: u32 = 4;
pub const MARK_SPINNER_PULSE_MULTIPLE: u32 = 5;
pub const MARK_SPINNER_PULSE_RINGS_2: u32 = 6;
pub const MARK_SPINNER_PULSE_RINGS_3: u32 = 7;
/// Spinner de execução — família `blocks-*` (fluxos de deploy).
pub const MARK_SPINNER_BLOCKS_SCALE: u32 = 8;
pub const MARK_SPINNER_BLOCKS_SHUFFLE_2: u32 = 9;
pub const MARK_SPINNER_BLOCKS_SHUFFLE_3: u32 = 10;
pub const MARK_SPINNER_BLOCKS_WAVE: u32 = 11;

fn color_by_idx(idx: f32) -> Rgb {
    match idx.round() as u32 {
        1 => rgb(0x6bd586), // Emerald / Success (runner conectado)
        2 => rgb(0xf2cf76), // Amber / Warning (aprovação pendente / aviso)
        3 => rgb(0xf17471), // Coral / Danger (falha)
        4 => rgb(0x7698fd), // Blue / Info (executando)
        _ => rgb(0x7698fd), // Accent blue fallback
    }
}

#[derive(Default)]
pub struct WorkployGfx {
    fx: Option<FxRenderer>,
    animating: bool,
}

impl Compositor<AppState> for WorkployGfx {
    fn animating(&self) -> bool {
        self.animating
    }

    fn render(&mut self, f: &mut Frame<'_, AppState>) {
        let (device, queue, size) = (f.device, f.queue, f.size);
        let raw_t = f.rt.t as f32;
        let rm = f.rt.reduced_motion;
        let s = &f.rt.state;
        // Phase-continuous effect time + deterministic non-zero resting offset for frame-0 snaps
        let phase_t = s.fx_phase.at(raw_t) + 0.35;

        let marks = &f.rt.marks;
        let mut slot_ranges: Vec<(u32, Range<usize>)> = Vec::new();
        let mut layers: Vec<FxLayer> = Vec::new();
        let mut placements: Vec<FxSlot> = Vec::new();

        for (n, i) in f.rt.instances.iter().enumerate() {
            if i.params[1] != KIND_EFFECT {
                continue;
            }
            let kind = marks.get(n).copied().unwrap_or(0) & 0xff;
            let k = i.params[3].max(1.0);
            let [x, y, w, h] = i.rect.map(|v| v / k);
            let r = i.radii.iter().fold(0.0f32, |a, v| a.max(v.abs())) / k;
            if w <= 0.0 || h <= 0.0 {
                continue;
            }
            let shape = Shape::rounded(x, y, w, h, r);
            let d = i.uv;
            let slot = FxSlot::default().scale(k).clip(i.clip, i.fade);
            let start = layers.len();

            match kind {
                MARK_PULSE_BREATHE => {
                    let col = color_by_idx(d[0]);
                    let reach = if d[1] > 0.0 { d[1] } else { 8.0 };
                    let period = if d[2] > 0.0 { d[2] } else { 2.6 };
                    layers.push(
                        Fx::pulse(PulseParams {
                            style: PulseStyle::Breathe,
                            color: col,
                            reach,
                            period,
                            intensity: if d[3] > 0.0 { d[3] } else { 0.55 },
                            ..Default::default()
                        })
                        .on(shape)
                        .at(phase_t)
                        .reduced(rm),
                    );
                    placements.push(slot);
                }
                MARK_PULSE_RADAR => {
                    let col = color_by_idx(d[0]);
                    let reach = if d[1] > 0.0 { d[1] } else { 10.0 };
                    let period = if d[2] > 0.0 { d[2] } else { 1.8 };
                    layers.push(
                        Fx::pulse(PulseParams {
                            style: PulseStyle::Radar,
                            color: col,
                            count: 2,
                            reach,
                            period,
                            width: 1.2,
                            intensity: if d[3] > 0.0 { d[3] } else { 0.72 },
                            ..Default::default()
                        })
                        .on(shape)
                        .at(phase_t)
                        .reduced(rm),
                    );
                    placements.push(slot);
                }
                MARK_PULSE_HEARTBEAT => {
                    let col = color_by_idx(d[0]);
                    let reach = if d[1] > 0.0 { d[1] } else { 10.0 };
                    let period = if d[2] > 0.0 { d[2] } else { 2.0 };
                    layers.push(
                        Fx::pulse(PulseParams {
                            style: PulseStyle::Heartbeat,
                            color: col,
                            reach,
                            period,
                            intensity: if d[3] > 0.0 { d[3] } else { 0.65 },
                            ..Default::default()
                        })
                        .on(shape)
                        .at(phase_t)
                        .reduced(rm),
                    );
                    placements.push(slot);
                }
                MARK_SPINNER_PULSE_SOFT => {
                    // `pulse-2`: dois pontos suaves que crescem e somem, com a
                    // mesma defasagem do SVG original (0.6 s em ciclo de 1.2 s).
                    let col = color_by_idx(d[0]);
                    let reach = if d[1] > 0.0 { d[1] } else { 7.0 };
                    let period = if d[2] > 0.0 { d[2] } else { 1.2 };
                    layers.push(
                        Fx::pulse(PulseParams {
                            style: PulseStyle::Radar,
                            color: col,
                            count: 2,
                            reach,
                            width: reach * 0.42,
                            period,
                            intensity: if d[3] > 0.0 { d[3] } else { 0.6 },
                            ..Default::default()
                        })
                        .on(shape)
                        .at(phase_t)
                        .reduced(rm),
                    );
                    placements.push(slot);
                }
                MARK_SPINNER_PULSE_MULTIPLE => {
                    // `pulse-multiple`: três pontos em sequência, cada um
                    // iniciando pouco depois do anterior (stagger = período/3).
                    let col = color_by_idx(d[0]);
                    let reach = if d[1] > 0.0 { d[1] } else { 8.5 };
                    let period = if d[2] > 0.0 { d[2] } else { 1.5 };
                    layers.push(
                        Fx::pulse(PulseParams {
                            style: PulseStyle::Radar,
                            color: col,
                            count: 3,
                            reach,
                            width: reach * 0.34,
                            period,
                            intensity: if d[3] > 0.0 { d[3] } else { 0.66 },
                            ..Default::default()
                        })
                        .on(shape)
                        .at(phase_t)
                        .reduced(rm),
                    );
                    placements.push(slot);
                }
                MARK_SPINNER_PULSE_RINGS_2 => {
                    // `pulse-rings-2`: dois anéis finos que expandem e somem.
                    let col = color_by_idx(d[0]);
                    let reach = if d[1] > 0.0 { d[1] } else { 11.0 };
                    let period = if d[2] > 0.0 { d[2] } else { 1.8 };
                    layers.push(
                        Fx::pulse(PulseParams {
                            style: PulseStyle::Radar,
                            color: col,
                            count: 2,
                            reach,
                            width: 1.1,
                            period,
                            intensity: if d[3] > 0.0 { d[3] } else { 0.72 },
                            ..Default::default()
                        })
                        .on(shape)
                        .at(phase_t)
                        .reduced(rm),
                    );
                    placements.push(slot);
                }
                MARK_SPINNER_PULSE_RINGS_3 => {
                    // `pulse-rings-3`: três anéis acelerando (defasagem 0.4 s).
                    let col = color_by_idx(d[0]);
                    let reach = if d[1] > 0.0 { d[1] } else { 12.0 };
                    let period = if d[2] > 0.0 { d[2] } else { 1.2 };
                    layers.push(
                        Fx::pulse(PulseParams {
                            style: PulseStyle::Radar,
                            color: col,
                            count: 3,
                            reach,
                            width: 1.1,
                            period,
                            intensity: if d[3] > 0.0 { d[3] } else { 0.78 },
                            ..Default::default()
                        })
                        .on(shape)
                        .at(phase_t)
                        .reduced(rm),
                    );
                    placements.push(slot);
                }
                MARK_SPINNER_BLOCKS_SCALE
                | MARK_SPINNER_BLOCKS_SHUFFLE_2
                | MARK_SPINNER_BLOCKS_SHUFFLE_3
                | MARK_SPINNER_BLOCKS_WAVE => {
                    // Família `blocks-*`: grade de blocos em que cada bloco
                    // respira com defasagem própria, recriando o ritmo do SVG.
                    let col = color_by_idx(d[0]);
                    let period = if d[2] > 0.0 { d[2] } else { 0.9 };
                    let inten = if d[3] > 0.0 { d[3] } else { 0.9 };
                    let bs = w.min(h) * 0.30;
                    // (fração x, fração y, defasagem em s) de cada bloco.
                    let cells: &[(f32, f32, f32)] = match kind {
                        // `blocks-scale`: 2x2 subindo em sequência (horário).
                        MARK_SPINNER_BLOCKS_SCALE => &[
                            (0.10, 0.10, 0.0),
                            (0.60, 0.10, 0.15),
                            (0.60, 0.60, 0.30),
                            (0.10, 0.60, 0.45),
                        ],
                        // `blocks-shuffle-2`: diagonais alternando (troca).
                        MARK_SPINNER_BLOCKS_SHUFFLE_2 => {
                            &[(0.10, 0.10, 0.0), (0.60, 0.60, period * 0.5)]
                        }
                        // `blocks-shuffle-3`: três cantos em rodízio.
                        MARK_SPINNER_BLOCKS_SHUFFLE_3 => &[
                            (0.10, 0.10, 0.0),
                            (0.10, 0.60, period * 0.33),
                            (0.60, 0.60, period * 0.67),
                        ],
                        // `blocks-wave`: 2x2 com defasagem diagonal (col+linha).
                        _ => &[
                            (0.10, 0.10, 0.0),
                            (0.60, 0.10, 0.13),
                            (0.10, 0.60, 0.13),
                            (0.60, 0.60, 0.26),
                        ],
                    };
                    // Nota: `placements` alinha 1:1 com `layers` — um slot por
                    // camada (todas compartilham o mesmo rect/clip do spinner).
                    for &(fx, fy, lag) in cells {
                        layers.push(
                            Fx::pulse(PulseParams {
                                style: PulseStyle::Breathe,
                                color: col,
                                reach: bs * 0.9,
                                period,
                                intensity: inten,
                                ..Default::default()
                            })
                            .on(Shape::rounded(
                                x + w * fx,
                                y + h * fy,
                                bs,
                                bs,
                                bs * 0.28,
                            ))
                            .at(phase_t + lag)
                            .reduced(rm),
                        );
                        placements.push(slot);
                    }
                }
                _ => {}
            }

            let end = layers.len();
            if end > start {
                slot_ranges.push((n as u32, start..end));
            }
        }

        self.animating = !rm && layers.iter().any(|l| l.is_animating());

        let fx = self.fx.get_or_insert_with(|| {
            FxRenderer::new(device, f.format, FxOptions { dither: true })
        });
        fx.set_srgb_blend(f.srgb_blend());
        fx.add_format(device, wgpu::TextureFormat::Rgba16Float);
        fx.prepare_with_slots(
            device,
            queue,
            &FxFrame {
                viewport: [size.0, size.1],
                scale: 1.0,
                layers: &layers,
                backdrop: None,
                sources: &[],
            },
            &placements,
        );

        let clear = f.rt.clear_color();
        f.ui.prepare(device, queue, &mut f.rt.text.atlas, &f.rt.instances, size);
        f.ui.draw_with_backdrop(
            f.encoder,
            f.target,
            clear,
            &mut |phase, site: &EffectSite<'_>| {
                let Some((_, range)) = slot_ranges.iter().find(|(idx, _)| *idx == site.index)
                else {
                    return;
                };
                match phase {
                    EffectPhase::Backdrop(enc) => {
                        if let Some(bd) = site.backdrop {
                            for k in range.clone() {
                                fx.blur_backdrop(device, queue, enc, bd, k);
                            }
                        }
                    }
                    EffectPhase::Draw(pass) => {
                        for k in range.clone() {
                            fx.draw_layer_in(pass, site.format, site.origin, k);
                        }
                    }
                }
            },
        );
    }
}
