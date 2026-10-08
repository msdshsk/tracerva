use tracerva::{Options, RefineOptions, refine};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let input = args.next().ok_or("usage: trace <input> <output.svg>")?;
    let output = args.next().ok_or("usage: trace <input> <output.svg>")?;
    if args.next().is_some() {
        return Err("usage: trace <input> <output.svg>".into());
    }
    let image = image::open(input)?.to_rgba8();
    let result = refine(
        &image,
        &Options::default(),
        &RefineOptions {
            geometry: true,
            smooth: 2.0,
            ..Default::default()
        },
    )?;
    std::fs::write(output, result.svg)?;
    Ok(())
}
