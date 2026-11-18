use std::ops::Range;

use rune::compose::{Compositor, Frame};
use rune::render::{EffectPhase, EffectSite, KIND_EFFECT};
use rune_fx::{Fx, FxFrame, FxLayer, FxOptions, FxRenderer, FxSlot, PulseParams, PulseStyle, Rgb, Shape, rgb};

use crate::state::AppState;

pub const MARK_PULSE_BREATHE: u32 = 1;
pub const MARK_PULSE_RADAR: u32 = 2;
pub const MARK_PULSE_HEARTBEAT: u32 = 3;

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
