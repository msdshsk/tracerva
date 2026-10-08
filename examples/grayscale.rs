use tracerva::{GrayscaleOptions, Options, RefineOptions, refine_grayscale};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !(2..=5).contains(&args.len()) {
        return Err(
            "usage: grayscale <input> <output.svg> [levels=8] [denoise=2] [min_area=16]".into(),
        );
    }
    let image = image::open(&args[0])?.to_rgba8();
    let result = refine_grayscale(
        &image,
        &Options::default(),
        &RefineOptions {
            geometry: true,
            smooth: 2.0,
            background: Some([255; 3]),
            ..Default::default()
        },
        &GrayscaleOptions {
            levels: args.get(2).map(|s| s.parse()).transpose()?.unwrap_or(8),
            denoise: args.get(3).map(|s| s.parse()).transpose()?.unwrap_or(2),
            min_region_area: args.get(4).map(|s| s.parse()).transpose()?.unwrap_or(16),
        },
    )?;
    eprintln!("{:?}", result.stats);
    std::fs::write(&args[1], result.svg)?;
    Ok(())
}
