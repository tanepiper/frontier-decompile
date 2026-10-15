//! Band-limited sample-rate conversion from the chip's 49,716 Hz to the host's output rate.
//!
//! A polyphase windowed-sinc filter (Kaiser window, 2 × 32 taps, 512 phases with linear
//! interpolation between them). The cut-off sits just below the lower Nyquist frequency, so
//! converting to 44.1 or 48 kHz does not fold the chip's upper harmonics back as aliases. The
//! ratio can be nudged a fraction of a percent at run time so a host can keep its output buffer
//! level steady without dropping or repeating samples.

const HALF: usize = 32;
const PHASES: usize = 512;

pub struct Resampler {
    /// Input samples per output sample (before the adjustment).
    ratio: f64,
    adjust: f64,
    /// Read position in `buf`, in input samples.
    pos: f64,
    buf: Vec<f32>,
    /// `PHASES + 1` rows of `2 × HALF` taps.
    table: Vec<f32>,
}

fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term) = (1.0, 1.0);
    for k in 1..50 {
        term *= (x / (2.0 * k as f64)).powi(2);
        sum += term;
        if term < 1e-12 * sum {
            break;
        }
    }
    sum
}

impl Resampler {
    pub fn new(in_rate: f64, out_rate: f64) -> Self {
        let ratio = in_rate / out_rate;
        // Cut-off as a fraction of the input rate: 0.5 is the input's Nyquist frequency.
        let cutoff = 0.5 * (1.0 / ratio).min(1.0) * 0.91;
        let beta = 8.0;
        let norm = bessel_i0(beta);
        let mut table = vec![0f32; (PHASES + 1) * 2 * HALF];
        for p in 0..=PHASES {
            let frac = p as f64 / PHASES as f64;
            let row = &mut table[p * 2 * HALF..(p + 1) * 2 * HALF];
            let mut sum = 0.0;
            let mut taps = [0f64; 2 * HALF];
            for (i, t) in taps.iter_mut().enumerate() {
                // Tap i weights input sample (floor(pos) − HALF + 1 + i).
                let x = i as f64 - (HALF as f64 - 1.0) - frac;
                let sinc = if x == 0.0 {
                    2.0 * cutoff
                } else {
                    (2.0 * std::f64::consts::PI * cutoff * x).sin() / (std::f64::consts::PI * x)
                };
                let w = x / HALF as f64;
                let win = if w.abs() >= 1.0 { 0.0 } else { bessel_i0(beta * (1.0 - w * w).sqrt()) / norm };
                *t = sinc * win;
                sum += *t;
            }
            for (r, t) in row.iter_mut().zip(taps) {
                *r = (t / sum) as f32;
            }
        }
        Resampler { ratio, adjust: 1.0, pos: (HALF - 1) as f64, buf: vec![0.0; HALF - 1], table }
    }

    /// Scale the conversion ratio by `factor` (clamped to ±1 %): above 1 consumes input faster.
    pub fn set_adjust(&mut self, factor: f64) {
        self.adjust = factor.clamp(0.99, 1.01);
    }

    /// Queue input samples.
    pub fn push(&mut self, input: &[i16]) {
        self.buf.extend(input.iter().map(|&s| s as f32 / 32768.0));
    }

    /// Input samples queued but not yet consumed.
    pub fn pending(&self) -> f64 {
        self.buf.len() as f64 - self.pos
    }

    /// Append every output sample the queued input allows.
    pub fn pull(&mut self, out: &mut Vec<f32>) {
        let step = self.ratio * self.adjust;
        while self.pos + (HALF as f64) < self.buf.len() as f64 {
            let base = self.pos.floor();
            let ph = (self.pos - base) * PHASES as f64;
            let pi = ph.floor() as usize;
            let mix = (ph - pi as f64) as f32;
            let start = base as usize + 1 - HALF;
            let input = &self.buf[start..start + 2 * HALF];
            let a = &self.table[pi * 2 * HALF..(pi + 1) * 2 * HALF];
            let b = &self.table[(pi + 1) * 2 * HALF..(pi + 2) * 2 * HALF];
            let mut acc = 0f32;
            for i in 0..2 * HALF {
                acc += input[i] * (a[i] + (b[i] - a[i]) * mix);
            }
            out.push(acc);
            self.pos += step;
        }
        // Drop input that no future output needs.
        let keep_from = (self.pos.floor() as usize + 1).saturating_sub(HALF);
        if keep_from > 4096 {
            self.buf.drain(..keep_from);
            self.pos -= keep_from as f64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_and_gain() {
        let mut r = Resampler::new(49_716.0, 48_000.0);
        // 1 kHz sine at half scale, one second.
        let input: Vec<i16> =
            (0..49_716).map(|i| ((i as f64 * 1000.0 * std::f64::consts::TAU / 49_716.0).sin() * 16384.0) as i16).collect();
        r.push(&input);
        let mut out = Vec::new();
        r.pull(&mut out);
        assert!((out.len() as i64 - 48_000).abs() < 40, "{}", out.len());
        let peak = out[1000..].iter().fold(0f32, |m, &s| m.max(s.abs()));
        assert!((peak - 0.5).abs() < 0.01, "{peak}");
    }

    #[test]
    fn rejects_above_nyquist() {
        let mut r = Resampler::new(49_716.0, 44_100.0);
        // 23 kHz is above the 22.05 kHz output Nyquist frequency: it must not alias back.
        let input: Vec<i16> =
            (0..20_000).map(|i| ((i as f64 * 23_000.0 * std::f64::consts::TAU / 49_716.0).sin() * 16384.0) as i16).collect();
        r.push(&input);
        let mut out = Vec::new();
        r.pull(&mut out);
        let peak = out[500..].iter().fold(0f32, |m, &s| m.max(s.abs()));
        assert!(peak < 0.01, "{peak}");
    }
}
