use std::ops::Range;

use rune::compose::{Compositor, Frame};
use rune::render::{EffectPhase, EffectSite, KIND_EFFECT};
use rune_fx::{
    ElectricParams, ElectricStyle, Fx, FxFrame, FxLayer, FxOptions, FxRenderer, FxSlot,
    PulseParams, PulseStyle, Rgb, Shape, ShimmerParams, rgb,
};
use workploy::model::SpinnerKind;

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
/// Borda elétrica calma (`ElectricStyle::Charged`) para cards em execução ativa.
pub const MARK_ELECTRIC_BORDER: u32 = 12;
/// Shimmer sutil para barras de progresso em andamento.
pub const MARK_SHIMMER_BAR: u32 = 13;

// ——————————————————————————————————————————————————————————————————————
// Ritmo central dos efeitos de pulso — todo tuning de período (s por
// ciclo) e intensidade vive aqui, em um único lugar.
//
// Regra do maestro: batida lenta e legível, nunca intermitência rápida.
// Spinners de fase com ciclo visível de 2,5–4 s; pulsos contínuos de
// status em ≥ 3 s; intensidade baixa o bastante para não ofuscar o
// texto ao lado. Os sítios de UI passam estes valores via `effect_data`
// e os fallbacks dos braços do compositor leem as mesmas constantes.
// ——————————————————————————————————————————————————————————————————————

/// Ciclo (s) dos spinners de fase — família `pulse-*` (CI).
pub const SPINNER_PERIOD_PULSE_SOFT: f32 = 3.2;
pub const SPINNER_PERIOD_PULSE_MULTIPLE: f32 = 3.6;
pub const SPINNER_PERIOD_PULSE_RINGS_2: f32 = 4.0;
pub const SPINNER_PERIOD_PULSE_RINGS_3: f32 = 4.0;
/// Ciclo (s) dos spinners de fase — família `blocks-*` (deploy).
pub const SPINNER_PERIOD_BLOCKS_SCALE: f32 = 2.8;
pub const SPINNER_PERIOD_BLOCKS_SHUFFLE_2: f32 = 2.9;
pub const SPINNER_PERIOD_BLOCKS_SHUFFLE_3: f32 = 3.0;
pub const SPINNER_PERIOD_BLOCKS_WAVE: f32 = 3.0;
/// Intensidade do spinner na fase inicial; cresce com a fase até o máximo.
pub const SPINNER_INTENSITY_MIN: f32 = 0.34;
/// Intensidade máxima do spinner: presença firme sem ofuscar o texto.
pub const SPINNER_INTENSITY_MAX: f32 = 0.50;
/// Batida calma (s) dos pulsos contínuos de status (conectado, saudável,
/// aprovação pendente).
pub const PULSE_TEMPO_CALM: f32 = 3.4;
/// Batida ativa (s) dos pulsos contínuos (execução, inspeção, aviso).
pub const PULSE_TEMPO_ACTIVE: f32 = 3.0;
/// Intensidade discreta para estados calmos.
pub const PULSE_INTENSITY_CALM: f32 = 0.38;
/// Intensidade para trabalho ativo.
pub const PULSE_INTENSITY_ACTIVE: f32 = 0.42;
/// Radar de alerta (conexão com problema): um ponto mais visível.
pub const PULSE_INTENSITY_ALERT: f32 = 0.46;

/// Período (s) de um ciclo do spinner de fase — 2,5–4 s, suave.
pub fn spinner_period(kind: SpinnerKind) -> f32 {
    use SpinnerKind as K;
    match kind {
        K::PulseSoft => SPINNER_PERIOD_PULSE_SOFT,
        K::PulseMultiple => SPINNER_PERIOD_PULSE_MULTIPLE,
        K::PulseRings2 => SPINNER_PERIOD_PULSE_RINGS_2,
        K::PulseRings3 => SPINNER_PERIOD_PULSE_RINGS_3,
        K::BlocksScale => SPINNER_PERIOD_BLOCKS_SCALE,
        K::BlocksShuffle2 => SPINNER_PERIOD_BLOCKS_SHUFFLE_2,
        K::BlocksShuffle3 => SPINNER_PERIOD_BLOCKS_SHUFFLE_3,
        K::BlocksWave => SPINNER_PERIOD_BLOCKS_WAVE,
    }
}

/// Intensidade do spinner: progride com a fase da execução (iniciando →
/// finalizando) sem passar de [`SPINNER_INTENSITY_MAX`].
pub fn spinner_intensity(kind: SpinnerKind) -> f32 {
    use SpinnerKind as K;
    let step = match kind {
        K::PulseSoft | K::BlocksScale => 0.0,
        K::PulseMultiple | K::BlocksShuffle2 => 0.05,
        K::PulseRings2 | K::BlocksShuffle3 => 0.11,
        K::PulseRings3 | K::BlocksWave => 0.16,
    };
    (SPINNER_INTENSITY_MIN + step).min(SPINNER_INTENSITY_MAX)
}

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
                    let reach = if d[1] > 0.0 { d[1] } else { 5.0 };
                    let period = if d[2] > 0.0 { d[2] } else { PULSE_TEMPO_CALM };
                    layers.push(
                        Fx::pulse(PulseParams {
                            style: PulseStyle::Breathe,
                            color: col,
                            reach,
                            period,
                            intensity: if d[3] > 0.0 { d[3] } else { PULSE_INTENSITY_CALM },
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
                    let reach = if d[1] > 0.0 { d[1] } else { 6.0 };
                    let period = if d[2] > 0.0 { d[2] } else { PULSE_TEMPO_ACTIVE };
                    layers.push(
                        Fx::pulse(PulseParams {
                            style: PulseStyle::Radar,
                            color: col,
                            count: 2,
                            reach,
                            period,
                            width: 0.9,
                            intensity: if d[3] > 0.0 { d[3] } else { PULSE_INTENSITY_ACTIVE },
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
                    let reach = if d[1] > 0.0 { d[1] } else { 6.0 };
                    let period = if d[2] > 0.0 { d[2] } else { PULSE_TEMPO_ACTIVE };
                    layers.push(
                        Fx::pulse(PulseParams {
                            style: PulseStyle::Heartbeat,
                            color: col,
                            reach,
                            period,
                            intensity: if d[3] > 0.0 { d[3] } else { PULSE_INTENSITY_CALM },
                            ..Default::default()
                        })
                        .on(shape)
                        .at(phase_t)
                        .reduced(rm),
                    );
                    placements.push(slot);
                }
                MARK_SPINNER_PULSE_SOFT => {
                    let col = color_by_idx(d[0]);
                    let reach = if d[1] > 0.0 { d[1] } else { 5.0 };
                    let period = if d[2] > 0.0 { d[2] } else { spinner_period(SpinnerKind::PulseSoft) };
                    layers.push(
                        Fx::pulse(PulseParams {
                            style: PulseStyle::Radar,
                            color: col,
                            count: 2,
                            reach,
                            width: 0.9,
                            period,
                            intensity: if d[3] > 0.0 {
                                d[3]
                            } else {
                                spinner_intensity(SpinnerKind::PulseSoft)
                            },
                            ..Default::default()
                        })
                        .on(shape)
                        .at(phase_t)
                        .reduced(rm),
                    );
                    placements.push(slot);
                }
                MARK_SPINNER_PULSE_MULTIPLE => {
                    let col = color_by_idx(d[0]);
                    let reach = if d[1] > 0.0 { d[1] } else { 6.0 };
                    let period = if d[2] > 0.0 {
                        d[2]
                    } else {
                        spinner_period(SpinnerKind::PulseMultiple)
                    };
                    layers.push(
                        Fx::pulse(PulseParams {
                            style: PulseStyle::Radar,
                            color: col,
                            count: 3,
                            reach,
                            width: 0.95,
                            period,
                            intensity: if d[3] > 0.0 {
                                d[3]
                            } else {
                                spinner_intensity(SpinnerKind::PulseMultiple)
                            },
                            ..Default::default()
                        })
                        .on(shape)
                        .at(phase_t)
                        .reduced(rm),
                    );
                    placements.push(slot);
                }
                MARK_SPINNER_PULSE_RINGS_2 => {
                    let col = color_by_idx(d[0]);
                    let reach = if d[1] > 0.0 { d[1] } else { 7.5 };
                    let period = if d[2] > 0.0 {
                        d[2]
                    } else {
                        spinner_period(SpinnerKind::PulseRings2)
                    };
                    layers.push(
                        Fx::pulse(PulseParams {
                            style: PulseStyle::Radar,
                            color: col,
                            count: 2,
                            reach,
                            width: 1.0,
                            period,
                            intensity: if d[3] > 0.0 {
                                d[3]
                            } else {
                                spinner_intensity(SpinnerKind::PulseRings2)
                            },
                            ..Default::default()
                        })
                        .on(shape)
                        .at(phase_t)
                        .reduced(rm),
                    );
                    placements.push(slot);
                }
                MARK_SPINNER_PULSE_RINGS_3 => {
                    let col = color_by_idx(d[0]);
                    let reach = if d[1] > 0.0 { d[1] } else { 8.5 };
                    let period = if d[2] > 0.0 {
                        d[2]
                    } else {
                        spinner_period(SpinnerKind::PulseRings3)
                    };
                    layers.push(
                        Fx::pulse(PulseParams {
                            style: PulseStyle::Radar,
                            color: col,
                            count: 3,
                            reach,
                            width: 1.0,
                            period,
                            intensity: if d[3] > 0.0 {
                                d[3]
                            } else {
                                spinner_intensity(SpinnerKind::PulseRings3)
                            },
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
                    let sk = match kind {
                        MARK_SPINNER_BLOCKS_SCALE => SpinnerKind::BlocksScale,
                        MARK_SPINNER_BLOCKS_SHUFFLE_2 => SpinnerKind::BlocksShuffle2,
                        MARK_SPINNER_BLOCKS_SHUFFLE_3 => SpinnerKind::BlocksShuffle3,
                        _ => SpinnerKind::BlocksWave,
                    };
                    let col = color_by_idx(d[0]);
                    let period = if d[2] > 0.0 { d[2] } else { spinner_period(sk) };
                    let inten = if d[3] > 0.0 { d[3] } else { spinner_intensity(sk) };
                    let bs = w.min(h) * 0.30;
                    // (fração x, fração y, defasagem como fração do período)
                    // de cada bloco: alongar o ciclo preserva a sequência.
                    let cells: &[(f32, f32, f32)] = match kind {
                        // `blocks-scale`: 2x2 subindo em sequência (horário).
                        MARK_SPINNER_BLOCKS_SCALE => &[
                            (0.10, 0.10, 0.0),
                            (0.60, 0.10, 0.20),
                            (0.60, 0.60, 0.40),
                            (0.10, 0.60, 0.60),
                        ],
                        // `blocks-shuffle-2`: diagonais alternando (troca).
                        MARK_SPINNER_BLOCKS_SHUFFLE_2 => {
                            &[(0.10, 0.10, 0.0), (0.60, 0.60, 0.5)]
                        }
                        // `blocks-shuffle-3`: três cantos em rodízio.
                        MARK_SPINNER_BLOCKS_SHUFFLE_3 => &[
                            (0.10, 0.10, 0.0),
                            (0.10, 0.60, 0.33),
                            (0.60, 0.60, 0.67),
                        ],
                        // `blocks-wave`: 2x2 com defasagem diagonal (col+linha).
                        _ => &[
                            (0.10, 0.10, 0.0),
                            (0.60, 0.10, 0.15),
                            (0.10, 0.60, 0.15),
                            (0.60, 0.60, 0.30),
                        ],
                    };
                    // Nota: `placements` alinha 1:1 com `layers` — um slot por
                    // camada (todas compartilham o mesmo rect/clip do spinner).
                    for &(fx, fy, lag) in cells {
                        layers.push(
                            Fx::pulse(PulseParams {
                                style: PulseStyle::Breathe,
                                color: col,
                                reach: bs * 0.45,
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
                            .at(phase_t + lag * period)
                            .reduced(rm),
                        );
                        placements.push(slot);
                    }
                }
                MARK_ELECTRIC_BORDER => {
                    let col = color_by_idx(d[0]);
                    let intensity = if d[3] > 0.0 { d[3] } else { 0.34 };
                    layers.push(
                        Fx::electric(ElectricParams {
                            style: ElectricStyle::Charged,
                            color: col,
                            core: rgb(0xeaf3ff),
                            cool: col,
                            amplitude: 0.65,
                            frequency: 2.2,
                            speed: 1.4,
                            core_width: 0.45,
                            intensity,
                            glow: 1.8,
                            forks: 0.08,
                            flicker: 0.0,
                            strands: 1,
                            gap: 0.5,
                            fork_len: 7.0,
                            occlusion: 0.14,
                            zap: None,
                        })
                        .on(shape)
                        .at(phase_t)
                        .reduced(rm),
                    );
                    placements.push(slot);
                }
                MARK_SHIMMER_BAR => {
                    let col = color_by_idx(d[0]);
                    let period = if d[2] > 0.0 { d[2] } else { 2.4 };
                    let intensity = if d[3] > 0.0 { d[3] } else { 0.38 };
                    layers.push(
                        Fx::shimmer(ShimmerParams {
                            color: rgb(0xffffff),
                            base: col,
                            base_alpha: 0.0,
                            period,
                            band: 32.0,
                            angle: 0.25,
                            intensity,
                            use_mask: false,
                            pause: 0.45,
                            feather: 2.0,
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
