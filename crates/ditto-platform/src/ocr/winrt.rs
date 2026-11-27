//! Windows: `Windows.Media.Ocr`.

use windows::Globalization::Language;
use windows::Graphics::Imaging::{
    BitmapAlphaMode, BitmapDecoder, BitmapInterpolationMode, BitmapPixelFormat, BitmapTransform, ColorManagementMode,
    ExifOrientationMode,
};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};

use super::{Status, second_pass_scale};
use crate::{PlatformError, Result};

const NO_LANGUAGE: &str = "No text recognition language is installed. Add one in Settings → Time & language → \
                           Language & region (the language's optional features include text recognition).";

impl From<windows::core::Error> for PlatformError {
    fn from(e: windows::core::Error) -> Self {
        PlatformError::Ocr(e.message())
    }
}

/// An engine for the user's languages, or failing that for any installed OCR language.
/// (`TryCreate…` returns null, which the bindings report as an error, when there is none.)
fn engine() -> Option<OcrEngine> {
    OcrEngine::TryCreateFromUserProfileLanguages().ok().or_else(|| {
        let languages = OcrEngine::AvailableRecognizerLanguages().ok()?;
        languages.into_iter().find_map(|language| OcrEngine::TryCreateFromLanguage(&language).ok())
    })
}

pub fn status() -> Status {
    match engine() {
        Some(engine) => {
            let language = engine.RecognizerLanguage().and_then(|l| l.DisplayName()).map(|n| n.to_string()).unwrap_or_default();
            Status { engine: "Windows OCR", available: true, detail: format!("Built into Windows; reads {language}.") }
        }
        None => Status { engine: "Windows OCR", available: false, detail: NO_LANGUAGE.into() },
    }
}

#[derive(Default)]
struct Pass {
    lines: Vec<String>,
    word_heights: Vec<f64>,
}

impl Pass {
    fn median_word_height(&self) -> Option<f64> {
        let mut heights = self.word_heights.clone();
        heights.sort_by(f64::total_cmp);
        heights.get(heights.len() / 2).copied()
    }
}

pub fn recognize(png: &[u8]) -> Result<String> {
    let engine = engine().ok_or(PlatformError::Unsupported("no text recognition language is installed"))?;
    // Chinese, Japanese and Korean have no spaces between words, but the engine reports every
    // character as a word and joins them with spaces.
    let joiner = match engine.RecognizerLanguage().and_then(|l: Language| l.LanguageTag()) {
        Ok(tag) if ["ja", "zh", "ko"].iter().any(|cjk| tag.to_string().to_lowercase().starts_with(cjk)) => "",
        _ => " ",
    };

    let stream = InMemoryRandomAccessStream::new()?;
    let writer = DataWriter::CreateDataWriter(&stream)?;
    writer.WriteBytes(png)?;
    writer.StoreAsync()?.join()?;
    writer.FlushAsync()?.join()?;
    writer.DetachStream()?;
    stream.Seek(0)?;
    let decoder = BitmapDecoder::CreateAsync(&stream)?.join()?;
    let (width, height) = (decoder.PixelWidth()?, decoder.PixelHeight()?);
    if width == 0 || height == 0 {
        return Ok(String::new());
    }

    let run = |scale: f64| -> Result<Pass> {
        let transform = BitmapTransform::new()?;
        if (scale - 1.0).abs() > 0.001 {
            transform.SetScaledWidth(((f64::from(width) * scale).round() as u32).max(1))?;
            transform.SetScaledHeight(((f64::from(height) * scale).round() as u32).max(1))?;
            transform.SetInterpolationMode(if scale > 1.0 { BitmapInterpolationMode::Cubic } else { BitmapInterpolationMode::Fant })?;
        }
        // The engine accepts few pixel formats; BGRA8 is always one of them.
        let bitmap = decoder
            .GetSoftwareBitmapTransformedAsync(
                BitmapPixelFormat::Bgra8,
                BitmapAlphaMode::Premultiplied,
                &transform,
                ExifOrientationMode::IgnoreExifOrientation,
                ColorManagementMode::DoNotColorManage,
            )?
            .join()?;
        let result = engine.RecognizeAsync(&bitmap)?.join()?;
        let mut pass = Pass::default();
        for line in result.Lines()? {
            let mut words = Vec::new();
            for word in line.Words()? {
                words.push(word.Text()?.to_string());
                pass.word_heights.push(f64::from(word.BoundingRect()?.Height));
            }
            pass.lines.push(words.join(joiner));
        }
        Ok(pass)
    };

    // Shrink only when the image is larger than the engine accepts.
    let max_dimension = OcrEngine::MaxImageDimension()?.max(1);
    let fit = (f64::from(max_dimension) / f64::from(width.max(height))).min(1.0);
    let mut pass = run(fit)?;
    if let Some(scale) = second_pass_scale(pass.median_word_height(), width, height, fit, max_dimension) {
        match run(scale) {
            Ok(enlarged) if !enlarged.lines.is_empty() => pass = enlarged,
            Ok(_) => {}
            Err(e) => log::debug!("second OCR pass at {scale:.2}× failed: {e}"),
        }
    }
    Ok(pass.lines.join("\n"))
}
