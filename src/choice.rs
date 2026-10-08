//! J0: ChoiceStream — record and replay bot random choices (substrate
//! for J2 choice mutation, J3 Go-Explore, K2 choice shrinking).
//!
//! Bots draw from this stream instead of a raw PRNG. Draws come from
//! `replay` while it lasts (each draw consumes one entry), then from
//! `continuation` (default: PRNG). Every draw is RECORDED with frame,
//! reduced value and bound, so the full decision sequence can be
//! persisted, mutated, shrunk and replayed.
//!
//! Shrink-friendly encoding: stored values are reduced, in `[0,
//! bound)`. When a replayed value is >= the current bound (surface or
//! candidate count changed), it CLAMPS via `value % bound` — never
//! panics, so mutated/shrunk streams are always valid. All-zero
//! choices mean "simplest": first variant, index 0, canonical values.
//!
//! Bias: Lemire's multiply-shift rejects modulo bias on small bounds
//! (top bits of the xorshift64* stream dominate).

use bevy::prelude::*;

/// Continuation semantics once replayed choices run out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Continuation {
    /// Keep drawing from the seeded PRNG (default).
    #[default]
    Prng,
    /// Draw zeros (simplest choices) — used by choice shrinking.
    Zeros,
    /// Reseed the PRNG with a fixed value — used by Go-Explore (J3)
    /// to diverge deterministically after a replayed prefix.
    Reseed(u64),
}

/// One recorded random draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Choice {
    pub frame: u64,
    /// Reduced value in [0, bound).
    pub value: u64,
    /// 0 = raw u64 draw (no reduction).
    pub bound: u64,
}

/// Source of bot randomness (Resource).
#[derive(Resource, Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ChoiceStream {
    /// xorshift64* state (moved out of PlaytestState by J0).
    rng: u64,
    /// Replay prefix: consumed draw-by-draw before the PRNG.
    replay: std::collections::VecDeque<Choice>,
    /// What to draw once replay is exhausted.
    pub continuation: Continuation,
    /// Whether the Reseed continuation has been applied yet.
    reseeded: bool,
    /// Every draw this run (frame, reduced value, bound).
    pub recorded: Vec<Choice>,
}

impl Default for ChoiceStream {
    fn default() -> Self {
        Self::from_seed(42)
    }
}

impl ChoiceStream {
    /// New stream from a scenario seed (seed 0 maps to 42, matching
    /// the historical PlaytestState behaviour).
    pub fn from_seed(seed: u64) -> Self {
        Self {
            rng: if seed == 0 { 42 } else { seed },
            replay: Default::default(),
            continuation: Continuation::Prng,
            reseeded: false,
            recorded: Vec::new(),
        }
    }

    /// Install a replay prefix (J0: `bot.choices`) and continuation.
    pub fn with_replay(mut self, choices: Vec<Choice>, continuation: Continuation) -> Self {
        self.replay = choices.into();
        self.continuation = continuation;
        self
    }

    /// The next raw u64 from the PRNG (xorshift64*, Vigna/Marsaglia
    /// scrambler — same stream PlaytestState used).
    fn next_raw(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    /// Uniform in [0, bound). Records the REDUCED value so shrinking
    /// toward 0 means "simplest choice". Out-of-range replayed
    /// values CLAMP via `% bound` (never panics; mutated streams
    /// stay valid). Lemire multiply-shift kills modulo bias.
    pub fn below(&mut self, frame: u64, bound: u64) -> u64 {
        if bound <= 1 {
            self.recorded.push(Choice {
                frame,
                value: 0,
                bound,
            });
            return 0;
        }
        let value = match self.replay.pop_front() {
            Some(c) => c.value % bound,
            None => match self.continuation {
                Continuation::Prng => {
                    // Lemire: (raw * bound) >> 64 — unbiased, top bits.
                    (((self.next_raw() as u128) * (bound as u128)) >> 64) as u64
                }
                Continuation::Zeros => 0,
                Continuation::Reseed(seed) => {
                    if !self.reseeded {
                        self.rng = if seed == 0 { 42 } else { seed };
                        self.reseeded = true;
                    }
                    (((self.next_raw() as u128) * (bound as u128)) >> 64) as u64
                }
            },
        };
        self.recorded.push(Choice {
            frame,
            value,
            bound,
        });
        value
    }

    /// Bernoulli(num/den) built on `below`.
    pub fn chance(&mut self, frame: u64, num: u64, den: u64) -> bool {
        self.below(frame, den) < num
    }

    /// Uniform f32 in [lo, hi] via a 2^24 draw. Shrinks toward 0.0
    /// when 0 is in range (canonical midpoint), else toward lo.
    pub fn unit_f32(&mut self, frame: u64, lo: f32, hi: f32) -> f32 {
        let draw = self.below(frame, 1 << 24) as f32 / (1u64 << 24) as f32;
        lo + (hi - lo) * draw
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_choices_are_simplest() {
        // All-zero replay: below() returns 0 for any bound; unit_f32
        // draws map to lo.
        let mut cs = ChoiceStream::from_seed(9).with_replay(
            vec![
                Choice {
                    frame: 1,
                    value: 0,
                    bound: 3
                };
                5
            ],
            Continuation::Zeros,
        );
        assert_eq!(cs.below(1, 3), 0);
        assert_eq!(cs.below(2, 100), 0);
        let v = cs.unit_f32(3, -1.0, 1.0);
        assert_eq!(v, -1.0, "zero draw maps to lo");
        // Recorded values are the reduced values.
        assert!(cs.recorded.iter().all(|c| c.value == 0));
    }

    #[test]
    fn out_of_range_choice_clamps() {
        // Replayed 999 with bound 3 gives a valid variant, no panic.
        let mut cs = ChoiceStream::from_seed(1).with_replay(
            vec![Choice {
                frame: 1,
                value: 999,
                bound: 3,
            }],
            Continuation::Zeros,
        );
        let v = cs.below(1, 3);
        assert!(v < 3, "clamped to [0,3): got {v}");
    }

    #[test]
    fn replay_then_continuation_zeros() {
        let mut cs = ChoiceStream::from_seed(7).with_replay(
            vec![Choice {
                frame: 1,
                value: 2,
                bound: 5,
            }],
            Continuation::Zeros,
        );
        assert_eq!(cs.below(1, 5), 2, "replay consumed first");
        assert_eq!(cs.below(2, 5), 0, "zeros after replay exhausted");
    }

    #[test]
    fn below_records_frame_value_bound() {
        let mut cs = ChoiceStream::from_seed(3);
        let v = cs.below(17, 4);
        let last = *cs.recorded.last().unwrap();
        assert_eq!(last.frame, 17);
        assert_eq!(last.value, v);
        assert_eq!(last.bound, 4);
        assert!(v < 4);
    }

    #[test]
    fn seed_reproducibility() {
        // Same seed, no replay → identical sequences.
        let mut a = ChoiceStream::from_seed(123);
        let mut b = ChoiceStream::from_seed(123);
        for f in 0..50 {
            assert_eq!(a.below(f, 1000), b.below(f, 1000));
        }
    }

    #[test]
    fn bias_free_small_bounds() {
        // Chi-square-ish sanity over 60k draws for bound 3 (loose
        // threshold; deterministic seed). Modulo bias on low bits
        // would skew this badly.
        let mut cs = ChoiceStream::from_seed(9);
        let mut counts = [0usize; 3];
        let n = 60_000;
        for f in 0..n as u64 {
            counts[cs.below(f, 3) as usize] += 1;
        }
        let expected = n / 3;
        for c in counts {
            let dev = (c as i64 - expected as i64).abs();
            assert!(
                dev < 700, // ~3.4 sigma of sqrt(n/3 * 2/3); loose
                "counts skewed: {counts:?}"
            );
        }
    }

    #[test]
    fn chance_is_bernoulli() {
        let mut cs = ChoiceStream::from_seed(5);
        assert!(cs.chance(1, 1, 1), "num==den always true");
        assert!(!cs.chance(2, 0, 1), "num==0 always false");
    }

    #[test]
    fn continuation_reseed_diverges() {
        let mut a = ChoiceStream::from_seed(11).with_replay(vec![], Continuation::Reseed(777));
        let mut b = ChoiceStream::from_seed(22).with_replay(vec![], Continuation::Reseed(777));
        for f in 0..20 {
            assert_eq!(
                a.below(f, 100),
                b.below(f, 100),
                "same reseed → same stream"
            );
        }
        let mut c = ChoiceStream::from_seed(11);
        assert_ne!(
            a.below(0, 1_000_000),
            c.below(0, 1_000_000),
            "reseed diverges from plain seed"
        );
    }

    #[test]
    fn replay_of_recorded_reproduces() {
        // Record a run, replay its recorded choices on a different
        // seed → same reduced values.
        let mut first = ChoiceStream::from_seed(42);
        let draws: Vec<u64> = (0..30).map(|f| first.below(f, 7)).collect();
        let recorded = first.recorded.clone();
        let mut second = ChoiceStream::from_seed(1).with_replay(recorded, Continuation::Zeros);
        for (i, expected) in draws.iter().enumerate() {
            assert_eq!(&second.below(i as u64, 7), expected);
        }
    }
}
