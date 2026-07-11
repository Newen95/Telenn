//! Hand-rolled radix-2 FFT — a few dozen lines suffice for offline analysis,
//! keeping the crate free of DSP dependencies.

use super::AnalysisError;

/// In-place iterative Cooley–Tukey FFT. `re` and `im` must have the same
/// power-of-two length.
pub fn fft(re: &mut [f32], im: &mut [f32]) -> Result<(), AnalysisError> {
    let n = re.len();
    if n != im.len() {
        return Err(AnalysisError::BadInput(
            "fft buffers differ in length".into(),
        ));
    }
    if n == 0 || !n.is_power_of_two() {
        return Err(AnalysisError::BadInput(format!(
            "fft length {n} is not a power of two"
        )));
    }

    // Bit-reversal permutation.
    let bits = n.trailing_zeros();
    for i in 0..n {
        let j = i.reverse_bits() >> (usize::BITS - bits);
        if j > i {
            re.swap(i, j);
            im.swap(i, j);
        }
    }

    // Butterflies, stage by stage.
    let mut len = 2;
    while len <= n {
        let ang = -std::f64::consts::TAU / len as f64;
        let (w_re, w_im) = (ang.cos() as f32, ang.sin() as f32);
        for start in (0..n).step_by(len) {
            let (mut cur_re, mut cur_im) = (1.0f32, 0.0f32);
            for k in start..start + len / 2 {
                let (a_re, a_im) = (re[k], im[k]);
                let (b_re, b_im) = (re[k + len / 2], im[k + len / 2]);
                let t_re = b_re * cur_re - b_im * cur_im;
                let t_im = b_re * cur_im + b_im * cur_re;
                re[k] = a_re + t_re;
                im[k] = a_im + t_im;
                re[k + len / 2] = a_re - t_re;
                im[k + len / 2] = a_im - t_im;
                let next_re = cur_re * w_re - cur_im * w_im;
                cur_im = cur_re * w_im + cur_im * w_re;
                cur_re = next_re;
            }
        }
        len *= 2;
    }
    Ok(())
}

/// Hann window of length `n`.
pub fn hann(n: usize) -> Vec<f32> {
    if n <= 1 {
        return vec![1.0; n];
    }
    (0..n)
        .map(|i| {
            let phase = std::f32::consts::TAU * i as f32 / (n - 1) as f32;
            0.5 * (1.0 - phase.cos())
        })
        .collect()
}

/// Hann-windowed power spectrum of `x`, zero-padded to the next power of two.
/// Returns `fft_len/2 + 1` power bins; use [`bin_hz`] with the same
/// `fft_len = next_power_of_two(x.len())` to map bins to frequencies.
pub fn power_spectrum(x: &[f32]) -> Result<Vec<f32>, AnalysisError> {
    if x.is_empty() {
        return Err(AnalysisError::BadInput("empty signal".into()));
    }
    let fft_len = x.len().next_power_of_two();
    let window = hann(x.len());
    let mut re = vec![0.0f32; fft_len];
    let mut im = vec![0.0f32; fft_len];
    for (i, (&s, &w)) in x.iter().zip(&window).enumerate() {
        re[i] = s * w;
    }
    fft(&mut re, &mut im)?;
    Ok((0..fft_len / 2 + 1)
        .map(|i| re[i] * re[i] + im[i] * im[i])
        .collect())
}

/// Centre frequency of `bin` for a spectrum of `fft_len` points.
pub fn bin_hz(bin: usize, fft_len: usize, sample_rate: u32) -> f32 {
    bin as f32 * sample_rate as f32 / fft_len as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, sr: u32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (std::f32::consts::TAU * freq * i as f32 / sr as f32).sin())
            .collect()
    }

    #[test]
    fn sine_peaks_at_its_bin() {
        let sr = 48_000;
        let n = 4096;
        // Bin-exact frequency so leakage stays minimal.
        let freq = bin_hz(128, n, sr);
        let power = power_spectrum(&sine(freq, sr, n)).unwrap();
        let peak_bin = power
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(i, _)| i)
            .unwrap();
        assert_eq!(peak_bin, 128);
    }

    #[test]
    fn impulse_spectrum_is_flat() {
        let mut re = vec![0.0f32; 256];
        let mut im = vec![0.0f32; 256];
        re[0] = 1.0;
        fft(&mut re, &mut im).unwrap();
        for i in 0..256 {
            let mag = (re[i] * re[i] + im[i] * im[i]).sqrt();
            assert!((mag - 1.0).abs() < 1e-4, "bin {i}: {mag}");
        }
    }

    #[test]
    fn parseval_energy_matches() {
        // Sum of |x|^2 == sum of |X|^2 / N.
        let x = sine(997.0, 48_000, 1024);
        let mut re = x.clone();
        let mut im = vec![0.0f32; 1024];
        fft(&mut re, &mut im).unwrap();
        let time_energy: f32 = x.iter().map(|s| s * s).sum();
        let freq_energy: f32 = re.iter().zip(&im).map(|(r, i)| r * r + i * i).sum::<f32>() / 1024.0;
        assert!(
            (time_energy - freq_energy).abs() / time_energy < 1e-3,
            "{time_energy} vs {freq_energy}"
        );
    }

    #[test]
    fn rejects_non_power_of_two() {
        let mut re = vec![0.0f32; 100];
        let mut im = vec![0.0f32; 100];
        assert!(fft(&mut re, &mut im).is_err());
    }
}
