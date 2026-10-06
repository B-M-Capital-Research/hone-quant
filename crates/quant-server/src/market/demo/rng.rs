//! Counter-based randomness. Every draw is a pure function of a hashed key — `(seed, stream,
//! symbol, date, …)` — so any value can be recomputed on its own and nothing depends on the order
//! in which symbols, dates or requests are processed.

use std::f64::consts::TAU;

const GOLDEN_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;
const INV_2_32: f64 = 1.0 / 4_294_967_296.0;
const INV_2_53: f64 = 1.0 / 9_007_199_254_740_992.0;

/// The SplitMix64 output function: a bijective 64-bit mixer with full avalanche.
#[inline]
pub fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Combines words into one well-mixed key; order matters.
pub fn key(parts: &[u64]) -> u64 {
    parts.iter().fold(GOLDEN_GAMMA, |acc, &part| {
        mix(acc.wrapping_add(GOLDEN_GAMMA) ^ mix(part))
    })
}

/// FNV-1a of a string, finalised with [`mix`].
pub fn hash_str(s: &str) -> u64 {
    let mut h = 0xCBF2_9CE4_8422_2325_u64;
    for b in s.bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01B3);
    }
    mix(h)
}

/// A SplitMix64 stream seeded from a key, with Box–Muller normals.
pub struct Rng {
    state: u64,
    spare: Option<f64>,
}

impl Rng {
    pub fn new(key: u64) -> Self {
        Self {
            state: key,
            spare: None,
        }
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(GOLDEN_GAMMA);
        mix(self.state)
    }

    /// Uniform in `[0, 1)` with 53 bits of precision.
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * INV_2_53
    }

    /// Two independent uniforms in `[0, 1)` (32 bits each) from a single draw.
    pub fn unit_pair(&mut self) -> (f64, f64) {
        let bits = self.next_u64();
        (
            (bits >> 32) as f64 * INV_2_32,
            (bits & 0xFFFF_FFFF) as f64 * INV_2_32,
        )
    }

    /// Standard normal via Box–Muller. Each transform yields two values; the second one is kept
    /// for the next call.
    pub fn normal(&mut self) -> f64 {
        if let Some(z) = self.spare.take() {
            return z;
        }
        let (a, b) = self.unit_pair();
        // 1 − a ∈ (0, 1] keeps the logarithm finite.
        let radius = (-2.0 * (1.0 - a).ln()).sqrt();
        let (sin, cos) = (TAU * b).sin_cos();
        self.spare = Some(radius * sin);
        radius * cos
    }

    /// A cheap approximate standard normal (Irwin–Hall): the centred, scaled sum of four 16-bit
    /// uniforms, bounded at ±3.46. Meant for many small steps whose sum matters more than their
    /// tails, such as the 5-minute increments of a pinned intraday path.
    #[inline]
    pub fn quick_normal(&mut self) -> f64 {
        let bits = self.next_u64();
        let sum = (bits & 0xFFFF) + (bits >> 16 & 0xFFFF) + (bits >> 32 & 0xFFFF) + (bits >> 48);
        // Each 16-bit uniform has mean 32767.5 and variance ≈ 65536² / 12.
        (sum as f64 - 131_070.0) * (SQRT_3 / 65_536.0)
    }
}

const SQRT_3: f64 = 1.732_050_807_568_877_2;

/// `exp` of a uniform draw between `ln(lo)` and `ln(hi)`.
pub fn log_uniform(lo: f64, hi: f64, u: f64) -> f64 {
    (lo.ln() + (hi.ln() - lo.ln()) * u).exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_order_sensitive_and_stable() {
        assert_eq!(key(&[1, 2, 3]), key(&[1, 2, 3]));
        assert_ne!(key(&[1, 2, 3]), key(&[3, 2, 1]));
        assert_ne!(key(&[1, 2]), key(&[1, 2, 0]));
        assert_ne!(hash_str("NVDA"), hash_str("NVDB"));
    }

    #[test]
    fn draws_have_the_expected_moments() {
        let mut rng = Rng::new(key(&[42]));
        let n = 200_000;
        let moments = |draw: &mut dyn FnMut() -> f64| {
            let (mut sum, mut sum_sq) = (0.0, 0.0);
            for _ in 0..n {
                let z = draw();
                sum += z;
                sum_sq += z * z;
            }
            let mean = sum / n as f64;
            (mean, sum_sq / n as f64 - mean * mean)
        };
        let box_muller = moments(&mut || rng.normal());
        let irwin_hall = moments(&mut || rng.quick_normal());
        for (mean, var) in [box_muller, irwin_hall] {
            assert!(mean.abs() < 0.01, "mean {mean}");
            assert!((var - 1.0).abs() < 0.02, "variance {var}");
        }
        let (mean, _) = moments(&mut || {
            let u = rng.unit();
            assert!((0.0..1.0).contains(&u));
            u
        });
        assert!((mean - 0.5).abs() < 0.005);
    }
}
