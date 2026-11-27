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

// ——————————————————————————————————————————————————————————————————————
// Ritmo dos efeitos de pulso (o custo de GPU com o app parado).
//
// Um efeito que o compositor desenha deixa a janela no display link
// enquanto alguma camada se declara animada: um frame por vsync (60 Hz),
// com o usuário parado, durante toda a vida do efeito. Pulso e spinner de
// fase são ciclos lentos (2,8 a 4 s), e 1/15 s de passo não se distingue
// neles; já o shimmer de uma barra de progresso acompanha trabalho ao vivo
// e fica em ritmo cheio.
//
// Contrato (`effect_data` de um efeito de pulso):
// - `d[3]`, a intensidade, leva o bit [`CALM_BIT`] quando o efeito é
//   decorativo. A intensidade real fica em 0.2–0.6, então o bit nunca colide
//   com ela; `calm_intensity` liga e `intensity_of` desliga. Um sítio que não
//   conhece o bit (intensidade pura) continua exatamente como antes, em 60 Hz;
// - raio (`d[1]`), período (`d[2]`) e cor (`d[0]`) não mudam de significado.
//
// O compositor decide por frame ([`FxClock`]): um efeito decorativo não o
// deixa em `animating()`, e sim pede o próximo tick ao app por
// [`TickPlan`] (o `render` não tem `Tx`; o `poll` de `state.rs` converte o
// pedido em `wake_after`). Spinner, shimmer e borda elétrica são movimento
// real e seguem o display link. A fase compartilhada (`fx_phase`) é amostrada
// uma vez por frame: todas as camadas decorativas leem a mesma amostra e
// avançam juntas.
// ——————————————————————————————————————————————————————————————————————

/// Taxa (Hz) dos efeitos decorativos calmos.
pub const FX_FPS_CALM: f32 = 15.0;
/// Bit de "decorativo calmo" no campo da intensidade (`d[3]`). A intensidade
/// real de um pulso fica em 0.2–0.6, então o bit nunca colide com ela; ver
/// [`intensity_of`] e [`calm_intensity`].
pub const CALM_BIT: f32 = 0.5;

/// Intensidade real de um pulso, tirando o bit de calmo.
pub fn intensity_of(v: f32) -> f32 {
    (v - CALM_BIT).max(0.0)
}

/// `d[3]` de um pulso decorativo: a intensidade com o bit de calmo ligado.
pub fn calm_intensity(intensity: f32) -> f32 {
    intensity + CALM_BIT
}

/// Ritmo de uma camada de pulso: o que ela pede de frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectRate {
    /// 60 Hz (o ritmo de sempre).
    Full,
    /// [`FX_FPS_CALM`].
    Calm,
}

impl EffectRate {
    /// Ritmo pedido por um pulso, a partir do campo da intensidade.
    pub fn of(intensity: f32) -> Self {
        if intensity >= CALM_BIT { Self::Calm } else { Self::Full }
    }

    /// Fps deste ritmo (0 = display link).
    pub fn fps(self) -> f32 {
        match self {
            Self::Full => 0.0,
            Self::Calm => FX_FPS_CALM,
        }
    }
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
    clock: FxClock,
}

/// Se a janela está desenhando e com foco, para o ritmo dos efeitos.
///
/// O Rune não expõe visibilidade nem foco ao app, então o compositor usa o
/// que ele mesmo vê: `Frame::rt` traz `window_resting` e a própria existência
/// de um frame já prova que a janela desenha. O congelamento por foco é feito
/// pelo próprio Rune (minimizada, ele corta `loop_wake` e não recorta `animating`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FxView {
    /// `false` com a janela minimizada ou coberta: nada desenhado pede frame.
    pub visible: bool,
}

impl FxView {
    /// O retrato da janela deste frame: um frame que chegou aqui está visível.
    pub fn drawing() -> Self {
        Self { visible: true }
    }

    /// Janela que não desenha (minimizada ou coberta): nada pede frame.
    #[cfg(test)]
    pub fn hidden() -> Self {
        Self { visible: false }
    }
}

/// Decide a cadência dos efeitos decorativos e quando a janela pede frame.
///
/// O estado é pequeno de propósito: o instante do último frame, o instante em
/// que o próximo tick fica pronto, que fase os frames atuais desenham e o que
/// o último frame pediu. Nada aqui agenda timers: quem pede o próximo frame é
/// o app (`state::poll_app`), porque só ele tem `Tx`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FxClock {
    last: Option<f64>,
    /// Instante em que a fase decorativa pode avançar de novo.
    next: Option<f64>,
    phase: Option<f32>,
    /// Maior fps pedido por uma camada decorativa no último frame.
    fps: f32,
    view: FxView,
    /// Tem movimento de camada que não é decorativo (spinner, shimmer,
    /// eléctrico): o loop segue o display link e nada é agendado pelo app.
    class_motion: bool,
    /// O último frame só desenhou camadas cheias (execução ativa, sem nada
    /// decorativo a puxar o ritmo): o app não precisa agendar tick.
    full_only: bool,
}

impl FxClock {
    /// Período (s) entre ticks para `fps`.
    pub fn period(fps: f32) -> f64 {
        if fps.is_finite() && fps > 0.0 { 1.0 / f64::from(fps) } else { 0.0 }
    }

    /// Depois de desenhar um frame: registra o instante e prepara o próximo
    /// tick. Retorna se o compositor deve pedir frames (loop em `Poll`).
    pub fn frame(&mut self, now: f64, view: FxView, fps: f32, class_motion: bool, animated: bool) -> bool {
        self.last = Some(now);
        self.view = view;
        self.fps = if fps.is_finite() && fps > 0.0 { fps } else { 0.0 };
        self.class_motion = class_motion;
        self.full_only = animated && !class_motion && self.fps <= 0.0;
        self.next = if self.fps > 0.0 && view.visible {
            Some(now + Self::period(self.fps))
        } else {
            None
        };
        self.wants_frames()
    }

    /// A fase compartilhada que este frame deve desenhar. Avança só quando o
    /// frame chega no instante preparado (ou sem tick preparado): um frame
    /// antecipado, de input ou commit, redesenha a mesma amostra.
    pub fn phase(&mut self, now: f64, from: impl FnOnce(f64) -> f32) -> f32 {
        if let (Some(next), Some(p)) = (self.next, self.phase)
            && now < next
        {
            return p;
        }
        let p = from(now);
        self.phase = Some(p);
        p
    }

    /// O compositor precisa pedir frames agora (o loop fica em `Poll`).
    ///
    /// Com tick decorativo em marcha, o compositor só se declara animado se o
    /// frame chegou depois do instante preparado: um frame antecipado (input,
    /// commit) deixa a decisão para o app, que agenda o tick por `wake_after`.
    pub fn wants_frames(&self) -> bool {
        if self.class_motion && self.view.visible {
            return true;
        }
        let Some(next) = self.next else { return false };
        self.view.visible && self.last > Some(next)
    }

    /// Reduced motion: os efeitos estão congelados e nada pede frame. A fase
    /// fica guardada para retomar de onde parou quando as animações voltarem.
    pub fn forget(&mut self) {
        self.next = None;
        self.fps = 0.0;
        self.class_motion = false;
        self.full_only = false;
    }

    /// Movimento de camada que não é decorativo (spinner, shimmer, eléctrico):
    /// o display link conduz, então não há tick decorativo a preparar.
    pub fn mark_raw(&mut self, now: f64, view: FxView) {
        self.last = Some(now);
        self.view = view;
        self.next = None;
        self.fps = 0.0;
        self.class_motion = false;
        self.full_only = false;
    }

    /// Quanto tempo falta para o próximo tick, para o app agendar o frame.
    /// `None` quando nada deve ser agendado: sem decorativo em marcha, com
    /// movimento de classe (o display link conduz) ou sem desenho.
    pub fn wake_after(&self, now: f64) -> Option<f64> {
        if self.class_motion || !self.view.visible {
            return None;
        }
        let next = self.next?;
        if self.fps <= 0.0 {
            return None;
        }
        Some((next - now).max(0.0))
    }
}

/// O pedido que o compositor deixa para o app: quando desenhar o próximo
/// tick dos efeitos decorativos. O `render` roda com o `Frame`, que não tem
/// `Tx`; o `poll` do app tem, e é ele quem agenda o frame.
///
/// O plano expira: um pedido antigo nunca é consumido duas vezes, e um
/// pedido que não foi consumido no próximo frame é descartado. Nada aqui
/// pode virar um timer que se reagenda para sempre.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TickPlan {
    /// Quando o pedido foi emitido (`clock` do Rune).
    at: Option<f64>,
    /// Segundos até o tick.
    wait: f64,
}

impl TickPlan {
    /// O que o compositor desenhou neste frame: `wait` segundos até o
    /// próximo tick, ou `None` quando nada decorativo pede frame.
    pub fn sync(&mut self, wait: Option<f64>, now: f64) {
        match wait {
            Some(w) if w.is_finite() => {
                self.at = Some(now);
                self.wait = w.clamp(0.0, 1.0);
            }
            _ => self.clear(),
        }
    }

    /// O compositor não quer tick (nada decorativo, display link conduzindo,
    /// janela minimizada ou sem foco).
    pub fn clear(&mut self) {
        self.at = None;
        self.wait = 0.0;
    }

    /// Há um tick pedido.
    #[cfg(test)]
    pub fn pending(&self) -> bool {
        self.at.is_some()
    }

    /// Tira o pedido vivo, se ele ainda for válido: um pedido que ficou mais
    /// tempo do que o próprio tick (por exemplo, se o poll não rodou) é
    /// descartado, e o próximo frame reavalia.
    pub fn take(&mut self, now: f64) -> Option<std::time::Duration> {
        let at = self.at?;
        let wait = self.wait;
        self.clear();
        if now - at > wait + 0.05 {
            None
        } else {
            Some(std::time::Duration::from_secs_f64(wait))
        }
    }
}

impl Compositor<AppState> for WorkployGfx {
    fn animating(&self) -> bool {
        self.animating
    }

    fn render(&mut self, f: &mut Frame<'_, AppState>) {
        let (device, queue, size) = (f.device, f.queue, f.size);
        let now = f.rt.clock;
        let raw_t = f.rt.t as f32;
        let rm = f.rt.reduced_motion;
        let s = &f.rt.state;
        // Phase-continuous effect time + deterministic non-zero resting offset for frame-0 snaps
        let phase_t = s.fx_phase.at(raw_t) + 0.35;
        // Fase dos efeitos decorativos: uma amostra por frame, compartilhada por
        // todas as camadas (ver `FxClock::phase`). Ela só avança no instante
        // armado, então um frame antecipado de input redesenha a mesma pose.
        let calm_phase = self.clock.phase(now, |t| s.fx_phase.at(t as f32) + 0.35);
        // Maior taxa pedida por um efeito decorativo neste frame (0 = nenhum).
        let mut calm_fps = 0.0f32;
        // Fase que um pulso desenha com o ritmo `rate`.
        let pulse_t = |rate: EffectRate| -> f32 {
            match rate {
                EffectRate::Calm => calm_phase,
                EffectRate::Full => phase_t,
            }
        };
        // Movimento que não é decorativo e que não passa pelo controle de
        // foco: spinners de fase, shimmer de barra e borda elétrica seguem o
        // display link enquanto existem (são raros e mostram trabalho real).
        let mut class_motion = false;

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
            // Ritmo pedido por este efeito: o bit de calmo no campo da intensidade
            // (ver `EffectRate` e `calm_intensity`).
            let rate = EffectRate::of(d[3]);

            match kind {
                MARK_PULSE_BREATHE | MARK_PULSE_RADAR | MARK_PULSE_HEARTBEAT => {
                    // Pulso de status: raio, período e cor como sempre; só o
                    // ritmo muda, pelo bit de calmo.
                    let style = match kind {
                        MARK_PULSE_BREATHE => PulseStyle::Breathe,
                        MARK_PULSE_RADAR => PulseStyle::Radar,
                        _ => PulseStyle::Heartbeat,
                    };
                    let reach = match d[1] {
                        r if r > 0.0 => r,
                        _ if kind == MARK_PULSE_BREATHE => 5.0,
                        _ => 6.0,
                    };
                    let period = match d[2] {
                        p if p > 0.0 => p,
                        _ if kind == MARK_PULSE_BREATHE => PULSE_TEMPO_CALM,
                        _ => PULSE_TEMPO_ACTIVE,
                    };
                    let fallback = if kind == MARK_PULSE_RADAR {
                        PULSE_INTENSITY_ACTIVE
                    } else {
                        PULSE_INTENSITY_CALM
                    };
                    let intensity = {
                        let v = intensity_of(d[3]);
                        if v > 0.0 { v } else { fallback }
                    };
                    layers.push(
                        Fx::pulse(PulseParams {
                            style,
                            color: color_by_idx(d[0]),
                            count: if style == PulseStyle::Radar { 2 } else { 1 },
                            width: if style == PulseStyle::Radar { 0.9 } else { 1.0 },
                            reach,
                            period,
                            intensity,
                            ..Default::default()
                        })
                        .on(shape)
                        .at(pulse_t(rate))
                        .reduced(rm),
                    );
                    placements.push(slot);
                    if rate == EffectRate::Calm {
                        calm_fps = calm_fps.max(rate.fps());
                    }
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
                    class_motion = true;
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
                    class_motion = true;
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
                    class_motion = true;
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

        // A partir daqui só o que o compositor sabe: um frame que chegou aqui
        // é uma janela desenhando. O Rune congela `loop_wake` quando a janela
        // minimiza, então não há frame para desenhar com ela escondida.
        let view = FxView::drawing();
        let layer_motion = layers.iter().any(|l| l.is_animating());
        if rm {
            self.clock.forget();
            self.animating = false;
        } else if layer_motion && class_motion {
            self.clock.mark_raw(now, view);
            self.animating = true;
        } else {
            self.animating = self.clock.frame(now, view, calm_fps, class_motion, layer_motion);
        }
        // O pedido volta ao estado do app (o `poll` o converte em
        // `wake_after`): o compositor não tem `Tx`, e manter `animating`
        // verdadeiro aqui seria voltar ao loop de 60 Hz que este ritmo
        // existe para tirar.
        let next = if self.animating { None } else { self.clock.wake_after(now) };
        f.rt.state.fx_tick.sync(next, now);

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

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> FxView {
        FxView::drawing()
    }

    #[test]
    fn a_decorative_effect_never_holds_the_window_on_the_display_link() {
        let mut clock = FxClock::default();
        // Um pulso decorativo calmo numa janela que desenha.
        assert!(
            !clock.frame(1.0, on(), FX_FPS_CALM, false, true),
            "um efeito decorativo não pode manter a janela em 60 Hz"
        );
        let wait = clock.wake_after(1.0).expect("pede o próximo tick");
        assert!((wait - 1.0 / 15.0).abs() < 1e-9, "tick em 15 Hz, veio {wait}");
    }

    #[test]
    fn a_minimized_window_asks_for_no_frame() {
        let mut clock = FxClock::default();
        assert!(!clock.frame(1.0, FxView::hidden(), FX_FPS_CALM, false, true));
        assert_eq!(clock.wake_after(1.0), None, "minimizado, zero frames");
    }

    #[test]
    fn a_full_rate_effect_inside_a_hidden_window_asks_for_nothing() {
        let mut clock = FxClock::default();
        assert!(!clock.frame(1.0, FxView::hidden(), 0.0, true, true));
        assert_eq!(clock.wake_after(1.0), None);
    }

    #[test]
    fn live_motion_keeps_the_full_rate() {
        let mut clock = FxClock::default();
        // Spinner/shimmer: movimento real, display link.
        assert!(clock.frame(1.0, on(), 0.0, true, true));
        assert_eq!(clock.wake_after(1.0), None, "o display link conduz");
    }

    #[test]
    fn the_calm_phase_holds_between_ticks_and_advances_on_the_tick() {
        let mut clock = FxClock::default();
        clock.frame(0.0, on(), FX_FPS_CALM, false, true);
        assert_eq!(clock.phase(0.0, |t| t as f32), 0.0);
        assert_eq!(clock.phase(0.03, |t| t as f32), 0.0, "um frame antecipado não avança a fase");
        assert_eq!(clock.phase(0.07, |t| t as f32), 0.07, "o tick avança a fase");
    }

    #[test]
    fn a_tick_plan_is_consumed_once_and_never_renews_itself() {
        let mut plan = TickPlan::default();
        plan.sync(Some(1.0 / 15.0), 10.0);
        assert!(plan.pending());
        let first = plan.take(10.0).expect("primeiro pedido");
        assert!((first.as_secs_f64() - 1.0 / 15.0).abs() < 1e-9);
        assert!(!plan.pending(), "consumido: não sobra um timer se reagendando");
        assert_eq!(plan.take(10.001), None);
    }

    #[test]
    fn a_stale_tick_plan_is_dropped_instead_of_firing_late() {
        let mut plan = TickPlan::default();
        plan.sync(Some(0.05), 10.0);
        assert_eq!(plan.take(20.0), None, "pedido velho não dispara fora de hora");
        assert!(!plan.pending());
    }
}
