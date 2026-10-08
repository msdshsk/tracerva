use serde::Deserialize;
use tracerva::{GrayscaleOptions, Options, Paint, RefineOptions, refine, refine_grayscale};
use wasm_bindgen::prelude::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    colors: usize,
    smooth: f64,
    geometry: bool,
    merge_distance: f64,
    background: Option<[u8; 3]>,
    outline: bool,
    #[serde(default)]
    palette: Vec<[u8; 3]>,
    #[serde(default)]
    grayscale: Option<GraySettings>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GraySettings {
    levels: usize,
    denoise: u32,
    min_region_area: usize,
}

fn convert(rgba: &[u8], width: u32, height: u32, settings: &str) -> Result<String, String> {
    let pixels = u64::from(width) * u64::from(height);
    if pixels == 0 || pixels > 16_000_000 || pixels * 4 != rgba.len() as u64 {
        return Err("Invalid RGBA dimensions (maximum 16 million pixels)".into());
    }
    let settings: Settings = serde_json::from_str(settings).map_err(|e| e.to_string())?;
    let image =
        image::RgbaImage::from_raw(width, height, rgba.to_vec()).ok_or("Invalid RGBA buffer")?;
    let options = Options {
        colors: settings.colors,
        ..Default::default()
    };
    let refine_options = RefineOptions {
        smooth: settings.smooth,
        geometry: settings.geometry,
        merge_distance: settings.merge_distance,
        palette: settings.palette,
        background: settings.background,
        paint: if settings.outline {
            Paint::Outline
        } else {
            Paint::Flat
        },
        ..Default::default()
    };
    let result = if let Some(gray) = settings.grayscale {
        refine_grayscale(
            &image,
            &options,
            &refine_options,
            &GrayscaleOptions {
                levels: gray.levels,
                denoise: gray.denoise,
                min_region_area: gray.min_region_area,
            },
        )
    } else {
        refine(&image, &options, &refine_options)
    }
    .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "svg": result.svg,
        "paths": result.stats.paths,
        "circles": result.stats.geometry_circles,
        "polygons": result.stats.geometry_polygons,
        "colors": result.stats.palette_after
    })
    .to_string())
}

/// Trace browser-decoded RGBA pixels. Returns SVG and statistics as JSON.
#[wasm_bindgen]
pub fn trace_rgba(rgba: &[u8], width: u32, height: u32, settings: &str) -> Result<String, JsValue> {
    convert(rgba, width, height, settings).map_err(|e| JsValue::from_str(&e))
}

#[cfg(test)]
mod tests {
    use super::*;
    const SETTINGS: &str = r#"{"colors":2,"smooth":2,"geometry":true,"merge_distance":0,"background":[255,255,255],"outline":false}"#;

    #[test]
    fn validates_buffer_before_allocating() {
        assert!(convert(&[], u32::MAX, u32::MAX, SETTINGS).is_err());
        assert!(convert(&[0; 3], 1, 1, SETTINGS).is_err());
        assert!(convert(&[], 0, 0, SETTINGS).is_err());
    }

    #[test]
    fn grayscale_bridge_returns_original_color_and_validates_parameters() {
        let mut settings: serde_json::Value = serde_json::from_str(SETTINGS).unwrap();
        settings["grayscale"] = serde_json::json!({"levels":8,"denoise":0,"min_region_area":0});
        let output = convert(&[183, 0, 0, 255], 1, 1, &settings.to_string()).unwrap();
        let output: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert!(output["svg"].as_str().unwrap().contains("#b70000"));
        settings["grayscale"]["levels"] = 65.into();
        assert!(convert(&[183, 0, 0, 255], 1, 1, &settings.to_string()).is_err());
    }

    #[test]
    fn traces_and_propagates_invalid_settings() {
        let output = convert(&[0, 0, 0, 255], 1, 1, SETTINGS).unwrap();
        let output: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(output["paths"], 1);
        assert!(output["svg"].as_str().unwrap().contains("<path"));
        assert!(convert(&[0; 4], 1, 1, "{}").is_err());
        assert!(
            convert(
                &[0; 4],
                1,
                1,
                &SETTINGS.replace("\"colors\":2", "\"colors\":0")
            )
            .is_err()
        );
    }
}
