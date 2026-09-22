pub fn print_amplitude(samples: &Vec<f32>) {

    // RMS (root mean square) gives you overall loudness
    let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();

    // Peak amplitude (loudest single sample)
    let peak = samples.iter().fold(0.0_f32, |max, &s| max.max(s.abs()));

    println!("RMS: {}, Peak: {}", rms, peak);
}
