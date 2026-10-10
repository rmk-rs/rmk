use super::Rgb8;

/// One opaque effect sample and the next instant its visible value can
/// change. `None` means the sample is static until an external event.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct EffectSample<C> {
    pub color: C,
    pub next_change_ms: Option<u64>,
}

/// Extensibility boundary for effects supplied by board or third-party
/// crates. The compositor owns ordering and scheduling; the effect owns only
/// its waveform.
pub trait LightingEffect<C> {
    fn sample(&self, now_ms: u64) -> EffectSample<C>;
}

/// Small built-in set sufficient for static scenes and common indicators.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum BuiltinEffect {
    Solid {
        color: Rgb8,
    },
    Blink {
        color: Rgb8,
        period_ms: u32,
        phase_ms: u32,
        duty: u8,
    },
    Breathe {
        color: Rgb8,
        period_ms: u32,
        phase_ms: u32,
        step_ms: u16,
    },
}

impl Default for BuiltinEffect {
    fn default() -> Self {
        Self::solid(Rgb8::BLACK)
    }
}

impl BuiltinEffect {
    pub const fn solid(color: Rgb8) -> Self {
        Self::Solid { color }
    }
}

impl LightingEffect<Rgb8> for BuiltinEffect {
    fn sample(&self, now_ms: u64) -> EffectSample<Rgb8> {
        match *self {
            Self::Solid { color } => EffectSample {
                color,
                next_change_ms: None,
            },
            Self::Blink {
                color,
                period_ms,
                phase_ms,
                duty,
            } => {
                if color == Rgb8::BLACK {
                    return EffectSample {
                        color,
                        next_change_ms: None,
                    };
                }
                if period_ms == 0 || duty >= 100 {
                    return EffectSample {
                        color,
                        next_change_ms: None,
                    };
                }
                if duty == 0 {
                    return EffectSample {
                        color: Rgb8::BLACK,
                        next_change_ms: None,
                    };
                }
                let local = phase_local(now_ms, period_ms, phase_ms);
                let on_ms = ((period_ms as u64 * duty as u64) / 100) as u32;
                if on_ms == 0 {
                    return EffectSample {
                        color: Rgb8::BLACK,
                        next_change_ms: None,
                    };
                }
                let (shown, delta) = if local < on_ms {
                    (color, on_ms - local)
                } else {
                    (Rgb8::BLACK, period_ms - local)
                };
                EffectSample {
                    color: shown,
                    next_change_ms: now_ms.checked_add(delta as u64),
                }
            }
            Self::Breathe {
                color,
                period_ms,
                phase_ms,
                step_ms,
            } => {
                if period_ms < 2 || step_ms == 0 || color == Rgb8::BLACK {
                    return EffectSample {
                        color,
                        next_change_ms: None,
                    };
                }
                let local = phase_local(now_ms, period_ms, phase_ms) as u64;
                let period = period_ms as u64;
                let step = step_ms as u64;
                if step >= period {
                    return EffectSample {
                        color: Rgb8::BLACK,
                        next_change_ms: None,
                    };
                }
                // Quantize both sampling and scheduling to phase-local step
                // boundaries. An unrelated event between boundaries can
                // therefore re-render without changing the advertised value.
                let sampled_local = local / step * step;
                let half = period / 2;
                let level = if sampled_local < half {
                    sampled_local * 255 / half
                } else {
                    (period - sampled_local) * 255 / (period - half)
                } as u8;
                let delta = (step - local % step).min(period - local);
                EffectSample {
                    color: color.scale(level),
                    next_change_ms: now_ms.checked_add(delta),
                }
            }
        }
    }
}

fn phase_local(now_ms: u64, period_ms: u32, phase_ms: u32) -> u32 {
    let period = period_ms as u64;
    ((now_ms % period + phase_ms as u64 % period) % period) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blink_wakes_on_exact_edges_and_dark_is_opaque() {
        let effect = BuiltinEffect::Blink {
            color: Rgb8::new(10, 20, 30),
            period_ms: 100,
            phase_ms: 0,
            duty: 25,
        };
        assert_eq!(effect.sample(24).next_change_ms, Some(25));
        assert_eq!(effect.sample(25).color, Rgb8::BLACK);
        assert_eq!(effect.sample(25).next_change_ms, Some(100));
    }

    #[test]
    fn phase_math_does_not_overflow_at_u64_max() {
        let effect = BuiltinEffect::Blink {
            color: Rgb8::new(1, 2, 3),
            period_ms: 100,
            phase_ms: 7,
            duty: 50,
        };
        assert_eq!(effect.sample(u64::MAX).next_change_ms, None);
    }

    #[test]
    fn breathe_is_constant_between_its_reported_phase_local_boundaries() {
        let effect = BuiltinEffect::Breathe {
            color: Rgb8::new(200, 100, 50),
            period_ms: 100,
            phase_ms: 7,
            step_ms: 10,
        };
        let first = effect.sample(3); // phase-local 10: exactly on a boundary
        assert_eq!(first.next_change_ms, Some(13));
        for now in 4..13 {
            assert_eq!(effect.sample(now).color, first.color);
            assert_eq!(effect.sample(now).next_change_ms, Some(13));
        }
        assert_ne!(effect.sample(13).color, first.color);
    }

    #[test]
    fn degenerate_effects_are_static() {
        let color = Rgb8::new(1, 2, 3);
        let effects = [
            BuiltinEffect::Blink {
                color,
                period_ms: 0,
                phase_ms: 0,
                duty: 50,
            },
            BuiltinEffect::Blink {
                color,
                period_ms: 10,
                phase_ms: 0,
                duty: 0,
            },
            BuiltinEffect::Blink {
                color,
                period_ms: 10,
                phase_ms: 0,
                duty: 1,
            },
            BuiltinEffect::Blink {
                color,
                period_ms: 10,
                phase_ms: 0,
                duty: 100,
            },
            BuiltinEffect::Breathe {
                color,
                period_ms: 1,
                phase_ms: 0,
                step_ms: 16,
            },
            BuiltinEffect::Breathe {
                color,
                period_ms: 10,
                phase_ms: 0,
                step_ms: 0,
            },
            BuiltinEffect::Blink {
                color: Rgb8::BLACK,
                period_ms: 10,
                phase_ms: 0,
                duty: 50,
            },
            BuiltinEffect::Breathe {
                color: Rgb8::BLACK,
                period_ms: 10,
                phase_ms: 0,
                step_ms: 1,
            },
        ];
        for effect in effects {
            assert_eq!(effect.sample(5).next_change_ms, None);
        }
    }
}
